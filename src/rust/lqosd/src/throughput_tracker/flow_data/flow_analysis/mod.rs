use std::sync::Mutex;
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
