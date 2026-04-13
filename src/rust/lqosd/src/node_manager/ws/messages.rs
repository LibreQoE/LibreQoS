use crate::lts2_sys::control_channel::{SupportTicket, SupportTicketSummary};
use crate::node_manager::WarningLevel;
use crate::node_manager::local_api::circuit::CircuitByIdData;
use crate::node_manager::local_api::circuit_activity::{
    CircuitFlowSankeyRow, CircuitSummaryData, CircuitTopAsnsData, CircuitTopAsnsQuery,
    CircuitTrafficFlowsPage, CircuitTrafficFlowsQuery,
};
use crate::node_manager::local_api::config::{ConfigSecretClearRequest, ConfigView};
use crate::node_manager::local_api::dashboard_themes::{DashletIdentity, ThemeEntry};
use crate::node_manager::local_api::device_counts::DeviceCount;
use crate::node_manager::local_api::directories::{
    CircuitDirectoryPage, CircuitDirectoryQuery, NodeDirectoryEntry, TreeGuardMetadataSummary,
};
use crate::node_manager::local_api::ethernet_caps::{EthernetCapsPage, EthernetCapsPageQuery};
use crate::node_manager::local_api::executive::{
    ExecutiveDashboardSummary, ExecutiveHeatmapPage, ExecutiveHeatmapPageQuery,
    ExecutiveLeaderboardPage, ExecutiveLeaderboardPageQuery,
};
use crate::node_manager::local_api::network_tree_lite::NetworkTreeLiteNode;
use crate::node_manager::local_api::node_rate_overrides::{
    NodeRateOverrideData, NodeRateOverrideQuery, NodeRateOverrideUpdate,
};
use crate::node_manager::local_api::node_topology_overrides::{
    NodeTopologyOverrideData, NodeTopologyOverrideQuery,
};
use crate::node_manager::local_api::packet_analysis::RequestAnalysisResult;
use crate::node_manager::local_api::scheduler::{SchedulerDetails, SchedulerStatus};
use crate::node_manager::local_api::search::SearchResult;
use crate::node_manager::local_api::shaped_devices_page::{
    ShapedDevicesPage, ShapedDevicesPageQuery,
};
use crate::node_manager::local_api::topology_manager::{
    TopologyManagerAttachmentRateOverrideClear, TopologyManagerAttachmentRateOverrideUpdate,
    TopologyManagerClear, TopologyManagerManualAttachmentGroupClear,
    TopologyManagerManualAttachmentGroupUpdate, TopologyManagerProbePolicyUpdate,
    TopologyManagerStateData, TopologyManagerUpdate,
};
use crate::node_manager::local_api::topology_probes::TopologyProbesStateData;
use crate::node_manager::local_api::tree_attached_circuits::{
    TreeAttachedCircuitsPage, TreeAttachedCircuitsQuery,
};
use crate::node_manager::local_api::unknown_ips::{ClearUnknownIpsResponse, UnknownIp};
use crate::node_manager::local_api::urgent::{UrgentList, UrgentStatus};
use crate::node_manager::local_api::{
    circuit_count::CircuitCount,
    circuit_live::{CircuitLiveMetrics, CircuitMetricsQuery},
    cpu_affinity::{
        CircuitBrief, CpuAffinityCircuitsPage, CpuAffinityRuntimeSnapshot, CpuAffinitySiteTreeNode,
        CpuAffinitySummaryEntry, PreviewWeightItem,
    },
    flow_explorer::FlowTimeline,
    lts::{
        AsnFlowSizeWeb, CakeData, FlowCountViewWeb, FullPacketData, LtsTrialConfig,
        PercentShapedWeb, RecentMedians, ShaperRttHistogramEntry, ShaperStatus,
        ThroughputData as LtsThroughputData, Top10Circuit, Worst10RttCircuit, Worst10RxmitCircuit,
    },
};
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::node_manager::ws::ticker::ipstats_conversion::IpStatsWithPlan;
use crate::throughput_tracker::TcpRetransmitTotal;
use crate::throughput_tracker::flow_data::{
    AsnCountryListEntry, AsnListEntry, AsnProtocolListEntry,
};
use lqos_bus::{
    Circuit, FlowbeeSummaryData, LtsCapabilitiesSummary, QueueStoreTransit, StormguardDebugEntry,
};
use lqos_config::QooProfileInfo;
use lqos_config::{Config, NetworkJsonTransport, ShapedDevice, WebUser};
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
    StopCircuitWatcher,
    StopPingMonitorWatch,
    CakeWatcher { circuit: String },
    Chatbot { browser_ts_ms: Option<f64> },
    ChatbotUserInput { text: String },
    WatchTreeAttachedCircuits { query: TreeAttachedCircuitsQuery },
    StopTreeAttachedCircuitsWatch,
    WatchCircuitMetrics { query: CircuitMetricsQuery },
    StopCircuitMetricsWatch,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum WsRequest {
    Subscribe {
        channel: PublishedChannels,
    },
    Unsubscribe {
        channel: PublishedChannels,
    },
    Private(PrivateRequest),
    HelloReply(WsHelloReply),
    DashletThemes,
    DashletSave {
        name: String,
        entries: Vec<DashletIdentity>,
    },
    DashletGet {
        name: String,
    },
    DashletDelete {
        name: String,
    },
    SchedulerStatus,
    SchedulerDetails,
    DeviceCount,
    DevicesAll,
    ShapedDevicesPage {
        query: ShapedDevicesPageQuery,
    },
    ExecutiveHeatmapPage {
        query: ExecutiveHeatmapPageQuery,
    },
    ExecutiveLeaderboardPage {
        query: ExecutiveLeaderboardPageQuery,
    },
    NetworkTree,
    NetworkTreeLite,
    FlowMap,
    GlobalWarnings,
    Search {
        term: String,
    },
    ReloadLibreQoS,
    LtsTrialConfig,
    CircuitCount,
    LtsStartSignup,
    LtsCapabilities,
    LtsRetryLicenseCheck,
    LtsSignUp {
        license_key: String,
    },
    LtsShaperStatus,
    LtsThroughput {
        seconds: i32,
    },
    LtsPackets {
        seconds: i32,
    },
    LtsPercentShaped {
        seconds: i32,
    },
    LtsFlows {
        seconds: i32,
    },
    LtsCake {
        seconds: i32,
    },
    LtsRttHisto {
        seconds: i32,
    },
    LtsTop10Downloaders {
        seconds: i32,
    },
    LtsWorst10Rtt {
        seconds: i32,
    },
    LtsWorst10Rxmit {
        seconds: i32,
    },
    LtsTopFlows {
        seconds: i32,
    },
    LtsRecentMedian,
    AdminCheck,
    GetConfig,
    QooProfiles,
    UpdateConfig {
        config: Config,
        #[serde(default)]
        clear_secrets: ConfigSecretClearRequest,
    },
    UpdateNetworkJsonOnly {
        network_json: Value,
    },
    UpdateNetworkAndDevices {
        network_json: Value,
        shaped_devices: Vec<ShapedDevice>,
    },
    GetNodeRateOverride {
        query: NodeRateOverrideQuery,
    },
    SetNodeRateOverride {
        update: NodeRateOverrideUpdate,
    },
    ClearNodeRateOverride {
        query: NodeRateOverrideQuery,
    },
    GetNodeTopologyOverride {
        query: NodeTopologyOverrideQuery,
    },
    GetTopologyManagerState,
    GetTopologyProbesState,
    SetTopologyManagerOverride {
        update: TopologyManagerUpdate,
    },
    ClearTopologyManagerOverride {
        clear: TopologyManagerClear,
    },
    SetTopologyManagerProbePolicy {
        update: TopologyManagerProbePolicyUpdate,
    },
    SetTopologyManagerAttachmentRateOverride {
        update: TopologyManagerAttachmentRateOverrideUpdate,
    },
    ClearTopologyManagerAttachmentRateOverride {
        clear: TopologyManagerAttachmentRateOverrideClear,
    },
    SetTopologyManagerManualAttachmentGroup {
        update: TopologyManagerManualAttachmentGroupUpdate,
    },
    ClearTopologyManagerManualAttachmentGroup {
        clear: TopologyManagerManualAttachmentGroupClear,
    },
    ListNics,
    NetworkJson,
    AllShapedDevices,
    GetShapedDevice {
        device_id: String,
    },
    CreateShapedDevice {
        device: ShapedDevice,
    },
    UpdateShapedDevice {
        original_device_id: String,
        device: ShapedDevice,
    },
    DeleteShapedDevice {
        device_id: String,
    },
    CircuitDirectoryPage {
        query: CircuitDirectoryQuery,
    },
    EthernetCapsPage {
        query: EthernetCapsPageQuery,
    },
    NodeDirectory,
    TreeGuardMetadataSummary,
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
    DeleteUser {
        username: String,
    },
    CircuitById {
        id: String,
    },
    CircuitDevices {
        circuit: String,
    },
    CircuitFlowSankey {
        circuit: String,
    },
    CircuitTopAsns {
        query: CircuitTopAsnsQuery,
    },
    CircuitTrafficFlowsPage {
        query: CircuitTrafficFlowsQuery,
    },
    SetCircuitRttExcluded {
        circuit_id: String,
        excluded: bool,
    },
    RequestAnalysis {
        ip: String,
    },
    CpuAffinitySummary,
    CpuAffinityRuntimeSnapshot,
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
    AsnFlowTimeline {
        asn: u32,
    },
    CountryFlowTimeline {
        iso_code: String,
    },
    ProtocolFlowTimeline {
        protocol: String,
    },
    UrgentStatus,
    UrgentList,
    UrgentClear {
        id: u64,
    },
    UrgentClearAll,
    UnknownIps,
    UnknownIpsClear,
    UnknownIpsCsv,
    SupportTicketList,
    SupportTicketGet {
        ticket_id: i64,
    },
    SupportTicketCreate {
        subject: String,
        priority: u8,
        body: String,
        commentor: String,
    },
    SupportTicketAddComment {
        ticket_id: i64,
        commentor: String,
        body: String,
    },
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BakeryCapacityInterfaceData {
    pub name: String,
    pub planned_qdiscs: usize,
    pub infra_qdiscs: usize,
    pub cake_qdiscs: usize,
    pub fq_codel_qdiscs: usize,
    pub estimated_memory_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BakeryLiveCapacityInterfaceData {
    pub name: String,
    pub live_qdiscs: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BakeryPreflightData {
    pub ok: bool,
    pub message: String,
    pub safe_budget: usize,
    pub hard_limit: usize,
    pub estimated_total_memory_bytes: u64,
    pub memory_available_bytes: Option<u64>,
    pub memory_guard_min_available_bytes: u64,
    pub memory_ok: bool,
    pub interfaces: Vec<BakeryCapacityInterfaceData>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BakeryStatusState {
    pub active_circuits: usize,
    pub mode: String,
    pub current_action_started_unix: Option<u64>,
    pub current_apply_phase: Option<String>,
    pub current_apply_total_tc_commands: usize,
    pub current_apply_completed_tc_commands: usize,
    pub current_apply_total_chunks: usize,
    pub current_apply_completed_chunks: usize,
    pub last_success_unix: Option<u64>,
    pub last_full_reload_success_unix: Option<u64>,
    pub last_failure_unix: Option<u64>,
    pub last_failure_summary: Option<String>,
    pub last_apply_type: String,
    pub last_total_tc_commands: usize,
    pub last_class_commands: usize,
    pub last_qdisc_commands: usize,
    pub last_build_duration_ms: u64,
    pub last_apply_duration_ms: u64,
    pub avg_tc_io_interval_ms: Option<u64>,
    pub last_tc_io_unix: Option<u64>,
    pub tc_io_interval_samples: usize,
    pub runtime_operations: BakeryRuntimeOperationsData,
    pub queue_distribution: Vec<BakeryQueueDistributionData>,
    pub live_capacity_interfaces: Vec<BakeryLiveCapacityInterfaceData>,
    pub live_capacity_safe_budget: usize,
    pub live_capacity_updated_at_unix: Option<u64>,
    pub preflight: Option<BakeryPreflightData>,
    pub reload_required: bool,
    pub reload_required_reason: Option<String>,
    pub dirty_subtree_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BakeryStatusData {
    pub current_state: BakeryStatusState,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BakeryQueueDistributionData {
    pub queue: u32,
    pub top_level_site_count: usize,
    pub site_count: usize,
    pub circuit_count: usize,
    pub download_mbps: u64,
    pub upload_mbps: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BakeryRuntimeOperationsData {
    pub submitted_count: usize,
    pub deferred_count: usize,
    pub applying_count: usize,
    pub awaiting_cleanup_count: usize,
    pub failed_count: usize,
    pub blocked_count: usize,
    pub dirty_count: usize,
    pub latest: Option<BakeryRuntimeOperationHeadlineData>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BakeryRuntimeOperationHeadlineData {
    pub operation_id: u64,
    pub site_hash: i64,
    pub site_name: Option<String>,
    pub action: String,
    pub status: String,
    pub attempt_count: u32,
    pub updated_at_unix: u64,
    pub next_retry_at_unix: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BakeryActivityEntry {
    pub ts: u64,
    pub event: String,
    pub status: String,
    pub summary: String,
    pub site_hash: Option<i64>,
    pub site_name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TreeguardStatusData {
    pub enabled: bool,
    pub dry_run: bool,
    pub paused_for_bakery_reload: bool,
    pub pause_reason: Option<String>,
    pub cpu_max_pct: Option<u8>,
    pub total_nodes: usize,
    pub total_circuits: usize,
    pub managed_nodes: usize,
    pub managed_circuits: usize,
    pub virtualized_nodes: usize,
    pub cake_circuits: usize,
    pub mixed_sqm_circuits: usize,
    pub fq_codel_circuits: usize,
    pub last_action_summary: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TreeguardActivityEntry {
    pub time: String,
    pub entity_type: String,
    pub entity_id: String,
    pub action: String,
    pub persisted: bool,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_kind: Option<String>,
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

#[derive(Debug, Serialize, Deserialize)]
pub struct CircuitDevicesResult {
    pub circuit_id: String,
    pub devices: Vec<Circuit>,
    pub ok: bool,
}

#[derive(Debug, Serialize)]
pub enum PingState {
    ChannelTest,
    NoResponse,
    Ping { time_nanos: u64, label: String },
}

#[derive(Serialize)]
#[serde(tag = "event")]
#[allow(clippy::large_enum_variant)]
pub enum WsResponse {
    #[serde(rename = "join")]
    Join {
        channel: PublishedChannels,
    },
    Hello {
        #[serde(flatten)]
        hello: WsHello,
    },
    Error {
        message: String,
    },
    DashletThemes {
        entries: Vec<ThemeEntry>,
    },
    DashletTheme {
        name: String,
        entries: Vec<DashletIdentity>,
    },
    DashletSaveResult {
        ok: bool,
        error: Option<String>,
    },
    DashletDeleteResult {
        ok: bool,
        error: Option<String>,
    },
    SchedulerStatus {
        data: SchedulerStatus,
    },
    SchedulerDetails {
        data: SchedulerDetails,
    },
    DeviceCount {
        data: DeviceCount,
    },
    GlobalWarnings {
        data: Vec<(WarningLevel, String)>,
    },
    UrgentStatus {
        data: UrgentStatus,
    },
    UrgentList {
        data: UrgentList,
    },
    UrgentClearResult {
        ok: bool,
    },
    UrgentClearAllResult {
        ok: bool,
    },
    UnknownIps {
        data: Vec<UnknownIp>,
    },
    UnknownIpsCleared {
        data: ClearUnknownIpsResponse,
    },
    UnknownIpsCsv {
        csv: String,
    },
    AdminCheck {
        ok: bool,
    },
    GetConfig {
        data: ConfigView,
    },
    QooProfiles {
        data: QooProfilesSummary,
    },
    ListNics {
        data: Vec<(String, String, String)>,
    },
    NetworkJson {
        data: Value,
    },
    AllShapedDevices {
        data: Vec<ShapedDevice>,
    },
    UpdateConfigResult {
        ok: bool,
        message: String,
    },
    UpdateNetworkJsonOnlyResult {
        ok: bool,
        message: String,
    },
    UpdateNetworkAndDevicesResult {
        ok: bool,
        message: String,
    },
    GetShapedDeviceResult {
        ok: bool,
        message: String,
        device: Option<ShapedDevice>,
    },
    CreateShapedDeviceResult {
        ok: bool,
        message: String,
        device: Option<ShapedDevice>,
    },
    UpdateShapedDeviceResult {
        ok: bool,
        message: String,
        device: Option<ShapedDevice>,
    },
    DeleteShapedDeviceResult {
        ok: bool,
        message: String,
        device_id: String,
    },
    CircuitDirectoryPage {
        data: CircuitDirectoryPage,
    },
    EthernetCapsPage {
        data: EthernetCapsPage,
    },
    NodeDirectory {
        data: Vec<NodeDirectoryEntry>,
    },
    TreeGuardMetadataSummary {
        data: TreeGuardMetadataSummary,
    },
    GetNodeRateOverride {
        data: NodeRateOverrideData,
    },
    SetNodeRateOverrideResult {
        ok: bool,
        message: String,
        data: NodeRateOverrideData,
    },
    ClearNodeRateOverrideResult {
        ok: bool,
        message: String,
        data: NodeRateOverrideData,
    },
    GetNodeTopologyOverride {
        data: NodeTopologyOverrideData,
    },
    GetTopologyManagerState {
        data: TopologyManagerStateData,
    },
    GetTopologyProbesState {
        data: TopologyProbesStateData,
    },
    SetTopologyManagerOverrideResult {
        ok: bool,
        message: String,
        data: TopologyManagerStateData,
    },
    ClearTopologyManagerOverrideResult {
        ok: bool,
        message: String,
        data: TopologyManagerStateData,
    },
    SetTopologyManagerProbePolicyResult {
        ok: bool,
        message: String,
        data: TopologyManagerStateData,
    },
    SetTopologyManagerAttachmentRateOverrideResult {
        ok: bool,
        message: String,
        data: TopologyManagerStateData,
    },
    ClearTopologyManagerAttachmentRateOverrideResult {
        ok: bool,
        message: String,
        data: TopologyManagerStateData,
    },
    SetTopologyManagerManualAttachmentGroupResult {
        ok: bool,
        message: String,
        data: TopologyManagerStateData,
    },
    ClearTopologyManagerManualAttachmentGroupResult {
        ok: bool,
        message: String,
        data: TopologyManagerStateData,
    },
    GetUsers {
        data: Vec<WebUser>,
    },
    AddUserResult {
        ok: bool,
        message: String,
    },
    UpdateUserResult {
        ok: bool,
        message: String,
    },
    DeleteUserResult {
        ok: bool,
        message: String,
    },
    LtsTrialConfigResult {
        data: LtsTrialConfig,
    },
    CircuitCountResult {
        data: CircuitCount,
    },
    LtsCapabilitiesResult {
        data: LtsCapabilitiesSummary,
    },
    LtsStartSignupResult {
        claim_id: String,
    },
    LtsSignUpResult {
        ok: bool,
        message: String,
    },
    LtsShaperStatus {
        data: Vec<ShaperStatus>,
    },
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
    LtsRecentMedian {
        data: Vec<RecentMedians>,
    },
    DevicesAll {
        data: Vec<ShapedDevice>,
    },
    ShapedDevicesPage {
        data: ShapedDevicesPage,
    },
    ExecutiveDashboardSummary {
        data: ExecutiveDashboardSummary,
    },
    ExecutiveHeatmapPage {
        data: ExecutiveHeatmapPage,
    },
    ExecutiveLeaderboardPage {
        data: ExecutiveLeaderboardPage,
    },
    FlowMap {
        data: Vec<(f64, f64, String, u64, f32)>,
    },
    CircuitByIdResult {
        id: String,
        data: Option<CircuitByIdData>,
        ok: bool,
    },
    RequestAnalysisResult {
        data: RequestAnalysisResult,
    },
    CpuAffinitySummary {
        data: Vec<CpuAffinitySummaryEntry>,
    },
    CpuAffinityRuntimeSnapshot {
        data: CpuAffinityRuntimeSnapshot,
    },
    CpuAffinityCircuits {
        data: CpuAffinityCircuitsPage,
    },
    CpuAffinityCircuitsAll {
        data: Vec<CircuitBrief>,
    },
    CpuAffinityPreviewWeights {
        data: Vec<PreviewWeightItem>,
    },
    CpuAffinitySiteTree {
        data: Option<CpuAffinitySiteTreeNode>,
    },
    AsnList {
        data: Vec<AsnListEntry>,
    },
    CountryList {
        data: Vec<AsnCountryListEntry>,
    },
    ProtocolList {
        data: Vec<AsnProtocolListEntry>,
    },
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
    ReloadResult {
        message: String,
    },
    Cadence,
    Throughput {
        data: ThroughputData,
    },
    RttHistogram {
        data: Vec<u32>,
    },
    FlowCount {
        active: u64,
        recent: u64,
    },
    TopDownloads {
        data: Vec<IpStatsWithPlan>,
    },
    TopUploads {
        data: Vec<IpStatsWithPlan>,
    },
    WorstRTT {
        data: Vec<IpStatsWithPlan>,
    },
    WorstRetransmits {
        data: Vec<IpStatsWithPlan>,
    },
    TopFlowsBytes {
        data: Vec<FlowbeeSummaryData>,
    },
    TopFlowsRate {
        data: Vec<FlowbeeSummaryData>,
    },
    AsnTopDownload {
        data: Vec<TopAsnRow>,
    },
    AsnTopUpload {
        data: Vec<TopAsnRow>,
    },
    EndpointsByCountry {
        data: Vec<(String, DownUpOrder<u64>, [f32; 2], String)>,
    },
    EtherProtocols {
        data: EtherProtocolsData,
    },
    IpProtocols {
        data: Vec<(String, DownUpOrder<u64>)>,
    },
    FlowDurations {
        data: Vec<(usize, u64)>,
    },
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
    NetworkTreeLite {
        data: Vec<(usize, NetworkTreeLiteNode)>,
    },
    NetworkTreeClients {
        data: Vec<Circuit>,
    },
    TreeAttachedCircuitsSnapshot {
        data: TreeAttachedCircuitsPage,
    },
    TreeAttachedCircuitsUpdate {
        data: TreeAttachedCircuitsPage,
    },
    CircuitMetricsSnapshot {
        data: Vec<CircuitLiveMetrics>,
    },
    CircuitMetricsUpdate {
        data: Vec<CircuitLiveMetrics>,
    },
    QueueStatsTotal {
        marks: DownUpOrder<u64>,
        drops: DownUpOrder<u64>,
    },
    CircuitCapacity {
        data: Vec<CircuitCapacityRow>,
    },
    TreeCapacity {
        data: Vec<NodeCapacity>,
    },
    Cpu {
        data: Vec<u32>,
    },
    Ram {
        data: RamData,
    },
    Retransmits {
        data: TcpRetransmitTotal,
    },
    StormguardStatus {
        data: Vec<(String, u64, u64)>,
    },
    StormguardDebug {
        data: Vec<StormguardDebugEntry>,
    },
    BakeryStatus {
        data: BakeryStatusData,
    },
    BakeryActivity {
        data: Vec<BakeryActivityEntry>,
    },
    TreeGuardStatus {
        data: TreeguardStatusData,
    },
    TreeGuardActivity {
        data: Vec<TreeguardActivityEntry>,
    },
    CircuitWatcher {
        data: CircuitSummaryData,
    },
    CircuitDevicesResult {
        data: CircuitDevicesResult,
    },
    SetCircuitRttExcludedResult {
        ok: bool,
        message: String,
        circuit_id: String,
        excluded: bool,
    },
    PingMonitor {
        ip: String,
        result: PingState,
    },
    CircuitFlowSankeyResult {
        circuit_id: String,
        flows: Vec<CircuitFlowSankeyRow>,
    },
    CircuitTopAsnsResult {
        circuit_id: String,
        data: CircuitTopAsnsData,
    },
    CircuitTrafficFlowsPageResult {
        circuit_id: String,
        data: CircuitTrafficFlowsPage,
    },
    CakeWatcher {
        #[serde(flatten)]
        data: QueueStoreTransit,
    },
    ChatbotChunk {
        text: String,
    },
    SupportTicketListResult {
        tickets: Vec<SupportTicketSummary>,
    },
    SupportTicketGetResult {
        ticket: Option<SupportTicket>,
    },
    SupportTicketCreateResult {
        ticket: SupportTicket,
    },
    SupportTicketAddCommentResult {
        ok: bool,
    },
}

pub fn encode_ws_message<T>(message: &T) -> Result<std::sync::Arc<Vec<u8>>, serde_cbor::Error>
where
    T: Serialize,
{
    let payload = serde_cbor::to_vec(message)?;
    Ok(std::sync::Arc::new(payload))
}
