//! Provides a support library for the support tool system.
mod support_info;
pub mod console;
mod sanity_checks;

use std::io::Write;
use std::net::TcpStream;
pub use support_info::gather_all_support_info;
pub use support_info::SupportDump;
pub use sanity_checks::run_sanity_checks;
use crate::console::{error, success};
pub use sanity_checks::SanityChecks;

const REMOTE_SYSTEM: &str = "stats.libreqos.io:9200";

pub fn submit_to_network(dump: SupportDump) {
    // Build the payload
    let serialized = dump.serialize_and_compress().unwrap();
    let length = serialized.len() as u64;
    let mut bytes = Vec::new();
    bytes.extend(1212u32.to_be_bytes());
    bytes.extend(length.to_be_bytes());
    bytes.extend(&serialized);
    bytes.extend(1212u32.to_be_bytes());

    // Do the actual connection
    let stream = TcpStream::connect(REMOTE_SYSTEM);
    if stream.is_err() {
        error(&format!("Unable to connect to {REMOTE_SYSTEM}"));
        println!("{stream:?}");
        return;
    }
    let mut stream = stream.unwrap();
    stream.write_all(&bytes).unwrap();
    success("Submitted to LibreQoS for Analysis. Thank you.");
}