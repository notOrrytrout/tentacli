use serde::Serialize;
use tentacli_packet::{LoginPacket, WorldPacket, Segment};
use crate::{depends_on, conditional};
mod packed_guid;

pub use packed_guid::{PackedGuid};