use std::{net::IpAddr, sync::Mutex};
use lqos_sys::flowbee_data::FlowbeeKey;
use once_cell::sync::Lazy;
use self::asn::AsnTable;
mod asn;
mod protocol;
pub use protocol::FlowProtocol;

use super::AsnId;

static ANALYSIS: Lazy<FlowAnalysisSystem> = Lazy::new(|| FlowAnalysisSystem::new());

pub struct FlowAnalysisSystem {
    asn_table: Mutex<Option<asn::AsnTable>>,
}

impl FlowAnalysisSystem {
    pub fn new() -> Self {
        // Periodically update the ASN table
        std::thread::spawn(|| {
            loop {
                let result = AsnTable::new();
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
    let table_lock = ANALYSIS.asn_table.lock();
    if table_lock.is_err() {
        return None;
    }
    let table = table_lock.unwrap();
    if table.is_none() {
        return None;
    }
    let table = table.as_ref().unwrap();
    if let Some(asn) = table.find_asn(ip) {
        Some(asn.asn)
    } else {
        None
    }
}

pub fn get_asn_name_and_country(asn: u32) -> (String, String) {
    let table_lock = ANALYSIS.asn_table.lock();
    if table_lock.is_err() {
        return ("".to_string(), "".to_string());
    }
    let table = table_lock.unwrap();
    if table.is_none() {
        return ("".to_string(), "".to_string());
    }
    let table = table.as_ref().unwrap();
    if let Some(row) = table.find_asn_by_id(asn) {
        (row.owners.clone(), row.country.clone())
    } else {
        ("".to_string(), "".to_string())
    }
}