use serde::{Serialize, Deserialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Default)]
pub struct IngestSession {
    pub license_key: Uuid,
    pub node_id: String,
    pub node_name: String,
    pub shaper_throughput: Vec<ShaperThroughput>,
    pub shaped_devices: Vec<ShapedDevices>,
    pub network_tree: Vec<NetworkTree>,
    pub circuit_throughput: Vec<CircuitThroughput>,
    pub circuit_retransmits: Vec<CircuitRetransmits>,
    pub circuit_rtt: Vec<CircuitRtt>,
    pub circuit_cake_drops: Vec<CircuitCakeDrops>,
    pub circuit_cake_marks: Vec<CircuitCakeMarks>,
    pub site_cake_drops: Vec<SiteCakeDrops>,
    pub site_cake_marks: Vec<SiteCakeMarks>,
    pub site_retransmits: Vec<SiteRetransmits>,
    pub site_rtt: Vec<SiteRtt>,
    pub site_throughput: Vec<SiteThroughput>,
    pub shaper_utilization: Option<Vec<ShaperUtilization>>,
    pub one_way_flows: Option<Vec<OneWayFlow>>,
    pub two_way_flows: Option<Vec<TwoWayFlow>>,
    pub allowed_ips: Option<Vec<String>>,
    pub ignored_ips: Option<Vec<String>>,
    pub blackboard_json: Option<Vec<u8>>,
    pub flow_count: Option<Vec<FlowCount>>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum RemoteCommand {
    Log(String)
}

#[repr(C)]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FreeTrialDetails {
    pub name: String,
    pub email: String,
    pub business_name: String,
    pub address1: String,
    pub address2: String,
    pub city: String,
    pub state: String,
    pub zip: String,
    pub country: String,
    pub phone: String,
    pub website: String,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CircuitThroughput {
    pub timestamp: u64,
    pub circuit_hash: i64,
    pub download_bytes: u64,
    pub upload_bytes: u64,
    pub packets_down: u64,
    pub packets_up: u64,
    pub tcp_packets_down: u64,
    pub tcp_packets_up: u64,
    pub udp_packets_down: u64,
    pub udp_packets_up: u64,
    pub icmp_packets_down: u64,
    pub icmp_packets_up: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct CircuitRetransmits {
    pub timestamp: u64,
    pub circuit_hash: i64,
    pub tcp_retransmits_down: u32,
    pub tcp_retransmits_up: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct CircuitRtt {
    pub timestamp: u64,
    pub circuit_hash: i64,
    pub median_rtt: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct CircuitCakeDrops {
    pub timestamp: u64,
    pub circuit_hash: i64,
    pub cake_drops_down: u32,
    pub cake_drops_up: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct CircuitCakeMarks {
    pub timestamp: u64,
    pub circuit_hash: i64,
    pub cake_marks_down: u32,
    pub cake_marks_up: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct SiteThroughput {
    pub timestamp: u64,
    pub site_hash: i64,
    pub download_bytes: u64,
    pub upload_bytes: u64,
    pub packets_down: u64,
    pub packets_up: u64,
    pub packets_tcp_down: u64,
    pub packets_tcp_up: u64,
    pub packets_udp_down: u64,
    pub packets_udp_up: u64,
    pub packets_icmp_down: u64,
    pub packets_icmp_up: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct SiteRetransmits {
    pub timestamp: u64,
    pub site_hash: i64,
    pub tcp_retransmits_down: u32,
    pub tcp_retransmits_up: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct SiteCakeDrops {
    pub timestamp: u64,
    pub site_hash: i64,
    pub cake_drops_down: u32,
    pub cake_drops_up: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct SiteCakeMarks {
    pub timestamp: u64,
    pub site_hash: i64,
    pub cake_marks_down: u32,
    pub cake_marks_up: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct SiteRtt {
    pub timestamp: u64,
    pub site_hash: i64,
    pub median_rtt: f32,
}

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq)]
#[repr(i32)]
pub enum LtsStatus {
    NotChecked = -1,
    AlwaysFree = 0,
    FreeTrial = 1,
    SelfHosted = 2,
    ApiOnly = 3,
    Full = 4,
    Invalid = 5,
}

impl LtsStatus {
    pub fn from_i32(value: i32) -> Self {
        match value {
            -1 => LtsStatus::NotChecked,
            1 => LtsStatus::AlwaysFree,
            2 => LtsStatus::FreeTrial,
            3 => LtsStatus::SelfHosted,
            4 => LtsStatus::ApiOnly,
            5 => LtsStatus::Full,
            _ => LtsStatus::Invalid,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct ShaperThroughput {
    pub tick: u64,
    pub bytes_per_second_down: i64,
    pub bytes_per_second_up: i64,
    pub shaped_bytes_per_second_down: i64,
    pub shaped_bytes_per_second_up: i64,
    pub packets_down: i64,
    pub packets_up: i64,
    pub tcp_packets_down: i64,
    pub tcp_packets_up: i64,
    pub udp_packets_down: i64,
    pub udp_packets_up: i64,
    pub icmp_packets_down: i64,
    pub icmp_packets_up: i64,
    pub max_rtt: Option<f32>,
    pub min_rtt: Option<f32>,
    pub median_rtt: Option<f32>,
    pub tcp_retransmits_down: i32,
    pub tcp_retransmits_up: i32,
    pub cake_marks_down: i32,
    pub cake_marks_up: i32,
    pub cake_drops_down: i32,
    pub cake_drops_up: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FlowCount {
    pub timestamp: u64,
    pub count: u64,
}

#[derive(Serialize, Deserialize)]
pub struct ShapedDevices {
    pub tick: u64,
    pub blob: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
pub struct NetworkTree {
    pub tick: u64,
    pub blob: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
pub struct OneWayFlow {
    pub start_time: u64,
    pub end_time: u64,
    pub local_ip: std::net::IpAddr,
    pub remote_ip: std::net::IpAddr,
    pub protocol: u8,
    pub dst_port: u16,
    pub src_port: u16,
    pub bytes: u64,
    pub circuit_hash: i64,
}

#[derive(Serialize, Deserialize)]
pub struct ShaperUtilization {
    pub tick: u64,
    pub average_cpu: f32,
    pub peak_cpu: f32,
    pub memory_percent: f32,
}

#[derive(Serialize, Deserialize)]
pub struct TwoWayFlow {
    pub start_time: u64,
    pub end_time: u64,
    pub local_ip: std::net::IpAddr,
    pub remote_ip: std::net::IpAddr,
    pub protocol: u8,
    pub dst_port: u16,
    pub src_port: u16,
    pub bytes_down: u64,
    pub bytes_up: u64,
    pub retransmit_times_down: Vec<i64>,
    pub retransmit_times_up: Vec<i64>,
    pub rtt: [f32; 2],
    pub circuit_hash: i64,
}