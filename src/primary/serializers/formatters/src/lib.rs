use serde::Serialize;
use tentacli_packet::{LoginPacket, WorldPacket, Segment};
use crate::{depends_on, conditional};
mod json_formatter;

pub use json_formatter::JsonFormatter;