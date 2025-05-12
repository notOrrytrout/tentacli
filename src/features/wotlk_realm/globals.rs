// Opcode::CMSG_CHAR_ENUM
use serde::Serialize;
use tentacli_packet::WorldPacket;
#[derive(WorldPacket, Serialize, Debug, Default)]
pub struct CharacterEnumOutgoing {}