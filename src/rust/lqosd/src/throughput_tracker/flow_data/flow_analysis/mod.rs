use std::{net::IpAddr, sync::Mutex};
use lqos_sys::flowbee_data::FlowbeeKey;
use once_cell::sync::Lazy;
mod asn;
mod protocol;
pub use protocol::FlowProtocol;
use super::AsnId;
mod finished_flows;
pub use finished_flows::FinishedFlowAnalysis;
pub use finished_flows::RECENT_FLOWS;
mod kernel_ringbuffer;
pub use kernel_ringbuffer::*;
mod rtt_types;
pub use rtt_types::RttData;

static ANALYSIS: Lazy<FlowAnalysisSystem> = Lazy::new(|| FlowAnalysisSystem::new());

pub struct FlowAnalysisSystem {
    asn_table: Mutex<Option<asn::GeoTable>>,
}

impl FlowAnalysisSystem {
    pub fn new() -> Self {
        // Periodically update the ASN table
        std::thread::spawn(|| {
            loop {
                let result = asn::GeoTable::load();
                match result {
                    Ok(table) => {
                        ANALYSIS.asn_table.lock().unwrap().replace(table);
                    }
                    Err(e) => {
                        log::error!("Failed to update ASN table: {e}");
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(60 * 60 * 24));
            }
        });

        Self {
            asn_table: Mutex::new(None),
        }
    }
}

pub fn setup_flow_analysis() -> anyhow::Result<()> {
    let e = ANALYSIS.asn_table.lock();
    if e.is_err() {
        anyhow::bail!("Failed to lock ASN table");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

pub fn get_asn_name_and_country(ip: IpAddr) -> (String, String) {
    if let Ok(table_lock) = ANALYSIS.asn_table.lock() {
        if let Some(table) = table_lock.as_ref() {
            return table.find_owners_by_ip(ip);
        }
    }
    (String::new(), String::new())
}

pub fn get_asn_lat_lon(ip: IpAddr) -> (f64, f64) {
    if let Ok(table_lock) = ANALYSIS.asn_table.lock() {
        if let Some(table) = table_lock.as_ref() {
            return table.find_lat_lon_by_ip(ip);
        }
    }
    (0.0, 0.0)
}