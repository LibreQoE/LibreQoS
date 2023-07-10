use std::collections::HashMap;
use chrono::{DateTime, FixedOffset};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug)]
pub enum WasmRequest {
    Auth { token: String },
    Login { license: String, username: String, password: String },
    GetNodeStatus,
    PacketChart { period: String },
    PacketChartSingle { period: String, node_id: String, node_name: String },
    ThroughputChart { period: String },
    ThroughputChartSingle { period: String, node_id: String, node_name: String },
    ThroughputChartSite { period: String, site_id: String },
    ThroughputChartCircuit { period: String, circuit_id: String },
    RttChart { period: String },
    RttChartSingle { period: String, node_id: String, node_name: String },
    RttChartSite { period: String, site_id: String },
    RttChartCircuit { period: String, circuit_id: String },
    SiteStack { period: String, site_id: String },
    RootHeat { period: String },
    SiteHeat { period: String, site_id: String },
    NodePerfChart { period: String, node_id: String, node_name: String },
    Tree { parent: String },
    SiteInfo { site_id: String },
    SiteParents { site_id: String },
    CircuitParents { circuit_id: String },
    RootParents,
    Search { term: String },
    CircuitInfo { circuit_id: String },
    ExtendedDeviceInfo { circuit_id: String },
    SignalNoiseChartExt { period: String, device_id: String },
    DeviceCapacityChartExt { period: String, device_id: String },
}

#[derive(Serialize, Deserialize, Debug)]
pub enum WasmResponse {
    AuthOk { token: String, name: String, license_key: String },
    AuthFail,
    LoginOk { token: String, name: String, license_key: String },
    LoginFail,
    NodeStatus { nodes: Vec<Node> },
    PacketChart { nodes: Vec<PacketHost> },
    BitsChart { nodes: Vec<ThroughputHost> },
    RttChart { nodes: Vec<RttHost>, histogram: Vec<u32> },
    RttChartSite { nodes: Vec<RttHost>, histogram: Vec<u32> },
    RttChartCircuit { nodes: Vec<RttHost>, histogram: Vec<u32> },
    SiteStack { nodes: Vec<ThroughputHost> },
    RootHeat { data: HashMap<String, Vec<(DateTime<FixedOffset>, f64)>>},
    SiteHeat { data: HashMap<String, Vec<(DateTime<FixedOffset>, f64)>>},
    NodePerfChart { nodes: Vec<PerfHost> },
    SiteTree { data: Vec<SiteTree> },
    SiteInfo { data: SiteTree },
    SiteParents { data: Vec<(String, String)> },
    SiteChildren { data: Vec<(String, String, String)> },
    SearchResult { hits: Vec<SearchResult> },
    CircuitInfo { data: Vec<CircuitList> },
    DeviceExt { data: Vec<ExtendedDeviceInfo> },
    DeviceExtSnr { data: Vec<SignalNoiseChartExt>, device_id: String },
    DeviceExtCapacity { data: Vec<CapacityChartExt>, device_id: String },
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Node {
    pub node_id: String,
    pub node_name: String,
    pub last_seen: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PacketHost {
    pub node_id: String,
    pub node_name: String,
    pub down: Vec<Packets>,
    pub up: Vec<Packets>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Packets {
    pub value: f64,
    pub date: String,
    pub l: f64,
    pub u: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ThroughputHost {
    pub node_id: String,
    pub node_name: String,
    pub down: Vec<Throughput>,
    pub up: Vec<Throughput>,
}

impl ThroughputHost {
    pub fn total(&self) -> f64 {
        self.down.iter().map(|x| x.value).sum::<f64>() + self.up.iter().map(|x| x.value).sum::<f64>()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Throughput {
    pub value: f64,
    pub date: String,
    pub l: f64,
    pub u: f64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ThroughputChart {
    pub msg: String,
    pub nodes: Vec<ThroughputHost>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Rtt {
    pub value: f64,
    pub date: String,
    pub l: f64,
    pub u: f64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RttHost {
    pub node_id: String,
    pub node_name: String,
    pub rtt: Vec<Rtt>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PerfHost {
    pub node_id: String,
    pub node_name: String,
    pub stats: Vec<Perf>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Perf {
    pub date: String,
    pub cpu: f64,
    pub cpu_max: f64,
    pub ram: f64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SiteTree {
    pub index: i32,
    pub site_name: String,
    pub site_type: String,
    pub parent: i32,
    pub max_down: i32,
    pub max_up: i32,
    pub current_down: i32,
    pub current_up: i32,
    pub current_rtt: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SearchResult {
    pub name: String,
    pub url: String,
    pub score: f64,
    pub icon: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CircuitList {
    pub circuit_name: String,
    pub device_id: String,
    pub device_name: String,
    pub parent_node: String,
    pub mac: String,
    pub download_min_mbps: i32,
    pub download_max_mbps: i32,
    pub upload_min_mbps: i32,
    pub upload_max_mbps: i32,
    pub comment: String,
    pub ip_range: String,
    pub subnet: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExtendedDeviceInfo {
    pub device_id: String,
    pub name: String,
    pub model: String,
    pub firmware: String,
    pub status: String,
    pub mode: String,
    pub channel_width: i32,
    pub tx_power: i32,
    pub interfaces: Vec<ExtendedDeviceInterface>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExtendedDeviceInterface {
    pub name: String,
    pub mac: String,
    pub status: String,
    pub speed: String,
    pub ip_list: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SignalNoiseChartExt {
    pub date: String,
    pub signal: f64,
    pub noise: f64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CapacityChartExt {
    pub date: String,
    pub dl: f64,
    pub ul: f64,
}