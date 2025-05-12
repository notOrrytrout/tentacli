use std::sync::{Arc, Mutex as SyncMutex, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen}
};
use futures::{FutureExt, StreamExt};
use crossterm::event::EventStream;
use tokio::task::JoinHandle;
use async_broadcast::{Receiver as BroadcastReceiver, Sender as BroadcastSender};
use tokio::time::sleep;
use tui::backend::CrosstermBackend;
use tui::layout::{Constraint, Direction, Layout};
use tui::Terminal;
use tentacli_traits::{Feature, FeatureError};
use tentacli_traits::types::HandlerOutput;

mod characters_modal;
mod debug_panel;
mod info_panel;
mod realm_modal;
mod debug_details_panel;
pub mod types;
mod title;
mod traits;

use crate::features::ui::traits::{UIComponent, UIModalComponent};
use crate::features::ui::characters_modal::CharactersModal;
use crate::features::ui::debug_panel::DebugPanel;
use crate::features::ui::realm_modal::RealmModal;
use crate::features::ui::title::Title;
use crate::features::ui::types::{LoggerOutput, UIEventFlags};

pub const MARGIN: u16 = 1;

#[derive(Default)]
pub struct UI {
    _receiver: Option<BroadcastReceiver<HandlerOutput>>,
    _sender: Option<BroadcastSender<HandlerOutput>>,
}

impl Feature for UI {
    fn set_broadcast_channel(
        &mut self,
        sender: BroadcastSender<HandlerOutput>,
        receiver: BroadcastReceiver<HandlerOutput>
    ) {
        self._sender = Some(sender);
        self._receiver = Some(receiver);
    }

    fn get_tasks(&mut self) -> Result<Vec<JoinHandle<Result<(), anyhow::Error>>>> {
        let sender = self._sender.as_ref().ok_or(FeatureError::SenderNotFound)?.clone();
        let mut receiver = self._receiver.as_mut().ok_or(FeatureError::ReceiverNotFound)?.clone();

        enable_raw_mode()?;
        execute!(std::io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;

        let terminal = Arc::new(SyncMutex::new(Terminal::new(CrosstermBackend::new(std::io::stdout()))?));

        let shutdown = Arc::new(AtomicBool::new(false));
        let event_flags = Arc::new(SyncMutex::new(UIEventFlags::NONE));
        let characters_modal = Arc::new(SyncMutex::new(CharactersModal::new()));
        let debug_panel = Arc::new(SyncMutex::new(DebugPanel::new()));
        let realm_modal = Arc::new(SyncMutex::new(RealmModal::new()));
        let title = Arc::new(SyncMutex::new(Title::new()));

        let handle_events = {
            let shutdown = Arc::clone(&shutdown);
            let terminal = Arc::clone(&terminal);
            let event_flags = Arc::clone(&event_flags);
            let characters_modal = Arc::clone(&characters_modal);
            let debug_panel = Arc::clone(&debug_panel);
            let realm_modal = Arc::clone(&realm_modal);
            let sender = sender.clone();

            tokio::spawn(async move {
                let mut reader = EventStream::new();
                while !shutdown.load(Ordering::Relaxed) {
                    let delay = sleep(Duration::from_millis(100)).fuse();
                    let next_event = reader.next().fuse();

                    tokio::select! {
                        _ = delay => {},
                        maybe_event = next_event => {
                            if let Some(Ok(event)) = maybe_event {
                                if let Event::Key(key) = event {
                                    let crossterm::event::KeyEvent { modifiers, code, .. } = key;

                                    event_flags.lock().unwrap().set(UIEventFlags::IS_EVENT_HANDLED, false);

                                    let outputs: Vec<HandlerOutput> = vec![
                                        characters_modal.lock().unwrap().handle_key_event(modifiers, code, Arc::clone(&event_flags)),
                                        realm_modal.lock().unwrap().handle_key_event(modifiers, code, Arc::clone(&event_flags)),
                                        debug_panel.lock().unwrap().handle_key_event(modifiers, code, Arc::clone(&event_flags)),
                                    ]
                                    .into_iter()
                                    .filter_map(|x| x)
                                    .collect();

                                    for output in outputs {
                                        let _ = sender.broadcast(output).await;
                                    }

                                    if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
                                        let is_exit_requested = event_flags.lock().unwrap()
                                            .contains(UIEventFlags::IS_EXIT_REQUESTED);

                                        if is_exit_requested {
                                            shutdown.store(true, Ordering::Relaxed);
                                        } else {
                                            event_flags.lock().unwrap().set(UIEventFlags::IS_EXIT_REQUESTED, true);
                                            let _ = sender.broadcast(HandlerOutput::DebugMessage(
                                                "Starting logout, please wait OR press Ctrl+C again for quick quit".to_string(),
                                                None
                                            )).await;
                                            let _ = sender.broadcast(HandlerOutput::ExitRequest).await;
                                        }
                                    }
                                } else if let Event::Resize(_, _) = event {
                                    let _ = terminal.lock().unwrap().autoresize();
                                }
                            }
                        }
                    }
                }

                Ok(())
            })
        };

        let handle_input = {
            let debug_panel = Arc::clone(&debug_panel);
            let characters_modal = Arc::clone(&characters_modal);
            let realm_modal = Arc::clone(&realm_modal);
            let event_flags = Arc::clone(&event_flags);

            tokio::spawn(async move {
                while let Ok(output) = receiver.recv().await {
                    match output {
                        HandlerOutput::SuccessMessage(message, details) => {
                            debug_panel.lock().unwrap().add_item(LoggerOutput::Success(message, details));
                        }
                        HandlerOutput::ErrorMessage(message, details) => {
                            debug_panel.lock().unwrap().add_item(LoggerOutput::Error(message, details));
                        }
                        HandlerOutput::DebugMessage(message, details) => {
                            debug_panel.lock().unwrap().add_item(LoggerOutput::Debug(message, details));
                        }
                        HandlerOutput::ResponseMessage(message, details) => {
                            debug_panel.lock().unwrap().add_item(LoggerOutput::Response(message, details));
                        }
                        HandlerOutput::RequestMessage(message, details) => {
                            debug_panel.lock().unwrap().add_item(LoggerOutput::Request(message, details));
                        }
                        HandlerOutput::TransferCharactersList(characters) => {
                            event_flags.lock().unwrap().set(UIEventFlags::IS_CHARACTERS_MODAL_OPENED, true);
                            characters_modal.lock().unwrap().set_items(characters);
                        }
                        HandlerOutput::TransferRealmsList(realms) => {
                            event_flags.lock().unwrap().set(UIEventFlags::IS_REALM_MODAL_OPENED, true);
                            realm_modal.lock().unwrap().set_items(realms);
                        }
                        _ => {}
                    }
                }
                Ok(())
            })
        };

        let handle_render = {
            let shutdown = Arc::clone(&shutdown);
            let terminal = Arc::clone(&terminal);
            let event_flags = Arc::clone(&event_flags);
            let characters_modal = Arc::clone(&characters_modal);
            let debug_panel = Arc::clone(&debug_panel);
            let realm_modal = Arc::clone(&realm_modal);
            let title = Arc::clone(&title);

            tokio::spawn(async move {
                {
                    let mut term = terminal.lock().unwrap();
                    term.clear()?;
                    term.hide_cursor()?;
                }

                while !shutdown.load(Ordering::Relaxed) {
                    terminal.lock().unwrap().draw(|frame| {
                        let chunks = Layout::default()
                            .direction(Direction::Vertical)
                            .margin(MARGIN)
                            .constraints([
                                Constraint::Length(3),
                                Constraint::Percentage(92),
                            ])
                            .split(frame.size());

                        title.lock().unwrap().render(frame, chunks[0]);
                        debug_panel.lock().unwrap().render(frame, chunks[1]);

                        if event_flags.lock().unwrap().contains(UIEventFlags::IS_CHARACTERS_MODAL_OPENED) {
                            characters_modal.lock().unwrap().render(frame, chunks[1]);
                        }

                        if event_flags.lock().unwrap().contains(UIEventFlags::IS_REALM_MODAL_OPENED) {
                            realm_modal.lock().unwrap().render(frame, chunks[1]);
                        }
                    })?;

                    sleep(Duration::from_millis(30)).await;
                }

                let _ = disable_raw_mode();
                let _ = execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
                Ok(())
            })
        };

        Ok(vec![
            handle_events,
            handle_input,
            handle_render,
        ])
    }
}
