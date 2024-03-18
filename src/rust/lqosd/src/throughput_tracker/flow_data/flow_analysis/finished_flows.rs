use super::{get_asn_lat_lon, get_asn_name_and_country, FlowAnalysis};
use crate::throughput_tracker::flow_data::{FlowbeeLocalData, FlowbeeRecipient};
use lqos_sys::flowbee_data::FlowbeeKey;
use once_cell::sync::Lazy;
use std::sync::{Arc, Mutex};

pub struct TimeBuffer {
    buffer: Mutex<Vec<TimeEntry>>,
}

struct TimeEntry {
    time: u64,
    data: (FlowbeeKey, FlowbeeLocalData, FlowAnalysis),
}

impl TimeBuffer {
    fn new() -> Self {
        Self {
            buffer: Mutex::new(Vec::new()),
        }
    }

    fn expire_over_five_minutes(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut buffer = self.buffer.lock().unwrap();
        buffer.retain(|v| now - v.time < 300);
    }

    fn push(&self, entry: TimeEntry) {
        let mut buffer = self.buffer.lock().unwrap();
        buffer.push(entry);
    }

    pub fn lat_lon_endpoints(&self) -> Vec<(f64, f64)> {
        let buffer = self.buffer.lock().unwrap();
        let mut my_buffer = buffer
            .iter()
            .map(|v| {
                let (key, _data, _analysis) = &v.data;
                let (lat, lon) = get_asn_lat_lon(key.remote_ip.as_ip());
                (lat, lon)
            })
            .filter(|(lat, lon)| *lat != 0.0 && *lon != 0.0)
            .collect::<Vec<(f64, f64)>>();

        // Sort by lat/lon
        my_buffer.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

        // Depuplicate
        my_buffer.dedup();

        my_buffer
    }

    pub fn country_summary(&self) -> Vec<(String, [u64; 2], [f32; 2])> {
        let buffer = self.buffer.lock().unwrap();
        let mut my_buffer = buffer
            .iter()
            .map(|v| {
                let (key, data, _analysis) = &v.data;
                let (_name, country) = get_asn_name_and_country(key.remote_ip.as_ip());
                let rtt = [0.0, 0.0];
                (country, data.bytes_sent, rtt)
            })
            .collect::<Vec<(String, [u64; 2], [f32; 2])>>();

        // Sort by country
        my_buffer.sort_by(|a, b| a.0.cmp(&b.0));

        // Summarize by country
        let mut country_summary = Vec::new();
        let mut last_country = String::new();
        let mut total_bytes = [0, 0];
        let mut total_rtt = [0.0f64, 0.0f64];
        let mut rtt_count = [0, 0];
        for (country, bytes, rtt) in my_buffer {
            if last_country != country {
                if !last_country.is_empty() {
                    // Store the country
                    let rtt = [
                        if total_rtt[0] > 0.0 {
                            rtt_count[0] += 1;
                            (total_rtt[0] / rtt_count[0] as f64) as f32
                        } else {
                            0.0
                        },
                        if total_rtt[1] > 0.0 {
                            rtt_count[1] += 1;
                            (total_rtt[1] / rtt_count[1] as f64) as f32
                        } else {
                            0.0
                        },
                    ];

                    if rtt_count[0] > 0 {
                        total_rtt[0] = (total_rtt[0] / rtt_count[0] as f64) as f64;
                    }
                    if rtt_count[1] > 0 {
                        total_rtt[1] = (total_rtt[1] / rtt_count[1] as f64) as f64;
                    }

                    country_summary.push((last_country, total_bytes, total_rtt));
                }

                last_country = country.to_string();
                total_bytes = [0, 0];
                total_rtt = [0.0, 0.0];
                rtt_count = [0, 0];
            }
            total_bytes[0] += bytes[0];
            total_bytes[1] += bytes[1];
            total_rtt[0] += rtt[0] as f64;
            total_rtt[1] += rtt[1] as f64;
            rtt_count[0] += 1;
            rtt_count[1] += 1;
        }

        // Store the last country
        let rtt = [
            if total_rtt[0] > 0.0 {
                (total_rtt[0] / rtt_count[0] as f64) as f32
            } else {
                0.0
            },
            if total_rtt[1] > 0.0 {
                (total_rtt[1] / rtt_count[1] as f64) as f32
            } else {
                0.0
            },
        ];

        country_summary.push((last_country, total_bytes, rtt));

        // Sort by bytes downloaded descending
        country_summary.sort_by(|a, b| b.1[1].cmp(&a.1[1]));

        country_summary
    }
}

pub static RECENT_FLOWS: Lazy<TimeBuffer> = Lazy::new(|| TimeBuffer::new());

pub struct FinishedFlowAnalysis {}

impl FinishedFlowAnalysis {
    pub fn new() -> Arc<Self> {
        log::debug!("Created Flow Analysis Endpoint");

        std::thread::spawn(|| loop {
            RECENT_FLOWS.expire_over_five_minutes();
            std::thread::sleep(std::time::Duration::from_secs(60 * 5));
        });

        Arc::new(Self {})
    }
}

impl FlowbeeRecipient for FinishedFlowAnalysis {
    fn enqueue(&self, key: FlowbeeKey, data: FlowbeeLocalData, analysis: FlowAnalysis) {
        log::debug!("Finished flow analysis");
        RECENT_FLOWS.push(TimeEntry {
            time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            data: (key, data, analysis),
        });
    }
}
