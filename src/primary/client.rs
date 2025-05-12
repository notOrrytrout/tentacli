use cfg_if::cfg_if;
use std::io::{Error, ErrorKind};
use std::sync::{Arc, Mutex as SyncMutex};
use std::time::Duration;
use tokio::sync::{Mutex, Notify};
use async_broadcast::{broadcast, Sender as BroadcastSender, Receiver as BroadcastReceiver};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use futures::future::join_all;
use tokio::time::sleep;
use anyhow::Result as AnyResult;
use tentacli_crypto::{Decryptor, Encryptor, WardenCrypt};
use tentacli_traits::Feature;
use tentacli_traits::types::opcodes::Opcode;
use tentacli_traits::types::shared::{DataStorage, Session};
use tentacli_traits::types::{
    HandlerInput, HandlerOutput, IncomingPacket,
    OutgoingPacket, ProcessorResult, Signal,
};
use tentacli_utils::encode_hex;

use crate::primary::network::{Reader, Writer};

#[derive(Default)]
pub struct CreateOptions {
    pub data_storage: Option<Arc<SyncMutex<DataStorage>>>,
}
pub struct RunOptions<'a> {
    pub external_features: Vec<Box<dyn Feature>>,
    pub config_path: &'a str,
    pub account: &'a str,
    pub password: &'a str,
    pub host: &'a str,
    pub port: u16,
    pub realm: &'a str,
    pub character: &'a str,
    pub log_file: Option<String>,
    pub login_delay: Option<u64>,
    pub dotenv_path: &'a str,
}


pub struct Client {
    _reader: Arc<Mutex<Option<Reader>>>,
    _writer: Arc<Mutex<Option<Writer>>>,
    _warden_crypt: Arc<SyncMutex<Option<WardenCrypt>>>,

    session: Arc<Mutex<Session>>,
    data_storage: Arc<SyncMutex<DataStorage>>,
}

impl Client {
    pub fn new(options: CreateOptions) -> Self {
        Self {
            _reader: Arc::new(Mutex::new(None)),
            _writer: Arc::new(Mutex::new(None)),
            _warden_crypt: Arc::new(SyncMutex::new(None)),
            session: Arc::new(Mutex::new(Session::new())),
            data_storage: options.data_storage.unwrap_or_else(|| Arc::new(SyncMutex::new(DataStorage::default()))),
        }
    }

    async fn connect_inner(host: &str, port: u16) -> Result<TcpStream, Error> {
        let addr = format!("{}:{}", host, port);
        TcpStream::connect(&addr).await
    }

    async fn set_stream_halves(
        stream: TcpStream,
        reader: Arc<Mutex<Option<Reader>>>,
        writer: Arc<Mutex<Option<Writer>>>,
        session_key: Option<Vec<u8>>,
        warden_crypt: Arc<SyncMutex<Option<WardenCrypt>>>,
    ) {
        let (rx, tx) = stream.into_split();

        if let Some(session_key) = session_key {
            *warden_crypt.lock().unwrap() = Some(WardenCrypt::new(&session_key));

            *reader.lock().await = Some(Reader::new(rx, Arc::clone(&warden_crypt), true, Some(Decryptor::new(&session_key))));
            *writer.lock().await = Some(Writer::new(tx, Arc::clone(&warden_crypt), true, Some(Encryptor::new(&session_key))));
        } else {
            *reader.lock().await = Some(Reader::new(rx, Arc::new(SyncMutex::new(None)), false, None));
            *writer.lock().await = Some(Writer::new(tx, Arc::new(SyncMutex::new(None)), false, None));
        }
    }

    pub async fn run(&mut self, options: RunOptions<'_>) -> AnyResult<()> {
        let host = options.host;
        let port = options.port;


        const BUFFER_SIZE: usize = 50;

        let notify = Arc::new(Notify::new());

        let (signal_sender, signal_receiver) = mpsc::channel::<Signal>(1);
        let (output_sender, output_receiver) = mpsc::channel::<OutgoingPacket>(BUFFER_SIZE);
        let (query_sender, query_receiver) = broadcast::<HandlerOutput>(BUFFER_SIZE);

        match Self::connect_inner(&host, port).await {
            Ok(stream) => {
                Self::set_stream_halves(stream, Arc::clone(&self._reader), Arc::clone(&self._writer), None, Arc::clone(&self._warden_crypt)).await;

                if let Err(err) = self.session.lock().await.set_config(&host, options.account, options.config_path) {
                    query_sender.broadcast(HandlerOutput::ErrorMessage(err.to_string(), None)).await.ok();
                }

                query_sender.broadcast(HandlerOutput::SuccessMessage(format!("Connected to {}:{}", host, port), None)).await.ok();
            },
            Err(err) => {
                query_sender.broadcast(HandlerOutput::ErrorMessage(format!("Cannot connect: {}", err), None)).await.ok();
                return Err(err.into());
            },
        }

        let mut features: Vec<Box<dyn Feature>> = options.external_features;

        cfg_if! {
            if #[cfg(feature = "ui")] {
                use crate::features::ui::UI;
                features.push(Box::new(UI::default()));
            } else if #[cfg(feature = "console")] {
                use crate::features::console::Console;
                features.push(Box::new(Console::default()));
            }
        }

        cfg_if! {
            if #[cfg(feature = "wotlk_login")] {
                use crate::features::wotlk_login::WotlkLogin;
                features.push(Box::new(WotlkLogin::default()));
            }
        }

        cfg_if! {
            if #[cfg(feature = "wotlk_realm")] {
                use crate::features::wotlk_realm::WotlkRealm;
                features.push(Box::new(WotlkRealm));
            }
        }

        for feature in &mut features {
            feature.set_broadcast_channel(query_sender.clone(), query_receiver.clone());
        }

        let all_tasks: Vec<JoinHandle<AnyResult<()>>> = vec![
            self.handle_read(signal_receiver, query_sender.clone(), notify.clone(), features),
            self.handle_output(signal_sender.clone(), output_sender.clone(), query_sender.clone(), query_receiver, notify.clone()),
            self.handle_write(output_receiver, query_sender),
        ];

        for task in join_all(all_tasks).await {
            if let Err(e) = task {
                eprintln!("Task panicked: {:?}", e);
            } else if let Ok(Err(err)) = task {
                eprintln!("Task failed: {:?}", err);
            }
        }

                return Ok(());
    }

    fn handle_read(
        &mut self,
        mut signal_receiver: Receiver<Signal>,
        query_sender: BroadcastSender<HandlerOutput>,
        notify: Arc<Notify>,
        features: Vec<Box<dyn Feature>>,
    ) -> JoinHandle<AnyResult<()>> {
        let reader = Arc::clone(&self._reader);
        let session = Arc::clone(&self.session);
        let data_storage = Arc::clone(&self.data_storage);

        tokio::spawn(async move {
            let mut realm_processors = vec![];
            let mut processors = vec![];
            let mut one_time_handler_maps = vec![];
            let mut initial_processors = vec![];

            for feature in features {
                realm_processors.extend(feature.get_realm_processors());
                processors.extend(feature.get_login_processors());
                one_time_handler_maps.extend(feature.get_one_time_handler_maps());
                initial_processors.extend(feature.get_initial_processors());
                return Ok(());
            }

            let mut realm_processors = Some(realm_processors);

            let handler_list = initial_processors.iter().flat_map(|p| p(Opcode::LOGIN_CHALLENGE as u16)).collect::<ProcessorResult>();

            Self::call_handlers(handler_list, &query_sender, &notify, HandlerInput {
                session: Arc::clone(&session),
                data: vec![],
                data_storage: Arc::clone(&data_storage),
                opcode: Opcode::LOGIN_CHALLENGE as u16,
            }).await;

            loop {
                tokio::select! {
                    _ = signal_receiver.recv() => {
                        processors = realm_processors.take().unwrap_or_default();
                    },
                    result = Self::read_packet(&reader) => {
                        match result {
                            Ok(IncomingPacket { opcode, body: data, .. }) => {
                                let input = HandlerInput {
                                    session: Arc::clone(&session),
                                    data,
                                    data_storage: Arc::clone(&data_storage),
                                    opcode,
                                };

                                let mut handler_list = processors.iter()
                                    .flat_map(|p| p(opcode))
                                    .collect::<ProcessorResult>();

                                for map in one_time_handler_maps.iter_mut() {
                                    if let Some(mut h) = map.remove(&opcode) {
                                        handler_list.append(&mut h);
                                    }
                                }

                                if handler_list.is_empty() {
                                    let name = Opcode::get_opcode_name(input.opcode as u32).unwrap_or(format!("Unknown opcode: {}", input.opcode));
                                    query_sender.broadcast(HandlerOutput::ResponseMessage(name, Some(encode_hex(&input.data)))).await.ok();
                                }

                                Self::call_handlers(handler_list, &query_sender, &notify, input).await;
                            },
                            Err(err) => {
                                query_sender.broadcast(HandlerOutput::ErrorMessage(err.to_string(), None)).await.ok();
                                sleep(Duration::from_secs(1)).await;
                            }
                        }
                    }
                }
            }

            #[allow(unreachable_code)]
                return Ok(());
        })
    }

    fn handle_output(
        &mut self,
        signal_sender: Sender<Signal>,
        output_sender: Sender<OutgoingPacket>,
        query_sender: BroadcastSender<HandlerOutput>,
        mut query_receiver: BroadcastReceiver<HandlerOutput>,
        notify: Arc<Notify>,
    ) -> JoinHandle<AnyResult<()>> {
        let session = Arc::clone(&self.session);
        let reader = Arc::clone(&self._reader);
        let writer = Arc::clone(&self._writer);
        let warden_crypt = Arc::clone(&self._warden_crypt);

        tokio::spawn(async move {
            loop {
                match query_receiver.recv().await {
                    Ok(output) => {
                        match output {
                            HandlerOutput::Data((opcode, data, json_details)) => {
                                output_sender.send(OutgoingPacket { opcode, data, json_details }).await?;
                            },
                            HandlerOutput::ConnectionRequest(host, port) => {
                                match Self::connect_inner(&host, port).await {
                                    Ok(stream) => {
                                        signal_sender.send(Signal::Reconnect).await?;

                                        let session_key = {
                                            let guard = session.lock().await;
                                            guard.srp.as_ref().unwrap().session_key.clone()
                                        };

                                        Self::set_stream_halves(stream, Arc::clone(&reader), Arc::clone(&writer), Some(session_key), Arc::clone(&warden_crypt)).await;

                                        query_sender.broadcast(HandlerOutput::SuccessMessage(format!("Connected to {}:{}", host, port), None)).await.ok();
                                    },
                                    Err(err) => {
                                        query_sender.broadcast(HandlerOutput::ErrorMessage(err.to_string(), None)).await.ok();
                                    }
                                }
                            },
                            HandlerOutput::Drop => break,
                            HandlerOutput::SelectRealm(realm) => {
                                session.lock().await.selected_realm = Some(realm);
                                notify.notify_one();
                            },
                            HandlerOutput::SelectCharacter(character) => {
                                session.lock().await.me = Some(character);
                                notify.notify_one();
                            },
                            _ => {},
                        }
                    },
                    Err(err) => {
                        query_sender.broadcast(HandlerOutput::ErrorMessage(err.to_string(), None)).await.ok();
                    },
                }
            }

                return Ok(());
        })
    }

    fn handle_write(
        &mut self,
        mut output_receiver: Receiver<OutgoingPacket>,
        query_sender: BroadcastSender<HandlerOutput>,
    ) -> JoinHandle<AnyResult<()>> {
        let writer = Arc::clone(&self._writer);

        tokio::spawn(async move {
            while let Some(packet) = output_receiver.recv().await {
                if !packet.data.is_empty() {
                    match Self::write_packet(&writer, &packet).await {
                        Ok(bytes_sent) => {
                            let name = Opcode::get_opcode_name(packet.opcode).unwrap_or(packet.opcode.to_string());
                            let msg = format!("{}: {} bytes sent", name, bytes_sent);
                            query_sender.broadcast(HandlerOutput::RequestMessage(msg, Some(packet.json_details))).await.ok();
                        },
                        Err(err) => {
                            query_sender.broadcast(HandlerOutput::ErrorMessage(err.to_string(), None)).await.ok();
                        }
                    }
                }
            }

                return Ok(());
        })
    }

    async fn call_handlers(
        handler_list: ProcessorResult,
        query_sender: &BroadcastSender<HandlerOutput>,
        notify: &Arc<Notify>,
        mut input: HandlerInput,
    ) {
        for mut handler in handler_list {
            match handler.handle(&mut input).await {
                Ok(outputs) => {
                    for output in outputs {
                        match output {
                            HandlerOutput::Freeze => notify.notified().await,
                            _ => {
                                query_sender.broadcast(output).await.ok();
                            },
                        }
                    }
                },
                Err(err) => {
                    query_sender.broadcast(HandlerOutput::ErrorMessage(err.to_string(), None)).await.ok();
                }
            }
        }
    }

    async fn read_packet(reader: &Arc<Mutex<Option<Reader>>>) -> AnyResult<IncomingPacket> {
        let error = Error::new(ErrorKind::NotFound, "Not connected to TCP");
        if let Some(reader) = &mut *reader.lock().await {
            reader.read().await.map_err(Into::into)
        } else {
            Err(anyhow::Error::new(error))
        }
    }

    async fn write_packet(writer: &Arc<Mutex<Option<Writer>>>, packet: &OutgoingPacket) -> AnyResult<usize> {
        let error = Error::new(ErrorKind::NotFound, "Not connected to TCP");
        if let Some(writer) = &mut *writer.lock().await {
            writer.write(packet).await.map_err(Into::into)
        } else {
            Err(anyhow::Error::new(error))
        }
    }
}