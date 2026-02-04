use crate::node_manager::WarningLevel;
use crate::node_manager::local_api::dashboard_themes::{DashletIdentity, ThemeEntry};
use crate::node_manager::local_api::device_counts::DeviceCount;
use crate::node_manager::local_api::packet_analysis::RequestAnalysisResult;
use crate::node_manager::local_api::scheduler::{SchedulerDetails, SchedulerStatus};
use crate::node_manager::local_api::search::SearchResult;
use crate::node_manager::local_api::unknown_ips::{ClearUnknownIpsResponse, UnknownIp};
use crate::node_manager::local_api::urgent::{UrgentList, UrgentStatus};
use crate::node_manager::local_api::{
    circuit_count::CircuitCount,
    cpu_affinity::{
        CircuitBrief, CpuAffinityCircuitsPage, CpuAffinitySiteTreeNode, CpuAffinitySummaryEntry,
        PreviewWeightItem,
    },
    flow_explorer::FlowTimeline,
    lts::{
        AsnFlowSizeWeb, CakeData, FlowCountViewWeb, FullPacketData, LtsTrialConfig,
        PercentShapedWeb, RecentMedians, ShaperRttHistogramEntry, ShaperStatus,
        ThroughputData as LtsThroughputData, Top10Circuit, Worst10RttCircuit, Worst10RxmitCircuit,
    },
};
use crate::throughput_tracker::flow_data::{
    AsnCountryListEntry, AsnListEntry, AsnProtocolListEntry,
};
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::node_manager::ws::ticker::ipstats_conversion::IpStatsWithPlan;
use crate::throughput_tracker::TcpRetransmitTotal;
use crate::throughput_tracker::flow_data::{FlowAnalysis, FlowbeeLocalData};
use lqos_bus::{
    AsnHeatmapData, Circuit, CircuitHeatmapData, ExecutiveSummaryHeader, FlowbeeSummaryData,
    QueueStoreTransit, SiteHeatmapData, StormguardDebugEntry,
};
use lqos_config::{Config, NetworkJsonTransport, ShapedDevice, WebUser};
use lqos_config::QooProfileInfo;
use lqos_utils::temporal_heatmap::HeatmapBlocks;
use lqos_utils::qoq_heatmap::QoqHeatmapBlocks;
use lqos_utils::units::DownUpOrder;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const WS_HANDSHAKE_REQUIREMENT: &str =
    "I accept that this is an unstable, internal API and is unsupported";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WsHello {
    pub version: String,
    pub requirement: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WsHelloReply {
    pub ack: String,
    pub token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PrivateRequest {
    CircuitWatcher { circuit: String },
    PingMonitor { ips: Vec<(String, String)> },
    FlowsByCircuit { circuit: String },
    CakeWatcher { circuit: String },
    Chatbot { browser_ts_ms: Option<f64> },
    ChatbotUserInput { text: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum WsRequest {
    Subscribe { channel: PublishedChannels },
    Unsubscribe { channel: PublishedChannels },
    Private(PrivateRequest),
    HelloReply(WsHelloReply),
    DashletThemes,
    DashletSave { name: String, entries: Vec<DashletIdentity> },
    DashletGet { name: String },
    DashletDelete { name: String },
    SchedulerStatus,
    SchedulerDetails,
    DeviceCount,
    DevicesAll,
    NetworkTree,
    FlowMap,
    GlobalWarnings,
    Search { term: String },
    ReloadLibreQoS,
    LtsTrialConfig,
    CircuitCount,
    LtsSignUp { license_key: String },
    LtsShaperStatus,
    LtsThroughput { seconds: i32 },
    LtsPackets { seconds: i32 },
    LtsPercentShaped { seconds: i32 },
    LtsFlows { seconds: i32 },
    LtsCake { seconds: i32 },
    LtsRttHisto { seconds: i32 },
    LtsTop10Downloaders { seconds: i32 },
    LtsWorst10Rtt { seconds: i32 },
    LtsWorst10Rxmit { seconds: i32 },
    LtsTopFlows { seconds: i32 },
    LtsRecentMedian,
    AdminCheck,
    GetConfig,
    QooProfiles,
    UpdateConfig { config: Config },
    UpdateNetworkAndDevices {
        network_json: Value,
        shaped_devices: Vec<ShapedDevice>,
    },
    ListNics,
    NetworkJson,
    AllShapedDevices,
    GetUsers,
    AddUser {
        username: String,
        password: Option<String>,
        role: String,
    },
    UpdateUser {
        username: String,
        password: Option<String>,
        role: String,
    },
    DeleteUser { username: String },
    CircuitById { id: String },
    RequestAnalysis { ip: String },
    CpuAffinitySummary,
    CpuAffinityCircuits {
        cpu: u32,
        direction: Option<String>,
        page: Option<usize>,
        page_size: Option<usize>,
        search: Option<String>,
    },
    CpuAffinityCircuitsAll {
        direction: Option<String>,
        search: Option<String>,
    },
    CpuAffinityPreviewWeights {
        direction: Option<String>,
        search: Option<String>,
    },
    CpuAffinitySiteTree,
    AsnList,
    CountryList,
    ProtocolList,
    AsnFlowTimeline { asn: u32 },
    CountryFlowTimeline { iso_code: String },
    ProtocolFlowTimeline { protocol: String },
    UrgentStatus,
    UrgentList,
    UrgentClear { id: u64 },
    UrgentClearAll,
    UnknownIps,
    UnknownIpsClear,
    UnknownIpsCsv,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QooProfilesSummary {
    #[serde(default)]
    pub default_profile_id: Option<String>,
    pub profiles: Vec<QooProfileInfo>,
}

#[derive(Debug, Serialize)]
pub struct ThroughputData {
    pub bps: DownUpOrder<u64>,
    pub pps: DownUpOrder<u64>,
    pub tcp_pps: DownUpOrder<u64>,
    pub udp_pps: DownUpOrder<u64>,
    pub icmp_pps: DownUpOrder<u64>,
    pub shaped_bps: DownUpOrder<u64>,
    pub max: DownUpOrder<u64>,
}

#[derive(Debug, Serialize)]
pub struct EtherProtocolsData {
    pub v4_bytes: DownUpOrder<u64>,
    pub v6_bytes: DownUpOrder<u64>,
    pub v4_packets: DownUpOrder<u64>,
    pub v6_packets: DownUpOrder<u64>,
    pub v4_rtt: DownUpOrder<u64>,
    pub v6_rtt: DownUpOrder<u64>,
}

#[derive(Debug, Serialize)]
pub struct RamData {
    pub total: u64,
    pub used: u64,
}

#[derive(Debug, Serialize)]
pub struct BakeryStatusState {
    #[serde(rename = "activeCircuits")]
    pub active_circuits: usize,
}

#[derive(Debug, Serialize)]
pub struct BakeryStatusData {
    #[serde(rename = "currentState")]
    pub current_state: BakeryStatusState,
}

#[derive(Debug, Serialize)]
pub struct TopAsnRow {
    pub name: String,
    pub value: u64,
    pub flow_count: u64,
    pub retransmit_percent: f64,
}

#[derive(Debug, Serialize)]
pub struct CircuitCapacityRow {
    pub circuit_id: String,
    pub circuit_name: String,
    pub capacity: [f64; 2],
    pub median_rtt: f32,
}

#[derive(Debug, Serialize)]
pub struct NodeCapacity {
    pub id: usize,
    pub name: String,
    pub down: f64,
    pub up: f64,
    pub max_down: f64,
    pub max_up: f64,
    pub median_rtt: f32,
}

#[derive(Debug, Serialize)]
pub struct OversubscribedSite {
    pub site_name: String,
    pub cap_down: f32,
    pub cap_up: f32,
    pub sub_down: f32,
    pub sub_up: f32,
    pub ratio_down: Option<f32>,
    pub ratio_up: Option<f32>,
    pub ratio_max: Option<f32>,
}

#[derive(Debug, Serialize)]
pub struct ExecutiveHeatmapsData {
    pub header: ExecutiveSummaryHeader,
    pub global: HeatmapBlocks,
    pub global_qoq: QoqHeatmapBlocks,
    pub circuits: Vec<CircuitHeatmapData>,
    pub sites: Vec<SiteHeatmapData>,
    pub asns: Vec<AsnHeatmapData>,
    pub oversubscribed_sites: Vec<OversubscribedSite>,
}

#[derive(Debug, Serialize)]
pub struct FlowbeeKeyTransit {
    pub remote_ip: String,
    pub local_ip: String,
    pub src_port: u16,
    pub dst_port: u16,
    pub ip_protocol: u8,
    pub device_name: String,
    pub asn_name: String,
    pub asn_country: String,
    pub protocol_name: String,
    pub last_seen_nanos: u64,
}

#[derive(Debug, Serialize)]
pub enum PingState {
    ChannelTest,
    NoResponse,
    Ping { time_nanos: u64, label: String },
}

#[derive(Serialize)]
#[serde(tag = "event")]
pub enum WsResponse {
    #[serde(rename = "join")]
    Join { channel: PublishedChannels },
    Hello {
        #[serde(flatten)]
        hello: WsHello,
    },
    Error { message: String },
    DashletThemes { entries: Vec<ThemeEntry> },
    DashletTheme {
        name: String,
        entries: Vec<DashletIdentity>,
    },
    DashletSaveResult { ok: bool, error: Option<String> },
    DashletDeleteResult { ok: bool, error: Option<String> },
    SchedulerStatus { data: SchedulerStatus },
    SchedulerDetails { data: SchedulerDetails },
    DeviceCount { data: DeviceCount },
    GlobalWarnings {
        data: Vec<(WarningLevel, String)>,
    },
    UrgentStatus { data: UrgentStatus },
    UrgentList { data: UrgentList },
    UrgentClearResult { ok: bool },
    UrgentClearAllResult { ok: bool },
    UnknownIps { data: Vec<UnknownIp> },
    UnknownIpsCleared { data: ClearUnknownIpsResponse },
    UnknownIpsCsv { csv: String },
    AdminCheck { ok: bool },
    GetConfig { data: Config },
    QooProfiles { data: QooProfilesSummary },
    ListNics {
        data: Vec<(String, String, String)>,
    },
    NetworkJson { data: Value },
    AllShapedDevices { data: Vec<ShapedDevice> },
    UpdateConfigResult { ok: bool, message: String },
    UpdateNetworkAndDevicesResult { ok: bool, message: String },
    GetUsers { data: Vec<WebUser> },
    AddUserResult { ok: bool, message: String },
    UpdateUserResult { ok: bool, message: String },
    DeleteUserResult { ok: bool, message: String },
    LtsTrialConfigResult { data: LtsTrialConfig },
    CircuitCountResult { data: CircuitCount },
    LtsSignUpResult { ok: bool, message: String },
    LtsShaperStatus { data: Vec<ShaperStatus> },
    LtsThroughput {
        seconds: i32,
        data: Vec<LtsThroughputData>,
    },
    LtsPackets {
        seconds: i32,
        data: Vec<FullPacketData>,
    },
    LtsPercentShaped {
        seconds: i32,
        data: Vec<PercentShapedWeb>,
    },
    LtsFlows {
        seconds: i32,
        data: Vec<FlowCountViewWeb>,
    },
    LtsCake {
        seconds: i32,
        data: Vec<CakeData>,
    },
    LtsRttHisto {
        seconds: i32,
        data: Vec<ShaperRttHistogramEntry>,
    },
    LtsTop10Downloaders {
        seconds: i32,
        data: Vec<Top10Circuit>,
    },
    LtsWorst10Rtt {
        seconds: i32,
        data: Vec<Worst10RttCircuit>,
    },
    LtsWorst10Rxmit {
        seconds: i32,
        data: Vec<Worst10RxmitCircuit>,
    },
    LtsTopFlows {
        seconds: i32,
        data: Vec<AsnFlowSizeWeb>,
    },
    LtsRecentMedian { data: Vec<RecentMedians> },
    DevicesAll { data: Vec<ShapedDevice> },
    FlowMap {
        data: Vec<(f64, f64, String, u64, f32)>,
    },
    CircuitByIdResult {
        id: String,
        devices: Vec<ShapedDevice>,
        ok: bool,
    },
    RequestAnalysisResult { data: RequestAnalysisResult },
    CpuAffinitySummary {
        data: Vec<CpuAffinitySummaryEntry>,
    },
    CpuAffinityCircuits { data: CpuAffinityCircuitsPage },
    CpuAffinityCircuitsAll { data: Vec<CircuitBrief> },
    CpuAffinityPreviewWeights { data: Vec<PreviewWeightItem> },
    CpuAffinitySiteTree { data: Option<CpuAffinitySiteTreeNode> },
    AsnList { data: Vec<AsnListEntry> },
    CountryList { data: Vec<AsnCountryListEntry> },
    ProtocolList { data: Vec<AsnProtocolListEntry> },
    AsnFlowTimeline {
        asn: u32,
        data: Vec<FlowTimeline>,
    },
    CountryFlowTimeline {
        iso_code: String,
        data: Vec<FlowTimeline>,
    },
    ProtocolFlowTimeline {
        protocol: String,
        data: Vec<FlowTimeline>,
    },
    SearchResults {
        term: String,
        results: Vec<SearchResult>,
    },
    ReloadResult { message: String },
    Cadence,
    Throughput { data: ThroughputData },
    RttHistogram { data: Vec<u32> },
    FlowCount { active: u64, recent: u64 },
    TopDownloads { data: Vec<IpStatsWithPlan> },
    TopUploads { data: Vec<IpStatsWithPlan> },
    WorstRTT { data: Vec<IpStatsWithPlan> },
    WorstRetransmits { data: Vec<IpStatsWithPlan> },
    TopFlowsBytes { data: Vec<FlowbeeSummaryData> },
    TopFlowsRate { data: Vec<FlowbeeSummaryData> },
    AsnTopDownload { data: Vec<TopAsnRow> },
    AsnTopUpload { data: Vec<TopAsnRow> },
    EndpointsByCountry {
        data: Vec<(String, DownUpOrder<u64>, [f32; 2], String)>,
    },
    EtherProtocols { data: EtherProtocolsData },
    IpProtocols { data: Vec<(String, DownUpOrder<u64>)> },
    FlowDurations { data: Vec<(usize, u64)> },
    EndpointLatLon {
        data: Vec<(f64, f64, String, u64, f32)>,
    },
    TreeSummary {
        data: Vec<(usize, NetworkJsonTransport)>,
    },
    TreeSummaryL2 {
        data: Vec<(usize, Vec<(usize, NetworkJsonTransport)>)>,
    },
    NetworkTree {
        data: Vec<(usize, NetworkJsonTransport)>,
    },
    NetworkTreeClients { data: Vec<Circuit> },
    QueueStatsTotal {
        marks: DownUpOrder<u64>,
        drops: DownUpOrder<u64>,
    },
    CircuitCapacity { data: Vec<CircuitCapacityRow> },
    TreeCapacity { data: Vec<NodeCapacity> },
    Cpu { data: Vec<u32> },
    Ram { data: RamData },
    Retransmits { data: TcpRetransmitTotal },
    StormguardStatus { data: Vec<(String, u64, u64)> },
    StormguardDebug { data: Vec<StormguardDebugEntry> },
    BakeryStatus { data: BakeryStatusData },
    ExecutiveHeatmaps { data: ExecutiveHeatmapsData },
    CircuitWatcher {
        circuit_id: String,
        devices: Vec<Circuit>,
        qoo_score: Option<f32>,
    },
    PingMonitor { ip: String, result: PingState },
    FlowsByCircuit {
        circuit_id: String,
        flows: Vec<(FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysis)>,
    },
    CakeWatcher {
        #[serde(flatten)]
        data: QueueStoreTransit,
    },
    ChatbotChunk { text: String },
}

pub fn encode_ws_message<T>(message: &T) -> Result<std::sync::Arc<Vec<u8>>, serde_cbor::Error>
where
    T: Serialize,
{
    let payload = serde_cbor::to_vec(message)?;
    Ok(std::sync::Arc::new(payload))
}
