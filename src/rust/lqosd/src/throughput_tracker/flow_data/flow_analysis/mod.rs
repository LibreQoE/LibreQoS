use allocative_derive::Allocative;
use lqos_sys::flowbee_data::FlowbeeKey;
use once_cell::sync::Lazy;
use serde::Serialize;
use std::{net::IpAddr, sync::Mutex};
use tracing::error;

mod asn;
mod protocol;
use super::AsnId;
pub use protocol::FlowProtocol;
mod finished_flows;
pub use finished_flows::FinishedFlowAnalysis;
pub use finished_flows::RECENT_FLOWS;
mod kernel_ringbuffer;
pub use kernel_ringbuffer::*;
mod rtt_types;
use crate::throughput_tracker::flow_data::flow_analysis::asn::AsnNameCountryFlag;
pub use finished_flows::{AsnCountryListEntry, AsnListEntry, AsnProtocolListEntry};
pub use rtt_types::RttData;

static ANALYSIS: Lazy<FlowAnalysisSystem> = Lazy::new(|| FlowAnalysisSystem::new());

pub struct FlowAnalysisSystem {
    asn_table: Mutex<Option<asn::GeoTable>>,
}

impl FlowAnalysisSystem {
    pub fn new() -> Self {
        // Moved from being periodically updated to being updated on startup
        let _ = std::thread::Builder::new()
            .name("GeoTable Updater".to_string())
            .spawn(|| {
                loop {
                    let result = asn::GeoTable::load();
                    match result {
                        Ok(table) => {
                            ANALYSIS.asn_table.lock().unwrap().replace(table);
                        }
                        Err(e) => {
                            error!("Failed to update ASN table: {e}");
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_secs(60 * 60));
                }
            });

        Self {
            asn_table: Mutex::new(None),
        }
    }

    pub fn len_and_capacity() -> (usize, usize, usize, usize) {
        if let Ok(lock) = ANALYSIS.asn_table.lock() {
            if let Some(table) = lock.as_ref() {
                table.len()
            } else {
                (0, 0, 0, 0)
            }
        } else {
            (0, 0, 0, 0)
        }
    }
}

pub fn setup_flow_analysis() -> anyhow::Result<()> {
    // This is locking the table, which triggers lazy-loading of the
    // data. It's not actually doing nothing.
    let e = ANALYSIS.asn_table.lock();
    if e.is_err() {
        anyhow::bail!("Failed to lock ASN table");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Allocative)]
pub struct FlowAnalysis {
    pub asn_id: AsnId,
    pub protocol_analysis: FlowProtocol,
}

impl FlowAnalysis {
    pub fn new(key: &FlowbeeKey) -> Self {
        let asn_id = lookup_asn_id(key.remote_ip.as_ip());
        let protocol_analysis = FlowProtocol::new(key);
        Self {
            asn_id: AsnId(asn_id.unwrap_or(0)),
            protocol_analysis,
        }
    }
}

pub fn lookup_asn_id(ip: IpAddr) -> Option<u32> {
    if let Ok(table_lock) = ANALYSIS.asn_table.lock() {
        if let Some(table) = table_lock.as_ref() {
            return table.find_asn(ip);
        }
    }
    None
}

pub fn get_asn_name_and_country(ip: IpAddr) -> AsnNameCountryFlag {
    if let Ok(table_lock) = ANALYSIS.asn_table.lock() {
        if let Some(table) = table_lock.as_ref() {
            return table.find_owners_by_ip(ip);
        }
    }
    AsnNameCountryFlag::default()
}

pub fn get_asn_lat_lon(ip: IpAddr) -> (f64, f64) {
    if let Ok(table_lock) = ANALYSIS.asn_table.lock() {
        if let Some(table) = table_lock.as_ref() {
            return table.find_lat_lon_by_ip(ip);
        }
    }
    (0.0, 0.0)
}

pub fn get_asn_name_by_id(id: u32) -> String {
    if let Ok(table_lock) = ANALYSIS.asn_table.lock() {
        if let Some(table) = table_lock.as_ref() {
            return table.find_name_by_id(id);
        }
    }
    "Unknown".to_string()
}
