// SPDX-FileCopyrightText: 2025 LibreQoE support@libreqos.io
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception

use allocative::Allocative;
use lqos_config::Tunables;
use serde::{Deserialize, Serialize};

/// Source system for an urgent issue
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Copy, Allocative)]
pub enum UrgentSource {
    /// Raised by the scheduler process
    Scheduler,
    /// Raised by the LibreQoS Python orchestrator
    LibreQoS,
    /// Raised by the local API server
    API,
    /// Raised by lqosd or other components
    System,
}

/// Severity of an urgent issue
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Copy, Allocative)]
pub enum UrgentSeverity {
    /// Error requires attention
    Error,
    /// Warning is informative/high-visibility
    Warning,
}

/// Scheduler progress metadata for startup or long-running refresh phases.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Allocative)]
pub struct SchedulerProgressReport {
    /// Whether the scheduler is actively working through a known phase sequence.
    pub active: bool,
    /// Stable machine-readable phase key.
    pub phase: String,
    /// Human-readable phase label for UI display.
    pub phase_label: String,
    /// Current 1-based step index within the reported phase sequence.
    pub step_index: u32,
    /// Total number of steps in the reported phase sequence.
    pub step_count: u32,
    /// Coarse completion percentage in the range `0..=100`.
    pub percent: u8,
    /// Unix timestamp when the progress state was last updated, if known.
    pub updated_unix: Option<u64>,
}

/// Cause class for the last integration import failure.
///
/// Classified at emission in `scheduler.py` (the layer holding the raw output)
/// and forwarded verbatim by lqosd — no text archaeology at display time.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Allocative)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationErrorClass {
    /// The endpoint returned a redirect loop (e.g. UISP `too many redirects`).
    RedirectLoop,
    /// Authentication/authorization failure (HTTP 401/403).
    Auth,
    /// Host unreachable, timeout, or DNS failure.
    Unreachable,
    /// The subprocess exited non-zero with no recognized pattern.
    ExitedUnknown,
    /// No integration failure recorded.
    None,
}

/// One bounded recent integration run for the System Health timeline.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Allocative)]
pub struct SchedulerRun {
    /// Unix timestamp of the run.
    pub at: u64,
    /// Whether the run succeeded.
    pub ok: bool,
    /// Duration in milliseconds, if known.
    pub ms: Option<u64>,
}

/// Scheduler integration-sync status emitted by `scheduler.py`.
///
/// Carries the INTEGRATION health axis explicitly so the scheduler-error axis
/// can mean loop/runtime errors only. The scheduler is the sole producer.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Allocative)]
pub struct SchedulerIntegrationStatus {
    /// The integration label that last ran (e.g. `UISP integration`).
    pub integration_name: Option<String>,
    /// Whether the last import completed successfully.
    pub last_import_ok: Option<bool>,
    /// Unix timestamp of the last import, if known.
    pub last_import_at: Option<u64>,
    /// Duration of the last import in milliseconds, if known.
    pub last_import_ms: Option<u64>,
    /// Consecutive failures since the last success (0 = healthy).
    pub consecutive_failures: u32,
    /// The classified cause of the last failure.
    pub error_class: IntegrationErrorClass,
    /// Stripped, actionable detail line(s) from the last failure.
    pub error_detail: Option<String>,
    /// The bounded recent-run ring (oldest first) for the timeline.
    pub recent_runs: Vec<SchedulerRun>,
}

/// One or more `BusRequest` objects must be included in a `BusSession`
/// request. Each `BusRequest` represents a single request for action
/// or data.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Allocative)]
pub enum BusRequest {
    /// A generic "is it alive?" test. Returns an `Ack`.
    Ping,

    /// Request total current throughput. Returns a
    /// `BusResponse::CurrentThroughput` value.
    GetCurrentThroughput,

    /// Retrieve the top N downloads by bandwidth use.
    GetTopNDownloaders {
        /// First row to retrieve (usually 0 unless you are paging)
        start: u32,
        /// Last row to retrieve (10 for top-10 starting at 0)
        end: u32,
    },

    /// Retrieve the top N uploads by bandwidth use.
    GetTopNUploaders {
        /// First row to retrieve (usually 0 unless you are paging)
        start: u32,
        /// Last row to retrieve (10 for top-10 starting at 0)
        end: u32,
    },

    /// Retrieve per-circuit TemporalHeatmap blocks.
    GetCircuitHeatmaps,

    /// Retrieve per-site TemporalHeatmap blocks.
    GetSiteHeatmaps,

    /// Retrieve per-ASN TemporalHeatmap blocks.
    GetAsnHeatmaps,

    /// Retrieve the global (roll-up) TemporalHeatmap.
    GetGlobalHeatmap,

    /// Retrieve headline metrics for the Executive Summary tab.
    GetExecutiveSummaryHeader,

    /// Retrieves the TopN hosts with the worst RTT, sorted by RTT descending.
    GetWorstRtt {
        /// First row to retrieve (usually 0 unless you are paging)
        start: u32,
        /// Last row to retrieve (10 for top-10 starting at 0)
        end: u32,
    },

    /// Retrieves the TopN hosts with the worst Retransmits, sorted by Retransmits descending.
    GetWorstRetransmits {
        /// First row to retrieve (usually 0 unless you are paging)
        start: u32,
        /// Last row to retrieve (10 for top-10 starting at 0)
        end: u32,
    },

    /// Retrieves the TopN hosts with the best RTT, sorted by RTT descending.
    GetBestRtt {
        /// First row to retrieve (usually 0 unless you are paging)
        start: u32,
        /// Last row to retrieve (10 for top-10 starting at 0)
        end: u32,
    },

    /// Retrieves current byte counters for all hosts.
    GetHostCounter,

    /// Simulate the previous version's `xdp_pping` command, returning
    /// RTT data for all mapped flows by TC handle.
    XdpPping,

    /// Divide current RTT data into histograms and return the data for
    /// rendering.
    RttHistogram,

    /// Cound the number of mapped and unmapped hosts detected by the
    /// system.
    HostCounts,

    /// Retrieve a list of all unmapped IPs that have been detected
    /// carrying traffic.
    AllUnknownIps,

    /// Reload the `LibreQoS.py` program and return details of the
    /// reload run.
    ReloadLibreQoS,

    /// Retrieve raw queue data for a given circuit ID.
    GetRawQueueData(String), // The string is the circuit ID

    /// Requests a real-time adjustment of the `lqosd` tuning settings
    UpdateLqosDTuning(u64, Tunables),

    /// Requests that the configuration be updated
    UpdateLqosdConfig(Box<lqos_config::Config>),

    /// Create or update a runtime-only dynamic circuit overlay entry.
    ///
    /// This does **not** mutate `ShapedDevices.csv`. The daemon may ignore this request when
    /// dynamic circuits are disabled in the configuration.
    CreateDynamicCircuit {
        /// Shaped-device-like definition for the dynamic circuit/device.
        shaped_device: Box<lqos_config::ShapedDevice>,
    },

    /// Remove a runtime-only dynamic circuit overlay entry.
    ///
    /// The daemon may ignore this request when dynamic circuits are disabled in the configuration.
    RemoveDynamicCircuit {
        /// Stable circuit identifier to remove.
        circuit_id: String,
    },

    /// Invalidate cached Node Manager authentication state.
    InvalidateAuthCache,

    /// Request that we start watching a circuit's queue
    WatchQueue(String),

    /// Request that the Rust side of things validate the CSV
    ValidateShapedDevicesCsv,

    /// Request details of part of the network tree
    GetNetworkMap {
        /// The parent of the map to retrieve
        parent: usize,
    },

    /// Request a batch of active probe observations from the shared probe
    /// provider owned by `lqosd`.
    ProbeBatch {
        /// Probe requests to execute or satisfy from cache.
        requests: Vec<lqos_probe::ProbeRequest>,
        /// Maximum acceptable observation age in milliseconds.
        max_age_ms: u64,
    },

    /// K1b §4.6 pin 3: look up the bridge publish lifecycle for one minted
    /// generation in the bridge broker's outcome ring. `None` when the
    /// generation is not (or no longer) present.
    BridgePublishStatus {
        /// The minted generation to look up.
        generation: String,
    },

    /// Request the full network tree
    GetFullNetworkMap,

    /// Retrieves the top N queues from the root level, and summarizes
    /// the others as "other"
    TopMapQueues(usize),

    /// Retrieve node names from network.json
    GetNodeNamesFromIds(Vec<usize>),

    /// Get all circuits and usage statistics
    GetAllCircuits,

    /// Get circuit usage statistics for a single circuit ID
    GetCircuitById {
        /// Circuit ID to query
        circuit_id: String,
    },

    /// Get live usage rollups aggregated by circuit ID.
    GetAllCircuitRollups,

    /// Get one live usage rollup aggregated by circuit ID.
    GetCircuitRollupById {
        /// Circuit ID to query.
        circuit_id: String,
    },

    /// Retrieve stats for all queues above a named circuit id
    GetFunnel {
        /// Circuit being analyzed, as the named circuit id
        target: String,
    },

    /// Obtain the lqosd statistics
    GetLqosStats,

    /// Tell Heimdall to hyper-focus on an IP address for a bit
    GatherPacketData(String),

    /// Give me a dump of the last 10 seconds of packet headers
    GetPacketHeaderDump(usize),

    /// Give me a libpcap format packet dump (shortened) of the last 10 seconds
    GetPcapDump(usize),

    /// If running on Equinix (the `equinix_test` feature is enabled),
    /// display a "run bandwidht test" link.
    #[cfg(feature = "equinix_tests")]
    RequestLqosEquinixTest,

    /// Request a dump of all active flows. This can be a lot of data.
    /// so this is intended for debugging
    DumpActiveFlows,

    /// Count the nubmer of active flows.
    CountActiveFlows,

    /// Top Flows Reports
    TopFlows {
        /// The type of top report to request
        flow_type: TopFlowType,
        /// The number of flows to return
        n: u32,
    },

    /// Flows by IP Address
    FlowsByIp(String),

    /// Current Endpoints by Country
    CurrentEndpointsByCountry,

    /// Lat/Lon of Endpoints
    CurrentEndpointLatLon,

    /// Duration of flows
    FlowDuration,

    /// Ether Protocol Summary
    EtherProtocolSummary,

    /// IP Protocol Summary
    IpProtocolSummary,

    /// Submit a piece of information to the blackboard
    BlackboardData {
        /// The subsystem to which the data applies
        subsystem: BlackboardSystem,
        /// The key for the data
        key: String,
        /// The value for the data
        value: String,
    },

    /// Submit binary data to the blackboard
    BlackboardBlob {
        /// The subsystem to which the data applies
        tag: String,
        /// The part of the data being submitted
        part: usize,
        /// The binary data
        blob: Vec<u8>,
    },

    /// Finish a blackboard session
    BlackboardFinish,

    /// Get current Stormguard statistics
    GetStormguardStats,

    /// Get current Stormguard debug snapshot
    GetStormguardDebug,

    /// Get current StormGuard lifecycle and dependency health.
    GetStormguardRuntimeStatus,

    /// Announce that the API is ready
    ApiReady,

    /// Announce that the chatbot is ready
    ChatbotReady,

    /// Announce that the scheduler is ready
    SchedulerReady,

    /// Announce a scheduler error
    SchedulerError(String),

    /// Announce informational scheduler output
    SchedulerOutput(String),

    /// AVAIL 0(c) (CARD `v3_availability_hardening_card` §2): read the
    /// broker's reconciliation convergence record (read-only; the broker is
    /// the sole writer). The Python scheduler consumes this to drive re-mint.
    GetConvergenceState,

    /// Announce scheduler startup or refresh progress metadata.
    SchedulerProgress(SchedulerProgressReport),

    /// Announce the integration-sync status metadata (the integration axis).
    SchedulerIntegrationStatus(SchedulerIntegrationStatus),

    /// Write an informational message to the lqosd logs
    LogInfo(String),

    /// Check the scheduler status
    CheckSchedulerStatus,

    /// Submit an urgent issue for high-priority operator visibility
    SubmitUrgentIssue {
        /// Source of the issue
        source: UrgentSource,
        /// Severity of the issue
        severity: UrgentSeverity,
        /// Machine-readable code for the issue (e.g. TC_U16_OVERFLOW)
        code: String,
        /// Human-readable message for display
        message: String,
        /// Optional JSON context payload
        context: Option<String>,
        /// Optional key to deduplicate repeated submissions
        dedupe_key: Option<String>,
    },

    /// Retrieve current urgent issues
    GetUrgentIssues,

    /// Clear a specific urgent issue by ID
    ClearUrgentIssue(u64),

    /// Clear urgent issues matching a code and dedupe key
    ClearUrgentIssueByIdentity {
        /// Machine-readable issue code
        code: String,
        /// Dedupe key used when the issue was submitted
        dedupe_key: String,
    },

    /// Clear all urgent issues
    ClearAllUrgentIssues,

    /// Retrieve device counts (shaped + unknown)
    GetDeviceCounts,

    /// Retrieve circuit counts (active + configured)
    GetCircuitCount,

    /// Retrieve flow map points (lat/lon endpoints)
    GetFlowMap,

    /// Retrieve list of ASNs with recent flow data
    GetAsnList,

    /// Retrieve list of countries with recent flow data
    GetCountryList,

    /// Retrieve list of protocols with recent flow data
    GetProtocolList,

    /// Retrieve flow timeline entries for an ASN
    GetAsnFlowTimeline {
        /// ASN number to filter
        asn: u32,
    },

    /// Retrieve flow timeline entries for a country
    GetCountryFlowTimeline {
        /// Country ISO code to filter
        iso_code: String,
    },

    /// Retrieve flow timeline entries for a protocol
    GetProtocolFlowTimeline {
        /// Protocol name to filter
        protocol: String,
    },

    /// Retrieve scheduler details (diagnostics)
    GetSchedulerDetails,

    /// Retrieve queue marks/drops totals
    GetQueueStatsTotal,

    /// Retrieve current top-level QoO history.
    GetQoo,

    /// Retrieve current QoO history for a site by site name.
    GetSiteQoo {
        /// Site name to query.
        site_name: String,
    },

    /// Retrieve current QoO history for a circuit by circuit ID.
    GetCircuitQoo {
        /// Circuit ID to query.
        circuit_id: String,
    },

    /// Retrieve per-circuit capacity utilization
    GetCircuitCapacity,

    /// Retrieve per-node capacity utilization
    GetTreeCapacity,

    /// Retrieve aggregate TCP retransmit summary
    GetRetransmitSummary,

    /// Retrieve two-level tree summary
    GetTreeSummaryL2,

    /// Search circuits/devices/sites by term
    Search {
        /// Search term
        term: String,
    },

    /// Retrieve current global warning list
    GetGlobalWarnings,

    /// Retrieve the current local LTS/license capability summary.
    GetLtsCapabilities,

    /// Retrieve current Insight license summary (licensed + optional max circuits).
    GetInsightLicenseSummary,

    /// Update configuration while preserving authoritative API credentials.
    ///
    /// This variant is appended to preserve existing bincode discriminants.
    UpdateLqosdConfigPreserveApiCredentials(Box<lqos_config::Config>),

    /// Read the read-only CPU & Workers snapshot assembled by the daemon
    /// (bridge worker affinity, applied ownership/crossings, bridge placement
    /// state, and moves availability). `lqos-74t.118.2`; the reply is a
    /// versioned JSON document, so the bus crate stays free of broker types.
    GetCpuWorkers,
}

impl BusRequest {
    /// Returns the stable request-type label used in bus diagnostics.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Ping => "Ping",
            Self::GetCurrentThroughput => "GetCurrentThroughput",
            Self::GetTopNDownloaders { .. } => "GetTopNDownloaders",
            Self::GetTopNUploaders { .. } => "GetTopNUploaders",
            Self::GetCircuitHeatmaps => "GetCircuitHeatmaps",
            Self::GetSiteHeatmaps => "GetSiteHeatmaps",
            Self::GetAsnHeatmaps => "GetAsnHeatmaps",
            Self::GetGlobalHeatmap => "GetGlobalHeatmap",
            Self::GetExecutiveSummaryHeader => "GetExecutiveSummaryHeader",
            Self::GetWorstRtt { .. } => "GetWorstRtt",
            Self::GetWorstRetransmits { .. } => "GetWorstRetransmits",
            Self::GetBestRtt { .. } => "GetBestRtt",
            Self::GetHostCounter => "GetHostCounter",
            Self::XdpPping => "XdpPping",
            Self::RttHistogram => "RttHistogram",
            Self::HostCounts => "HostCounts",
            Self::AllUnknownIps => "AllUnknownIps",
            Self::ReloadLibreQoS => "ReloadLibreQoS",
            Self::GetRawQueueData(_) => "GetRawQueueData",
            Self::UpdateLqosDTuning(_, _) => "UpdateLqosDTuning",
            Self::UpdateLqosdConfig(_) => "UpdateLqosdConfig",
            Self::CreateDynamicCircuit { .. } => "CreateDynamicCircuit",
            Self::RemoveDynamicCircuit { .. } => "RemoveDynamicCircuit",
            Self::InvalidateAuthCache => "InvalidateAuthCache",
            Self::WatchQueue(_) => "WatchQueue",
            Self::ValidateShapedDevicesCsv => "ValidateShapedDevicesCsv",
            Self::GetNetworkMap { .. } => "GetNetworkMap",
            Self::ProbeBatch { .. } => "ProbeBatch",
            Self::BridgePublishStatus { .. } => "BridgePublishStatus",
            Self::GetFullNetworkMap => "GetFullNetworkMap",
            Self::TopMapQueues(_) => "TopMapQueues",
            Self::GetNodeNamesFromIds(_) => "GetNodeNamesFromIds",
            Self::GetAllCircuits => "GetAllCircuits",
            Self::GetCircuitById { .. } => "GetCircuitById",
            Self::GetAllCircuitRollups => "GetAllCircuitRollups",
            Self::GetCircuitRollupById { .. } => "GetCircuitRollupById",
            Self::GetFunnel { .. } => "GetFunnel",
            Self::GetLqosStats => "GetLqosStats",
            Self::GatherPacketData(_) => "GatherPacketData",
            Self::GetPacketHeaderDump(_) => "GetPacketHeaderDump",
            Self::GetPcapDump(_) => "GetPcapDump",
            #[cfg(feature = "equinix_tests")]
            Self::RequestLqosEquinixTest => "RequestLqosEquinixTest",
            Self::DumpActiveFlows => "DumpActiveFlows",
            Self::CountActiveFlows => "CountActiveFlows",
            Self::TopFlows { .. } => "TopFlows",
            Self::FlowsByIp(_) => "FlowsByIp",
            Self::CurrentEndpointsByCountry => "CurrentEndpointsByCountry",
            Self::CurrentEndpointLatLon => "CurrentEndpointLatLon",
            Self::FlowDuration => "FlowDuration",
            Self::EtherProtocolSummary => "EtherProtocolSummary",
            Self::IpProtocolSummary => "IpProtocolSummary",
            Self::BlackboardData { .. } => "BlackboardData",
            Self::BlackboardBlob { .. } => "BlackboardBlob",
            Self::BlackboardFinish => "BlackboardFinish",
            Self::GetStormguardStats => "GetStormguardStats",
            Self::GetStormguardDebug => "GetStormguardDebug",
            Self::GetStormguardRuntimeStatus => "GetStormguardRuntimeStatus",
            Self::ApiReady => "ApiReady",
            Self::ChatbotReady => "ChatbotReady",
            Self::SchedulerReady => "SchedulerReady",
            Self::SchedulerError(_) => "SchedulerError",
            Self::SchedulerOutput(_) => "SchedulerOutput",
            Self::GetConvergenceState => "GetConvergenceState",
            Self::SchedulerProgress(_) => "SchedulerProgress",
            Self::SchedulerIntegrationStatus(_) => "SchedulerIntegrationStatus",
            Self::LogInfo(_) => "LogInfo",
            Self::CheckSchedulerStatus => "CheckSchedulerStatus",
            Self::SubmitUrgentIssue { .. } => "SubmitUrgentIssue",
            Self::GetUrgentIssues => "GetUrgentIssues",
            Self::ClearUrgentIssue(_) => "ClearUrgentIssue",
            Self::ClearUrgentIssueByIdentity { .. } => "ClearUrgentIssueByIdentity",
            Self::ClearAllUrgentIssues => "ClearAllUrgentIssues",
            Self::GetDeviceCounts => "GetDeviceCounts",
            Self::GetCircuitCount => "GetCircuitCount",
            Self::GetFlowMap => "GetFlowMap",
            Self::GetAsnList => "GetAsnList",
            Self::GetCountryList => "GetCountryList",
            Self::GetProtocolList => "GetProtocolList",
            Self::GetAsnFlowTimeline { .. } => "GetAsnFlowTimeline",
            Self::GetCountryFlowTimeline { .. } => "GetCountryFlowTimeline",
            Self::GetProtocolFlowTimeline { .. } => "GetProtocolFlowTimeline",
            Self::GetSchedulerDetails => "GetSchedulerDetails",
            Self::GetQueueStatsTotal => "GetQueueStatsTotal",
            Self::GetCircuitCapacity => "GetCircuitCapacity",
            Self::GetTreeCapacity => "GetTreeCapacity",
            Self::GetRetransmitSummary => "GetRetransmitSummary",
            Self::GetTreeSummaryL2 => "GetTreeSummaryL2",
            Self::Search { .. } => "Search",
            Self::GetGlobalWarnings => "GetGlobalWarnings",
            Self::GetQoo => "GetQoo",
            Self::GetSiteQoo { .. } => "GetSiteQoo",
            Self::GetCircuitQoo { .. } => "GetCircuitQoo",
            Self::GetLtsCapabilities => "GetLtsCapabilities",
            Self::GetInsightLicenseSummary => "GetInsightLicenseSummary",
            Self::UpdateLqosdConfigPreserveApiCredentials(_) => {
                "UpdateLqosdConfigPreserveApiCredentials"
            }
            Self::GetCpuWorkers => "GetCpuWorkers",
        }
    }

    /// Returns true when a timed-out request can fail fast without hiding daemon-side mutation.
    pub fn can_fail_fast_on_timeout(&self) -> bool {
        matches!(
            self,
            Self::Ping
                | Self::GetCurrentThroughput
                | Self::GetTopNDownloaders { .. }
                | Self::GetTopNUploaders { .. }
                | Self::GetCircuitHeatmaps
                | Self::GetSiteHeatmaps
                | Self::GetAsnHeatmaps
                | Self::GetGlobalHeatmap
                | Self::GetExecutiveSummaryHeader
                | Self::GetWorstRtt { .. }
                | Self::GetWorstRetransmits { .. }
                | Self::GetBestRtt { .. }
                | Self::GetHostCounter
                | Self::XdpPping
                | Self::RttHistogram
                | Self::HostCounts
                | Self::AllUnknownIps
                | Self::GetRawQueueData(_)
                | Self::ValidateShapedDevicesCsv
                | Self::GetNetworkMap { .. }
                | Self::GetFullNetworkMap
                | Self::TopMapQueues(_)
                | Self::GetNodeNamesFromIds(_)
                | Self::GetAllCircuits
                | Self::GetCircuitById { .. }
                | Self::GetFunnel { .. }
                | Self::GetLqosStats
                | Self::GetPacketHeaderDump(_)
                | Self::GetPcapDump(_)
                | Self::DumpActiveFlows
                | Self::CountActiveFlows
                | Self::TopFlows { .. }
                | Self::FlowsByIp(_)
                | Self::CurrentEndpointsByCountry
                | Self::CurrentEndpointLatLon
                | Self::FlowDuration
                | Self::EtherProtocolSummary
                | Self::IpProtocolSummary
                | Self::GetStormguardStats
                | Self::GetStormguardDebug
                | Self::GetStormguardRuntimeStatus
                | Self::CheckSchedulerStatus
                | Self::GetUrgentIssues
                | Self::GetDeviceCounts
                | Self::GetCircuitCount
                | Self::GetFlowMap
                | Self::GetAsnList
                | Self::GetCountryList
                | Self::GetProtocolList
                | Self::GetAsnFlowTimeline { .. }
                | Self::GetCountryFlowTimeline { .. }
                | Self::GetProtocolFlowTimeline { .. }
                | Self::GetSchedulerDetails
                | Self::GetQueueStatsTotal
                | Self::GetCircuitCapacity
                | Self::GetTreeCapacity
                | Self::GetRetransmitSummary
                | Self::GetTreeSummaryL2
                | Self::Search { .. }
                | Self::GetGlobalWarnings
                | Self::GetLtsCapabilities
                | Self::GetInsightLicenseSummary
                | Self::GetCpuWorkers
        )
    }
}

/// Defines the parts of the blackboard
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Copy, Allocative)]
pub enum BlackboardSystem {
    /// The system as a whole
    System,
    /// A specific site
    Site,
    /// A specific circuit
    Circuit,
    /// A specific device
    Device,
}

/// Defines the type of "top" flow being requested
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Copy, Allocative)]
pub enum TopFlowType {
    /// Top flows by current estimated bandwidth use
    RateEstimate,
    /// Top flows by total bytes transferred
    Bytes,
    /// Top flows by total packets transferred
    Packets,
    /// Top flows by total drops
    Drops,
    /// Top flows by round-trip time estimate
    RoundTripTime,
}

#[cfg(test)]
mod tests {
    use super::{BusRequest, TopFlowType};

    #[test]
    fn request_kind_omits_payload_details() {
        assert_eq!(BusRequest::Ping.kind(), "Ping");
        assert_eq!(
            BusRequest::GetNetworkMap { parent: 42 }.kind(),
            "GetNetworkMap"
        );
        assert_eq!(
            BusRequest::TopFlows {
                flow_type: TopFlowType::RateEstimate,
                n: 10,
            }
            .kind(),
            "TopFlows"
        );
    }
}
