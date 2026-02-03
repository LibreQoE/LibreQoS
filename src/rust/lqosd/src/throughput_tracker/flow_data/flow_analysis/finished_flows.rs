use super::{FlowAnalysis, get_asn_lat_lon, get_asn_name_and_country, get_asn_name_by_id, FlowbeeEffectiveDirection};
use crate::shaped_devices_tracker::SHAPED_DEVICES;
use crate::throughput_tracker::flow_data::FlowbeeLocalData;
use allocative_derive::Allocative;
use crossbeam_channel::Sender;
use fxhash::FxHashMap;
use itertools::Itertools;
use lqos_bus::BusResponse;
use lqos_config::load_config;
use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::{boot_time_nanos_to_unix_now, unix_now};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use std::time::Duration;
use tracing::debug;

pub struct TimeBuffer {
    buffer: Mutex<Vec<TimeEntry>>,
}

#[derive(Clone, Allocative)]
struct TimeEntry {
    time: u64,
    data: (FlowbeeKey, FlowbeeLocalData, FlowAnalysis),
}

#[derive(Debug, Serialize)]
pub struct FlowDurationSummary {
    pub count: usize,
    pub duration: u64,
}

#[derive(Debug, Serialize)]
pub struct AsnListEntry {
    pub count: usize,
    pub asn: u32,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct AsnCountryListEntry {
    pub count: usize,
    pub name: String,
    pub iso_code: String,
}

#[derive(Debug, Serialize)]
pub struct AsnProtocolListEntry {
    pub count: usize,
    pub protocol: String,
}

impl TimeBuffer {
    fn new() -> Self {
        Self {
            buffer: Mutex::new(Vec::new()),
        }
    }

    fn expire_over_one_minutes(&self) {
        if let Ok(now) = unix_now() {
            let one_minute_ago = now - 60;
            let mut buffer = self.buffer.lock();
            buffer.retain(|v| v.time > one_minute_ago);
            buffer.shrink_to_fit();
        }
    }

    fn push(&self, entry: TimeEntry) {
        let mut buffer = self.buffer.lock();
        buffer.push(entry);
    }

    pub fn lat_lon_endpoints(&self) -> Vec<(f64, f64, String, u64, f32)> {
        let buffer = self.buffer.lock();
        let mut my_buffer = buffer
            .iter()
            .map(|v| {
                let (key, data, _analysis) = &v.data;
                let (lat, lon) = get_asn_lat_lon(key.remote_ip.as_ip());
                let geo = get_asn_name_and_country(key.remote_ip.as_ip());
                (
                    lat,
                    lon,
                    geo.country,
                    data.bytes_sent.down,
                    data.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Download) as f32, // TODO: Fix this type
                )
            })
            .filter(|(lat, lon, ..)| *lat != 0.0 && *lon != 0.0)
            .collect::<Vec<(f64, f64, String, u64, f32)>>();

        // Sort by lat/lon (total ordering for floats)
        my_buffer.sort_by(|a, b| a.0.total_cmp(&b.0));

        // Deduplicate
        my_buffer.dedup();

        my_buffer
    }

    pub fn country_summary(&self) -> Vec<(String, DownUpOrder<u64>, [f32; 2], String)> {
        let buffer = self.buffer.lock();
        let mut my_buffer = buffer
            .iter()
            .map(|v| {
                let (key, data, _analysis) = &v.data;
                let geo = get_asn_name_and_country(key.remote_ip.as_ip());
                let rtt = [data.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Download) as f32, data.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Upload) as f32]; // TODO: Fix these types
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
                    ));
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
        slice.sort_by(|a, b| a.cmp(b));
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
        slice.sort_by(|a, b| a.total_cmp(b));
        let mid = slice.len() / 2;
        if slice.len() % 2 == 0 {
            (slice[mid] + slice[mid - 1]) / 2.0
        } else {
            slice[mid]
        }
    }

    pub fn ether_protocol_summary(&self) -> BusResponse {
        let buffer = self.buffer.lock();

        let mut v4_bytes_sent = DownUpOrder::zeroed();
        let mut v4_packets_sent = DownUpOrder::zeroed();
        let mut v6_bytes_sent = DownUpOrder::zeroed();
        let mut v6_packets_sent = DownUpOrder::zeroed();
        let mut v4_rtt = [Vec::new(), Vec::new()];
        let mut v6_rtt = [Vec::new(), Vec::new()];

        buffer.iter().for_each(|v| {
            let (key, data, _analysis) = &v.data;
            if key.local_ip.is_v4() {
                // It's V4
                v4_bytes_sent.checked_add(data.bytes_sent);
                v4_packets_sent.checked_add(data.packets_sent);
                // TODO: This is awful code, fix it.
                if data.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Download) > 0 {
                    v4_rtt[0].push(data.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Download));
                }
                if data.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Upload) > 0 {
                    v4_rtt[1].push(data.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Upload));
                }
            } else {
                // It's V6
                v6_bytes_sent.checked_add(data.bytes_sent);
                v6_packets_sent.checked_add(data.packets_sent);
                // TODO: This is awful code, fix it.
                if data.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Download) > 0 {
                    v6_rtt[0].push(data.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Download));
                }
                if data.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Upload) > 0 {
                    v6_rtt[1].push(data.get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Upload));
                }
            }
        });

        let v4_rtt = DownUpOrder::new(Self::median(&v4_rtt[0]), Self::median(&v4_rtt[1]));
        let v6_rtt = DownUpOrder::new(Self::median(&v6_rtt[0]), Self::median(&v6_rtt[1]));

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
        let buffer = self.buffer.lock();

        let mut results = FxHashMap::default();

        buffer.iter().for_each(|v| {
            let (_key, data, analysis) = &v.data;
            let proto = analysis.protocol_analysis.to_string();
            let entry = results.entry(proto).or_insert(DownUpOrder::zeroed());
            entry.checked_add(data.bytes_sent);
        });

        let mut results = results
            .into_iter()
            .collect::<Vec<(String, DownUpOrder<u64>)>>();
        results.sort_by(|a, b| b.1.down.cmp(&a.1.down));
        // Keep only the top 10
        results.truncate(10);
        results
    }

    pub fn flow_duration_summary(&self) -> Vec<FlowDurationSummary> {
        let buffer = self.buffer.lock();

        buffer
            .iter()
            .map(|f| Duration::from_nanos(f.data.1.last_seen.saturating_sub(f.data.1.start_time))) // Duration in nanoseconds
            .map(|nanos| nanos.as_secs())
            .sorted()
            .dedup_with_count() // Now we're (count, duration in seconds)
            .map(|(count, duration)| FlowDurationSummary { count, duration })
            .collect()
    }

    pub fn all_flows_for_asn(&self, id: u32) -> Vec<(FlowbeeKey, FlowbeeLocalData, FlowAnalysis)> {
        let buffer = self.buffer.lock();
        buffer
            .iter()
            .filter(|flow| flow.data.2.asn_id.0 == id)
            .map(|flow| flow.data.clone())
            .collect()
    }

    pub fn all_flows_for_country(
        &self,
        iso_code: &str,
    ) -> Vec<(FlowbeeKey, FlowbeeLocalData, FlowAnalysis)> {
        let buffer = self.buffer.lock();
        buffer
            .iter()
            .filter(|flow| {
                let country = get_asn_name_and_country(flow.data.0.remote_ip.as_ip());
                country.flag == iso_code
            })
            .map(|flow| flow.data.clone())
            .collect()
    }

    pub fn all_flows_for_protocol(
        &self,
        protocol_name: &str,
    ) -> Vec<(FlowbeeKey, FlowbeeLocalData, FlowAnalysis)> {
        let buffer = self.buffer.lock();
        buffer
            .iter()
            .filter(|flow| flow.data.2.protocol_analysis.to_string() == protocol_name)
            .map(|flow| flow.data.clone())
            .collect()
    }

    /// Builds a list of all ASNs with recent data, and how many flows they have.
    pub fn asn_list(&self) -> Vec<AsnListEntry> {
        // 1: Clone: large operation, don't keep the buffer locked longer than we have to
        let buffer = {
            let buffer = self.buffer.lock();
            buffer.clone()
        };

        // Filter out short flows and reduce to the ASN ID# only
        let mut buffer: Vec<_> = buffer
            .into_iter()
            .filter(|flow| {
                // Total flow time > 3 seconds
                flow.data.1.last_seen - flow.data.1.start_time > 3_000_000_000
            })
            .map(|flow| flow.data.2.asn_id.0)
            .collect();

        // Sort the buffer
        buffer.sort_unstable();

        // Deduplicate and count, decorate with name
        buffer
            .into_iter()
            .sorted()
            .dedup_with_count()
            .map(|(count, asn)| AsnListEntry {
                count,
                asn,
                name: get_asn_name_by_id(asn),
            })
            .collect()
    }

    /// Builds a list of ASNs by country with recent data, and how many flows they have.
    pub fn country_list(&self) -> Vec<AsnCountryListEntry> {
        // 1: Clone: large operation, don't keep the buffer locked longer than we have to
        let buffer = {
            let buffer = self.buffer.lock();
            buffer.clone()
        };

        // Filter out the short flows and get the country & flag
        let mut buffer: Vec<(String, String)> = buffer
            .into_iter()
            .filter(|flow| {
                // Total flow time > 3 seconds
                flow.data.1.last_seen - flow.data.1.start_time > 3_000_000_000
            })
            .map(|flow| {
                let country = get_asn_name_and_country(flow.data.0.remote_ip.as_ip());
                (country.country, country.flag)
            })
            .collect();

        // Sort the buffer
        buffer.sort_unstable_by(|a, b| a.0.cmp(&b.0));

        // Deduplicate and count, decorate with name
        buffer
            .into_iter()
            .sorted()
            .dedup_with_count()
            .map(|(count, asn)| AsnCountryListEntry {
                count,
                name: asn.0,
                iso_code: asn.1,
            })
            .collect()
    }

    /// Builds a list of protocols with recent data, and how many flows they have.
    pub fn protocol_list(&self) -> Vec<AsnProtocolListEntry> {
        // 1: Clone: large operation, don't keep the buffer locked longer than we have to
        let buffer = {
            let buffer = self.buffer.lock();
            buffer.clone()
        };

        // Filter out the short flows and get the country & flag
        let mut buffer: Vec<String> = buffer
            .into_iter()
            .filter(|flow| {
                // Total flow time > 3 seconds
                flow.data.1.last_seen - flow.data.1.start_time > 3_000_000_000
            })
            .map(|flow| flow.data.2.protocol_analysis.to_string())
            .collect();

        // Sort the buffer
        buffer.sort_unstable_by(|a, b| a.cmp(&b));

        // Deduplicate and count, decorate with name
        buffer
            .into_iter()
            .sorted()
            .dedup_with_count()
            .map(|(count, protocol)| AsnProtocolListEntry { count, protocol })
            .collect()
    }

    #[allow(dead_code)]
    pub fn len_and_capacity(&self) -> (usize, usize) {
        let buffer = self.buffer.lock();
        (buffer.len(), buffer.capacity())
    }
}

pub static RECENT_FLOWS: Lazy<TimeBuffer> = Lazy::new(|| TimeBuffer::new());

pub struct FinishedFlowAnalysis {}

impl FinishedFlowAnalysis {
    pub fn new() -> Sender<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))> {
        debug!("Created Flow Analysis Endpoint");
        let (tx, rx) =
            crossbeam_channel::bounded::<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>(65535);

        let _ = std::thread::Builder::new()
            .name("Flow Expiration".to_string())
            .spawn(|| {
                loop {
                    RECENT_FLOWS.expire_over_one_minutes();
                    std::thread::sleep(std::time::Duration::from_secs(10));
                }
            });
        let _ = std::thread::Builder::new()
            .name("Flow Analysis".to_string())
            .spawn(move || {
                while let Ok((key, (data, analysis))) = rx.recv() {
                    enqueue(key, data, analysis);
                }
                tracing::error!("Flow Analysis thread died");
            });

        tx
    }
}

fn enqueue(key: FlowbeeKey, data: FlowbeeLocalData, analysis: FlowAnalysis) {
    debug!("Finished flow analysis");
    let start_time = boot_time_nanos_to_unix_now(data.start_time).unwrap_or(0);
    let last_seen = boot_time_nanos_to_unix_now(data.last_seen).unwrap_or(0);

    let one_way = data.bytes_sent.down == 0 || data.bytes_sent.up == 0;
    let sd = SHAPED_DEVICES.load();
    let circuit_hash = sd.get_circuit_hash_from_ip(&key.local_ip);

    if !one_way {
        //data.trim(); // Remove the trailing 30 seconds of zeroes
        //let tp_buf_dn = data.throughput_buffer.iter().map(|v| v.down).collect();
        //let tp_buf_up = data.throughput_buffer.iter().map(|v| v.up).collect();

        let retransmit_times_down = data
            .get_retry_times_down()
            .iter()
            .filter(|n| **n > 0)
            .map(|t| boot_time_nanos_to_unix_now(*t).unwrap_or(0) as i64)
            .collect();
        let retransmit_times_up = data
            .get_retry_times_up()
            .iter()
            .filter(|n| **n > 0)
            .map(|t| boot_time_nanos_to_unix_now(*t).unwrap_or(0) as i64)
            .collect();

        if let Err(e) = crate::lts2_sys::two_way_flow(
            start_time,
            last_seen,
            key.local_ip.as_ip(),
            key.remote_ip.as_ip(),
            key.ip_protocol,
            key.dst_port,
            key.src_port,
            data.bytes_sent.down,
            data.bytes_sent.up,
            data.packets_sent.down as i64,
            data.packets_sent.up as i64,
            retransmit_times_down,
            retransmit_times_up,
            data.get_summary_rtt_as_micros(FlowbeeEffectiveDirection::Download) as f32,
            data.get_summary_rtt_as_micros(FlowbeeEffectiveDirection::Upload) as f32,
            circuit_hash,
        ) {
            debug!("Failed to send two-way flow to LTS2: {e:?}");
        }
        if let Ok(time) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            RECENT_FLOWS.push(TimeEntry {
                time: time.as_secs(),
                data: (key, data, analysis),
            })
        };
    } else {
        // We have a one-way flow!
        let Ok(config) = load_config() else {
            return;
        };
        if !config.long_term_stats.gather_stats {
            return;
        }
        if let Err(e) = crate::lts2_sys::one_way_flow(
            start_time,
            last_seen,
            key.local_ip.as_ip(),
            key.remote_ip.as_ip(),
            key.ip_protocol,
            key.dst_port,
            key.src_port,
            data.bytes_sent.sum(),
            circuit_hash,
        ) {
            debug!("Failed to send one-way flow to LTS2: {e:?}");
        }
    }
}
