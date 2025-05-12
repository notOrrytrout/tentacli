use serde::Serialize;
use tentacli_packet::{LoginPacket, WorldPacket, Segment};
use crate::{depends_on, conditional};
use async_trait::async_trait;

use crate::types::{HandlerInput, HandlerResult};

#[async_trait]
pub trait PacketHandler {
    async fn handle(&mut self, input: &mut HandlerInput) -> HandlerResult;
}