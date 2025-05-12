use serde::Serialize;
use tentacli_packet::{LoginPacket, WorldPacket, Segment};
use crate::{depends_on, conditional};
mod rc4;
mod srp;
mod warden_crypt;

pub use rc4::{Encryptor, Decryptor, RC4};
pub use srp::Srp;
pub use warden_crypt::WardenCrypt;