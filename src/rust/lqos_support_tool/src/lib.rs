//! Provides a support library for the support tool system.
pub mod console;
mod sanity_checks;
mod support_info;

use crate::console::{error, success};
pub use sanity_checks::SanityChecks;
pub use sanity_checks::run_sanity_checks;
use std::io::Write;
use std::net::TcpStream;
pub use support_info::SupportDump;
pub use support_info::gather_all_support_info;

const REMOTE_SYSTEM: &str = "stats.libreqos.io:9200";

pub fn submit_to_network(dump: SupportDump) {
    match submit_to_network_result(dump) {
        Ok(()) => {
            success("Submitted to LibreQoS for Analysis. Thank you.");
        }
        Err(err) => {
            error(&err);
        }
    }
}

pub fn submit_to_network_result(dump: SupportDump) -> Result<(), String> {
    // Build the payload
    let serialized = dump
        .serialize_and_compress()
        .map_err(|e| format!("Failed to serialize support data: {e}"))?;
    let length = serialized.len() as u64;
    let mut bytes = Vec::new();
    bytes.extend(1212u32.to_be_bytes());
    bytes.extend(length.to_be_bytes());
    bytes.extend(&serialized);
    bytes.extend(1212u32.to_be_bytes());

    // Do the actual connection
    let mut stream = TcpStream::connect(REMOTE_SYSTEM)
        .map_err(|e| format!("Unable to connect to {REMOTE_SYSTEM}: {e}"))?;
    stream
        .write_all(&bytes)
        .map_err(|e| format!("Failed to submit support data: {e}"))?;
    Ok(())
}
