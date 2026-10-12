//! Dependency-free byte core for Export.
//!
//! One little-endian container, a streaming SHA-256, a CRC, and magic-byte sniffing.
//! No codecs and no network live here.

mod container;
mod crc32;
mod sha256;
mod sniff;

pub use container::{read_box, write_box, Error as BoxError, Section, MAGIC, VERSION};
pub use crc32::crc32;
pub use sha256::{hash as sha256, Sha256};
pub use sniff::{sniff, Kind};
