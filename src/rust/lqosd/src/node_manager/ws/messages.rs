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
    CircuitDirectoryPage, CircuitDirectoryQuery, NodeDirectoryEntry,
};
use crate::node_manager::local_api::ethernet_caps::{EthernetCapsPage, EthernetCapsPageQuery};
use crate::node_manager::local_api::executive::{
    ExecutiveDashboardSummary, ExecutiveHeatmapPage, ExecutiveHeatmapPageQuery,
    ExecutiveLeaderboardPage, ExecutiveLeaderboardPageQuery,
};
use crate::node_manager::local_api::local_api_keys::LocalApiKeyCreation;
use crate::node_manager::local_api::network_tree_lite::NetworkTreeLiteNode;
use crate::node_manager::local_api::node_rate_overrides::{
    NodeRateOverrideData, NodeRateOverrideQuery, NodeRateOverrideUpdate,
};
use crate::node_manager::local_api::node_topology_overrides::{
    NodeTopologyOverrideData, NodeTopologyOverrideQuery,
};
use crate::node_manager::local_api::packet_analysis::RequestAnalysisResult;
use crate::node_manager::local_api::scheduler::{SchedulerDetails, SchedulerStatus};
use crate::bridge_supervisor::BridgeHealthStatus;
use crate::bridge_broker::outcomes::PublishRecord;
use crate::bridge_broker::BridgeConvergenceSnapshot;
use lqos_overrides::CircuitSpeedOverride;
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
    flow_explorer::{FlowTimeline, FlowTimelineBucket},
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
    StormguardRuntimeStatus,
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
    StopCakeWatcher,
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
    /// E2: the bridge Supervisor's verdict + raw report detail
    /// (None when the DPDK bridge is not configured).
    BridgeHealthStatus,
    /// Read-only DPDK dataplane status (config + system sources, labelled by
    /// provenance). Never writes anything.
    DpdkStatus,
    /// E3b: recent bridge publish outcomes (pending k/n, applied,
    /// rejected + typed reason) — a read of the broker's own state.
    BridgePublishStatus,
    /// L4 §5: the operator convergence snapshot (state + the held op in
    /// operator language + escalation counters) for the Bridge page.
    BridgeConvergence,
    /// `lqos-8h8` (c): the maintenance-window operator OVERRIDE — apply the
    /// currently-held change NOW and/or arm a time-boxed global break-glass
    /// (Admin-only; audited). Writes the control file the broker consumes.
    BridgeMaintenanceOverride {
        apply_now: bool,
        break_glass_minutes: Option<u64>,
    },
    /// E3a (retarget): the operator circuit-speed override for one
    /// circuit (None = no override — the base plan applies).
    GetCircuitSpeedOverride {
        circuit_id: String,
    },
    /// E3a (retarget): upsert or clear the operator circuit-speed
    /// override. `speeds: None` = resume (delete); Admin-only.
    SetCircuitSpeedOverride {
        circuit_id: String,
        speeds: Option<CircuitSpeedOverride>,
    },
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
    CreateLocalApiKey {
        name: String,
    },
    RevokeLocalApiKey {
        id: String,
    },
    RemoveLegacyLocalApiKey,
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
    UrgentStatus {
        request_id: Option<u64>,
    },
    UrgentList {
        request_id: Option<u64>,
    },
    UrgentClear {
        id: u64,
        request_id: Option<u64>,
    },
    UrgentClearAll {
        request_id: Option<u64>,
    },
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
    /// `.64.2.2`: the offered/pre-shaping DEMAND (bridge mode — `bridge_actual_bytes_per_second`).
    /// In legacy mode `bps` already carries the demand, so this mirrors it.
    #[serde(default)]
    pub actual_bps: DownUpOrder<u64>,
    /// `.66.2`: the whole-box CLASS rates (DEMAND/enqueue). `mapped` = a circuit
    /// was resolved for the subscriber IP; `unmapped` = the `best_effort` slot;
    /// `mapped_bps + unmapped_bps == actual_bps`. Bridge mode only (zeroed in
    /// legacy mode).
    ///
    /// ADDITIVE: `ThroughputData` is `Serialize`-only, so the additive guarantee
    /// is a NEW CBOR map key that old readers ignore (the `.65 actual_bps`
    /// precedent) — NOT `serde(default)`.
    ///
    /// `.66.3` note: the pulse's "vs 24 h" chip uses the existing `LtsThroughput`
    /// path (`local_api/lts/last_24_hours.rs::throughput_period_data`), which is
    /// INSIGHT-LICENSE-GATED (`super::insight_gate().await?` → `FORBIDDEN` when
    /// unentitled, `ws.rs:1475`); the widget must degrade/hide the chip there.
    pub mapped_bps: DownUpOrder<u64>,
    pub unmapped_bps: DownUpOrder<u64>,
    /// `.117`: the backend-computed unmapped share of the DEMAND class
    /// counters (`unmapped / (mapped + unmapped) * 100`). The pulse displays
    /// this single value so the percentage cannot diverge from the class
    /// counters by summing a different pair in the UI.
    pub unmapped_pct: f32,
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
    BridgeHealthStatus {
        data: Option<BridgeHealthStatus>,
    },
    BridgePublishStatus {
        data: Vec<PublishRecord>,
    },
    BridgeMaintenanceOverride {
        ok: bool,
        message: String,
    },
    BridgeConvergence {
        data: BridgeConvergenceSnapshot,
    },
    GetCircuitSpeedOverrideResult {
        circuit_id: String,
        data: Option<CircuitSpeedOverride>,
    },
    SetCircuitSpeedOverrideResult {
        ok: bool,
        /// False when the stored override already equaled the request
        /// (a no-op dispatch): no file write, no new publish generation
        /// — the UI must settle immediately instead of awaiting an
        /// outcome that will never exist (the false-NO_ACK fix).
        changed: bool,
        message: String,
        suspended: bool,
        /// F-NA (docs/false_no_ack_outcome_card.md): the MINTED
        /// post-write generation (present only when changed=true and
        /// the mint succeeded). The UI polls the outcome for THIS
        /// exact generation — return-to-base (revert-to-plan) lands on
        /// a retained generation the old key-diff could never see.
        generation: Option<String>,
    },
    DeviceCount {
        data: DeviceCount,
    },
    GlobalWarnings {
        data: Vec<(WarningLevel, String)>,
    },
    UrgentStatus {
        data: UrgentStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<u64>,
    },
    UrgentList {
        data: UrgentList,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<u64>,
    },
    UrgentClearResult {
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<u64>,
    },
    UrgentClearAllResult {
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<u64>,
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
    CreateLocalApiKeyResult {
        ok: bool,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        key: Option<LocalApiKeyCreation>,
    },
    RevokeLocalApiKeyResult {
        ok: bool,
        message: String,
    },
    RemoveLegacyLocalApiKeyResult {
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
    DpdkStatusResult {
        data: crate::node_manager::local_api::dpdk_status::DpdkStatus,
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
        /// `(lat, lng, country, bytes_down, rtt_internet_nanos, bytes_up)`.
        data: Vec<(f64, f64, String, u64, f32, u64)>,
    },
    CircuitByIdResult {
        id: String,
        data: Option<CircuitByIdData>,
        ok: bool,
    },
    RequestAnalysisResult {
        data: RequestAnalysisResult,
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
        buckets: Vec<FlowTimelineBucket>,
    },
    CountryFlowTimeline {
        iso_code: String,
        data: Vec<FlowTimeline>,
        buckets: Vec<FlowTimelineBucket>,
    },
    ProtocolFlowTimeline {
        protocol: String,
        data: Vec<FlowTimeline>,
        buckets: Vec<FlowTimelineBucket>,
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
    StormguardRuntime {
        data: StormguardRuntimeStatus,
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
