use super::{get_asn_lat_lon, get_asn_name_and_country, FlowAnalysis};
use crate::throughput_tracker::flow_data::{FlowbeeLocalData, FlowbeeRecipient};
use fxhash::FxHashMap;
use lqos_bus::BusResponse;
use lqos_sys::flowbee_data::FlowbeeKey;
use once_cell::sync::Lazy;
use std::sync::{Arc, Mutex};
use lqos_utils::units::DownUpOrder;

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

    pub fn len(&self) -> usize {
        let buffer = self.buffer.lock().unwrap();
        buffer.len()
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

    pub fn lat_lon_endpoints(&self) -> Vec<(f64, f64, String, u64, f32)> {
        let buffer = self.buffer.lock().unwrap();
        let mut my_buffer = buffer
            .iter()
            .map(|v| {
                let (key, data, _analysis) = &v.data;
                let (lat, lon) = get_asn_lat_lon(key.remote_ip.as_ip());
                let geo = get_asn_name_and_country(key.remote_ip.as_ip());
                (lat, lon, geo.country, data.bytes_sent.down, data.rtt[0].as_nanos() as f32)
            })
            .filter(|(lat, lon, ..)| *lat != 0.0 && *lon != 0.0)
            .collect::<Vec<(f64, f64, String, u64, f32)>>();

        // Sort by lat/lon
        my_buffer.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

        // Depuplicate
        my_buffer.dedup();

        my_buffer
    }

    pub fn country_summary(&self) -> Vec<(String, DownUpOrder<u64>, [f32; 2], String)> {
        let buffer = self.buffer.lock().unwrap();
        let mut my_buffer = buffer
            .iter()
            .map(|v| {
                let (key, data, _analysis) = &v.data;
                let geo = get_asn_name_and_country(key.remote_ip.as_ip());
                let rtt = [
                    data.rtt[0].as_nanos() as f32,
                    data.rtt[1].as_nanos() as f32,
                ];
                (geo.country, data.bytes_sent, rtt, geo.flag)
            })
            .collect::<Vec<(String, DownUpOrder<u64>, [f32; 2], String)>>();

        // Sort by country
        my_buffer.sort_by(|a, b| a.0.cmp(&b.0));

        // Iterate through the buffer and summarize by country. We want to accumulate
        // all the RTTs into a list, so we can take a MEDIAN.
        let mut last_country = String::new();
        let mut last_flag = String::new();
        let mut country_summary = Vec::new();
        let mut rtt_buffer = [Vec::new(), Vec::new()];
        let mut total_bytes = DownUpOrder::zeroed();
        for (country, bytes, rtt, flag) in my_buffer.iter() {
            if last_country != *country {

                // Store progress (but not the first one)
                if !last_country.is_empty() {
                    country_summary.push((
                        last_country.to_string(),
                        total_bytes.clone(),
                        [
                            Self::median_f32(&rtt_buffer[0]),
                            Self::median_f32(&rtt_buffer[1]),
                        ],
                        last_flag.clone(),
                    )
                    );
                }

                // Clear accumulated stats
                rtt_buffer[0].clear();
                rtt_buffer[1].clear();
                total_bytes = DownUpOrder::zeroed();
            }

            // Accumulate RTTs
            if rtt[0] > 0.0 {
                rtt_buffer[0].push(rtt[0]);
            }
            if rtt[1] > 0.0 {
                rtt_buffer[1].push(rtt[1]);
            }

            // Accumulate traffic
            total_bytes.checked_add(*bytes);

            // Next, please
            last_country = country.clone();
            last_flag = flag.clone();
        }

        // Sort by bytes downloaded descending
        country_summary.sort_by(|a, b| b.1.down.cmp(&a.1.down));

        country_summary
    }

    fn median(slice: &[u64]) -> u64 {
        if slice.is_empty() {
            return 0;
        }
        let mut slice = slice.to_vec();
        slice.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mid = slice.len() / 2;
        if slice.len() % 2 == 0 {
            (slice[mid] + slice[mid - 1]) / 2
        } else {
            slice[mid]
        }
    }

    fn median_f32(slice: &[f32]) -> f32 {
        if slice.is_empty() {
            return 0.0;
        }
        let mut slice = slice.to_vec();
        slice.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mid = slice.len() / 2;
        if slice.len() % 2 == 0 {
            (slice[mid] + slice[mid - 1]) / 2.0
        } else {
            slice[mid]
        }
    }

    pub fn ether_protocol_summary(&self) -> BusResponse {
        let buffer = self.buffer.lock().unwrap();

        let mut v4_bytes_sent = DownUpOrder::zeroed();
        let mut v4_packets_sent = DownUpOrder::zeroed();
        let mut v6_bytes_sent = DownUpOrder::zeroed();
        let mut v6_packets_sent = DownUpOrder::zeroed();
        let mut v4_rtt = [Vec::new(), Vec::new()];
        let mut v6_rtt = [Vec::new(), Vec::new()];

        buffer
            .iter()
            .for_each(|v| {
                let (key, data, _analysis) = &v.data;
                if key.local_ip.is_v4() {
                    // It's V4
                    v4_bytes_sent.checked_add(data.bytes_sent);
                    v4_packets_sent.checked_add(data.packets_sent);
                    if data.rtt[0].as_nanos() > 0 {
                        v4_rtt[0].push(data.rtt[0].as_nanos());
                    }
                    if data.rtt[1].as_nanos() > 0 {
                        v4_rtt[1].push(data.rtt[1].as_nanos());
                    }
                } else {
                    // It's V6
                    v6_bytes_sent.checked_add(data.bytes_sent);
                    v6_packets_sent.checked_add(data.packets_sent);
                    if data.rtt[0].as_nanos() > 0 {
                        v6_rtt[0].push(data.rtt[0].as_nanos());
                    }
                    if data.rtt[1].as_nanos() > 0 {
                        v6_rtt[1].push(data.rtt[1].as_nanos());
                    }

                }
            });
        
        let v4_rtt = DownUpOrder::new(
            Self::median(&v4_rtt[0]),
            Self::median(&v4_rtt[1]),
        );
        let v6_rtt = DownUpOrder::new(
            Self::median(&v6_rtt[0]),
            Self::median(&v6_rtt[1]),
        );

        BusResponse::EtherProtocols {
            v4_bytes: v4_bytes_sent,
            v6_bytes: v6_bytes_sent,
            v4_packets: v4_packets_sent,
            v6_packets: v6_packets_sent,
            v4_rtt,
            v6_rtt,
        }
    }

    pub fn ip_protocol_summary(&self) -> Vec<(String, DownUpOrder<u64>)> {
        let buffer = self.buffer.lock().unwrap();

        let mut results = FxHashMap::default();

        buffer
            .iter()
            .for_each(|v| {
                let (_key, data, analysis) = &v.data;
                let proto = analysis.protocol_analysis.to_string();
                let entry = results.entry(proto).or_insert(DownUpOrder::zeroed());
                entry.checked_add(data.bytes_sent);
            });

        let mut results = results.into_iter().collect::<Vec<(String, DownUpOrder<u64>)>>();
        results.sort_by(|a, b| b.1.down.cmp(&a.1.down));
        // Keep only the top 10
        results.truncate(10);
        results
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
