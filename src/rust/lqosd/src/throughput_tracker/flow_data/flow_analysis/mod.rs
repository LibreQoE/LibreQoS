use std::{net::IpAddr, sync::Mutex};
use once_cell::sync::Lazy;

use self::asn::AsnTable;
mod asn;

static ANALYSIS: Lazy<FlowAnalysis> = Lazy::new(|| FlowAnalysis::new());

pub struct FlowAnalysis {
    asn_table: Mutex<Option<asn::AsnTable>>,
}

impl FlowAnalysis {
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