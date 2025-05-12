use serde::Serialize;
use tentacli_packet::{LoginPacket, WorldPacket, Segment};
use crate::{depends_on, conditional};
mod data_storage;
mod session;

pub use data_storage::DataStorage;
pub use session::{Session, ActionFlags, StateFlags};