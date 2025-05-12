use anyhow::Result as AnyResult;
use async_broadcast::{Sender as BroadcastSender, Receiver as BroadcastReceiver};
use tokio::task::JoinHandle;
use colored::*;
use tentacli_traits::{Feature, FeatureError};
use tentacli_traits::types::HandlerOutput;

#[derive(Default)]
pub struct Console {
    _receiver: Option<BroadcastReceiver<HandlerOutput>>,
    _sender: Option<BroadcastSender<HandlerOutput>>,
}

impl Feature for Console {
    fn set_broadcast_channel(
        &mut self,
        sender: BroadcastSender<HandlerOutput>,
        receiver: BroadcastReceiver<HandlerOutput>,
    ) {
        self._sender = Some(sender);
        self._receiver = Some(receiver);
    }

    fn get_tasks(&mut self) -> AnyResult<Vec<JoinHandle<AnyResult<()>>>> {
        let mut receiver = self
            ._receiver
            .as_mut()
            .ok_or(FeatureError::ReceiverNotFound)?
            .clone();

        let handle_input = || {
            tokio::spawn(async move {
                loop {
                    if let Ok(output) = receiver.recv().await {
                        match output {
                            HandlerOutput::SuccessMessage(message, _) => {
                                println!("{}", format!("[SUCCESS]: {}", message).bright_green());
                            }
                            HandlerOutput::ErrorMessage(message, _) => {
                                println!("{}", format!("[ERROR]: {}", message).bright_red());
                            }
                            HandlerOutput::DebugMessage(message, _) => {
                                println!("{}", format!("[DEBUG]: {}", message).bright_black());
                            }
                            HandlerOutput::ResponseMessage(message, _) => {
                                println!("{}", format!("[RECV]: {}", message).bright_magenta());
                            }
                            HandlerOutput::RequestMessage(message, _) => {
                                println!("{}", format!("[SEND]: {}", message).bright_cyan());
                            }
                            _ => {}
                        }

                        break Ok(());
                    }
                }
            })
        };

        Ok(vec![handle_input()])
    }
}
