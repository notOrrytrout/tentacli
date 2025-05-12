use cfg_if::cfg_if;

#[cfg(feature = "wotlk_login")]
use anyhow::Result as AnyResult;

use std::io::Cursor;
use std::sync::{Arc, Mutex as SyncMutex};

use byteorder::{BigEndian, LittleEndian};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

use tentacli_crypto::{Decryptor, Encryptor, WardenCrypt};
use tentacli_traits::types::{IncomingPacket, OutgoingPacket};
use tentacli_traits::types::opcodes::Opcode;

cfg_if! {
    if #[cfg(feature = "wotlk_login")] {
        // Proceed
    } else {
        compile_error!("Login feature must be enabled!");
    }
}

pub const INCOME_WORLD_OPCODE_LENGTH: usize = 2;
pub const OUTCOME_WORLD_PACKET_HEADER_LENGTH: usize = 6;

pub struct Reader {
    _stream: BufReader<OwnedReadHalf>,
    _decryptor: Option<Decryptor>,
    _warden_crypt: Arc<SyncMutex<Option<WardenCrypt>>>,
    _need_sync: bool,
}

impl Reader {
    pub fn new(
        reader: OwnedReadHalf,
        warden_crypt: Arc<SyncMutex<Option<WardenCrypt>>>,
        need_sync: bool,
        decryptor: Option<Decryptor>,
    ) -> Self {
        Self {
            _stream: BufReader::new(reader),
            _decryptor: decryptor,
            _warden_crypt: warden_crypt,
            _need_sync: need_sync,
        }
    }

    pub async fn read(&mut self) -> AnyResult<IncomingPacket> {
        let (opcode, body) = if let Some(decryptor) = self._decryptor.as_mut() {
            let mut header = vec![0u8; 4];
            self._stream.read_exact(&mut header).await?;

            if !self._need_sync {
                decryptor.decrypt(&mut header);
            } else {
                self._need_sync = false;
            }

            let is_long_packet = header[0] >= 0x80;

            if is_long_packet {
                let extra_byte = self._stream.read_u8().await?;
                header.push(extra_byte);
            }

            let mut header_reader = Cursor::new(&header);
            let size = if is_long_packet {
                byteorder::ReadBytesExt::read_u24::<BigEndian>(&mut header_reader)? as usize
            } else {
                byteorder::ReadBytesExt::read_u16::<BigEndian>(&mut header_reader)? as usize
            };

            let opcode = byteorder::ReadBytesExt::read_u16::<LittleEndian>(&mut header_reader)?;

            let mut body = vec![0u8; size - INCOME_WORLD_OPCODE_LENGTH];
            self._stream.read_exact(&mut body).await?;

            if opcode == Opcode::SMSG_WARDEN_DATA {
                if let Some(ref mut crypt) = *self._warden_crypt.lock().unwrap() {
                    crypt.decrypt(&mut body);
                }
            }

            (opcode, body)
        } else {
            let opcode = self._stream.read_u8().await?;
            let body = match opcode {
                Opcode::LOGIN_CHALLENGE => {
                    let mut buf = vec![0u8; 32];
                    self._stream.read_exact(&mut buf).await?;
                    buf
                }
                Opcode::LOGIN_PROOF => {
                    let mut buf = vec![0u8; 32];
                    self._stream.read_exact(&mut buf).await?;
                    buf
                }
                Opcode::REALM_LIST => {
                    let mut buf = vec![0u8; 128];
                    self._stream.read_exact(&mut buf).await?;
                    buf
                }
                _ => vec![],
            };

            (opcode as u16, body)
        };

        Ok(IncomingPacket {
            opcode,
            body,
            header: vec![],
        })
    }
}

pub struct Writer {
    _stream: OwnedWriteHalf,
    _encryptor: Option<Encryptor>,
    _warden_crypt: Arc<SyncMutex<Option<WardenCrypt>>>,
    _need_sync: bool,
}

impl Writer {
    pub fn new(
        writer: OwnedWriteHalf,
        warden_crypt: Arc<SyncMutex<Option<WardenCrypt>>>,
        need_sync: bool,
        encryptor: Option<Encryptor>,
    ) -> Self {
        Self {
            _stream: writer,
            _encryptor: encryptor,
            _warden_crypt: warden_crypt,
            _need_sync: need_sync,
        }
    }

    pub async fn write(&mut self, packet: &OutgoingPacket) -> AnyResult<usize> {
        let packet_bytes = match self._encryptor.as_mut() {
            Some(encryptor) => {
                if self._need_sync {
                    self._need_sync = false;
                    packet.data.to_vec()
                } else {
                    let mut header = packet.data[..OUTCOME_WORLD_PACKET_HEADER_LENGTH].to_vec();
                    encryptor.encrypt(&mut header);

                    let body = if packet.opcode == Opcode::CMSG_WARDEN_DATA {
                        let mut body = packet.data[OUTCOME_WORLD_PACKET_HEADER_LENGTH..].to_vec();
                        if let Some(ref mut crypt) = *self._warden_crypt.lock().unwrap() {
                            crypt.encrypt(&mut body);
                        }
                        body
                    } else {
                        packet.data[OUTCOME_WORLD_PACKET_HEADER_LENGTH..].to_vec()
                    };

                    [header, body].concat()
                }
            }
            None => packet.data.to_vec(),
        };

        let written = self._stream.write(&packet_bytes).await?;
        self._stream.flush().await?;
        Ok(written)
    }
}
