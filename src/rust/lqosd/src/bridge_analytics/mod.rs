//! Bridge analytics polling actor (E2, `docs` card
//! `lqosd_broker_emission_card.md` §5 + §5.1).
//!
//! Owns the lqosd-side cache of bridge-sourced per-circuit live stats:
//! a named-thread actor polls the bridge's analytics unix socket
//! (`analytics_ipc::connect_snapshot`) at a product-configurable 1 s
//! default and translates cumulative host counters into rates
//! (`translate` — differencing, counter-reset handling, first-poll
//! seed).
//!
//! FENCES (card §5.1 corrections 2 + 4):
//! - This actor owns its OWN cache. It NEVER writes the shared
//!   `CIRCUIT_LIVE_SNAPSHOT` — the kernel lazy-rebuild path and its
//!   invalidation hooks stay untouched for the three remaining
//!   consumers (tree_attached_circuits, the dynamic-page sort,
//!   TreeGuard). The repoint is ENDPOINT-level only
//!   (`local_api::circuit_live`).
//! - The stale flag rides INSIDE the Arc-swapped snapshot (never a
//!   separate atomic — no torn (data, flag) pairs).
//! - On socket absence the cache serves LAST-GOOD + stale — never zeros
//!   (the G-D2 honesty posture: control-plane loss must not fabricate
//!   empty counters).
//!
//! Activation: the `[dataplane_bridge]` config section (or the
//! `LIBREQOS_ANALYTICS_SOCKET` environment variable — the bench rig
//! pattern) opts in; kernel-dataplane installations are untouched and
//! the endpoint keeps its kernel source.

pub mod translate;

/// Cluster-2: the flow-history ring + recent_flows retention. Increment 1
/// (mechanism + oracles); the actor/endpoint wiring is increment 2.
pub mod flow_history;

/// 3a cluster-1: the circuit/site temporal-heatmap accumulator. Increment 1
/// (mechanism + oracles, declared-not-wired); the poll fold + the consumer
/// repoints are increment 2.
pub mod circuit_heatmap;

/// 3a group-1: the bridge QoO/QoQ port pin). Increment 1
/// (mechanism + oracles, declared-not-wired); the poll fold + the consumer
/// repoints are increment 2.
pub mod circuit_qoo;
/// Carve-out sweep #1: the LTS2 (Insight) per-flow export consumer.
pub(crate) mod lts2_flow_export;
/// The flow-completion feed consumer (`lqos-74t.2.x`): a dedicated drain
/// thread over the bridge's completion ring + the durable `(epoch, seq)` cursor.
pub(crate) mod completion_consumer;

use crate::node_manager::local_api::network_tree_lite::NetworkTreeLiteNode;
use crate::system_stats::SystemStats;
use std::collections::VecDeque;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use analytics_ipc::{DEFAULT_MAX_ROWS, QdiscDetailRow, connect_queue_stats, connect_snapshot};
use arc_swap::ArcSwap;
use fxhash::{FxHashMap, FxHashSet};
use lqos_bus::{
    CakeTransit, QueueSampleDirection, QueueSampleSnapshot, QueueSampleTin, QueueStoreTransit,
};
use lqos_queue_tracker::{
    ALL_QUEUE_SUMMARY, QueueCounts, TOTAL_QUEUE_STATS, set_queue_stats_stale,
};
use lqos_utils::XdpIpAddress;
use lqos_utils::rtt::{FlowbeeEffectiveDirection, RttBuffer, RttData};
use lqos_utils::units::DownUpOrder;
use once_cell::sync::Lazy;
use tracing::{debug, info, warn};

use translate::{CircuitLiveRow, HostKey, difference, join_by_circuit};

/// Full population per card §5.1.6: the bridge sorts by traffic and then
/// truncates to the request limit — a display-sized limit would make
/// IDLE circuits vanish (fabricated absence). `DEFAULT_MAX_ROWS`
/// (10 000) is the bridge's maximum and carries the whole table.
pub(crate) const FULL_POPULATION_LIMIT: usize = DEFAULT_MAX_ROWS;

/// Polling floor: counters are cumulative; sub-250 ms polling only adds
/// socket churn without rate resolution.
const MIN_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How long a circuit's last-good rate survives a missing/reset poll. Long
/// enough to bridge a transient counter reset or a dropped host row, short
/// enough that a genuinely gone circuit stops being reported.
const LAST_GOOD_WINDOW_MS: u64 = 30_000;

/// The actor's own bridge-sourced cache. `stale` rides inside the swap.
#[derive(Clone, Debug, Default)]
pub struct BridgeStatsSnapshot {
    pub by_circuit_id: FxHashMap<String, CircuitLiveRow>,
    pub stale: bool,
    pub captured_at_millis: u64,
    /// Bridge-internal numeric ids seen this poll (diagnostics).
    pub host_rows: usize,
    /// Per-IP interval rates (T2a device-level join + flow counts).
    pub by_ip: FxHashMap<IpAddr, IpLiveRow>,
    /// `.49.1`: the SEPARATE best-effort (unmapped, `circuit_id == 0`) host
    /// map — sibling of `by_ip`. Fed by the wire `best_effort_hosts` rows
    /// (their OWN bridge budget), NEVER the shaped `by_ip` (which stays
    /// byte-identical by contract). Only the unknown surface reads it.
    pub best_effort_by_ip: FxHashMap<IpAddr, IpLiveRow>,
    /// Subscriber IPs with at least one ACTIVE flow this poll (T2a:
    /// per-circuit active-flow counts without a flow-row export).
    pub active_flow_ips: FxHashSet<IpAddr>,
    /// K2 slice 1: the whole-bridge top-level interval rates (actual +
    /// shaped), differenced per poll. The bridge's own totals are not
    /// truncated by the host-row limit — the sparkline reads these two
    /// series directly. Zero until a second poll establishes a window.
    pub bridge_actual_bytes_per_second: DownUpOrder<u64>,
    pub bridge_shaped_bytes_per_second: DownUpOrder<u64>,
    /// `.66.2`: the whole-box CLASS rates (DEMAND/enqueue), differenced from the
    /// cumulative `snapshot.mapped` / `snapshot.unmapped` counters (`.66.1`).
    /// `mapped` = a circuit was resolved for the subscriber IP; `unmapped` = the
    /// `CircuitId::best_effort` slot; `mapped + unmapped == actual` (NOT
    /// `shaped`). Bytes/second; readers convert with `to_bits_from_bytes`. Zero
    /// until a second poll establishes a window (and zero in legacy mode).
    pub bridge_mapped_bytes_per_second: DownUpOrder<u64>,
    pub bridge_unmapped_bytes_per_second: DownUpOrder<u64>,
    /// Circuits whose rate in this snapshot is carried last-good because a
    /// counter reset or a missing host row suppressed the interval delta.
    /// They are deliberately NOT dropped/zeroed; the UI can mark them stale.
    pub last_good_circuit_ids: FxHashSet<String>,
    /// K2 slice 1: the bridge's ACTIVE flow rows for this poll — the
    /// TopFlows source (5-tuple, bytes, RTT samples, TCP counters, state).
    pub active_flows: Vec<analytics_ipc::FlowRow>,
    /// 3a netflow: the bridge's disclosed untracked-flow packet counter (the
    /// export stats block echoes it so the two counters can be compared).
    pub flow_untracked_packets: u64,
    /// `.65`: the DIRECTIONAL per-flow interval bytes/second (down/up
    /// separately), differenced from the DELIVERED (shaped) per-flow counters
    /// across polls, keyed by FlowId. Absent for a flow's first interval or
    /// after a counter reset (no-rate; a reader falls back to the lifetime
    /// average). The Shaper Top ASN widgets AND the circuit page Top-ASN/Flows
    /// tabs read this to reconstruct a bits/s rate (×8) per direction for the
    /// bridge's active-flow rows. (This was the `.65` directional map; the
    /// legacy COMBINED `flow_bytes_per_second` map it superseded — its only
    /// reader was the circuit page — is retired with the U=0 fix.)
    pub flow_bytes_per_second_directional: FxHashMap<analytics_ipc::FlowId, DownUpOrder<u64>>,
 /// 3b §1.1 ruling c): per-protocol SENT packets/second,
    /// differenced per flow by the same seed pattern. `GetCurrentThroughput`
    /// reads these so the tcp/udp/icmp splits are true rates. CLASSIFIED-only:
    /// non-6/17/1 transports are counted in the all-protocol pps fold but not
    /// bucketed here, so their sum is a lower bound on that fold.
    pub protocol_packets_per_second: ProtocolPacketRates,
}

/// Per-IP live rates for one poll interval (T2a).
#[derive(Clone, Copy, Debug, Default)]
pub struct IpLiveRow {
    pub bytes_per_second: DownUpOrder<u64>,
    pub shaped_bytes_per_second: DownUpOrder<u64>,
    /// K2 slice 2 §7: per-direction ENQUEUED packets per second (the
    /// observed-traffic analog; lqtop's Down/Up pps columns).
    pub packets_per_second: DownUpOrder<u64>,
    /// K2 slice 2 §7: per-direction SENT (dequeued) packets per second.
    /// 3b: the `GetCurrentThroughput` pps + protocol splits read this series
    /// so their conservation identity holds (`sent_pkts` is a sent counter).
    pub packets_sent_per_second: DownUpOrder<u64>,
    pub tcp_packets: u64,
    pub tcp_retransmits: u64,
    /// K2 slice 1: per-IP RTT p50 (nanos) from the host rows' raw samples.
    pub rtt_current_p50_nanos: Option<u64>,
    /// `.49.1`: age since the host was last seen (nanos; `u64::MAX` =
    /// never-seen), via the SAME `translate::age_nanos` math. The unknown-IPs
    /// reader uses it for Last Seen + the 300 s window backstop.
    pub last_seen_nanos: u64,
    /// `.49.1`: cumulative per-direction BYTES for the row (the wire
    /// `HostRow.actual` totals). The unmapped class has no flow deltas, so the
    /// rate fields stay 0; the unknown-IPs reader shows these TOTALS
    /// (acceptance: "unknown IPs (with last-seen/bytes)"). 0 for shaped rows
    /// (their byte view is the rate series).
    pub total_bytes: DownUpOrder<u64>,
}

/// T2b: the per-tick tree rollup — E2 circuit rows summed through the
/// store's parents chains (self-inclusive below root), keyed by node
/// index. Read-joined by the tree consumers under `is_active()`; the
/// store's own `current_*` stays kernel-exclusive. `stale`/last-good
/// propagates per tick; no state is held between ticks beyond this
/// published snapshot (the CIRCUIT_RTT_BUFFERS precedent).
#[derive(Clone, Debug, Default)]
pub struct TreeLiveView {
    pub current_throughput: FxHashMap<usize, DownUpOrder<u64>>,
    /// K2 slice 1: per-node RTT p50 (ms) rolled up from the circuits' p50s.
    pub rtts: FxHashMap<usize, Vec<f32>>,
    /// K2 slice 2 729b: per-node pooled RTT histogram, bucketed locally from
    /// the circuits' host samples with the bridge wire's `latency_bucket`.
    pub rtt_histograms: FxHashMap<usize, analytics_ipc::RttHistogram>,
    /// K2 slice 2 729b: per-node passive RTT p90 (ms) computed from
    /// `rtt_histograms` — StormGuard's bridged passive source.
    pub rtt_p90_ms: FxHashMap<usize, f32>,
    /// K2 slice 1: per-node count of circuits with an RTT sample (down, up).
    pub current_rtt_flows: FxHashMap<usize, (u32, u32)>,
    /// K2 slice 1: per-node retransmit rollup + the paired TCP packet counts.
    pub current_retransmits: FxHashMap<usize, (u64, u64)>,
    pub current_tcp_retransmit_packets: FxHashMap<usize, (u64, u64)>,
    /// Circuit ids that matched no store node this build (rate-limited
    /// WARN at the builder; the T1 divergence assert covers the inverse).
    pub unmatched_circuits: usize,
    pub stale: bool,
    pub captured_at_millis: u64,
}

static TREE_LIVE_VIEW: Lazy<ArcSwap<Option<Arc<TreeLiveView>>>> =
    Lazy::new(|| ArcSwap::new(Arc::new(None)));

/// The latest tree live-view, if one has ever been built.
pub fn tree_live_view() -> Option<Arc<TreeLiveView>> {
    (*TREE_LIVE_VIEW.load_full()).clone()
}

/// K2 slice 2 729b: the bridge's per-node passive RTT p90 (ms), keyed by node
/// NAME (StormGuard's site key). `None` when the actor has no live view
/// (kernel path) or the last poll went stale — the caller then keeps the
/// per-node `rtts` p50-based p90 (the honest fallback).
pub fn bridge_passive_rtt_p90_by_name() -> Option<std::collections::HashMap<String, f32>> {
    let view = tree_live_view()?;
    if view.stale {
        return None;
    }
    let mut by_name = std::collections::HashMap::new();
    lqos_network_devices::with_network_json_read(|net_json| {
        let nodes = net_json.get_nodes_when_ready();
        for (idx, p90) in &view.rtt_p90_ms {
            if let Some(node) = nodes.get(*idx) {
                by_name.insert(node.name.clone(), *p90);
            }
        }
    });
    Some(by_name)
}

/// T2b + K2 slice 1 read-join: overlays a transport's live fields
/// (`current_throughput`, `rtts`, `current_rtt_flows`, `current_retransmits`,
/// `current_tcp_retransmit_packets`) from the tree live-view when the E2
/// actor is active. LIVE FIELDS ONLY — names, types, parents and every other
/// field are untouched; the store's own fields stay kernel-exclusive (no
/// write-back). Inactive actor or no view ever built ⇒ identity (the legacy
/// kernel path, v2 unchanged).
pub fn overlay_live_fields(idx: usize, transport: &mut lqos_config::NetworkJsonTransport) {
    if !is_active() {
        return;
    }
    let Some(view) = tree_live_view() else {
        return;
    };
    apply_live_fields(&view, idx, transport);
}

/// Cluster 5 (kernel_gating_audit_card §2.5): the lite tree payload's live
/// overlay. Same source/fields as [`overlay_live_fields`]. `current_tcp_packets`
/// is kernel-exclusive (honest-empty on bridge boxes); `qoo` is **no longer**
/// honest-empty — since `lqos-74t.47.1.4` it is filled from `circuit_qoo` in
/// `node_to_transport_lite` (this overlay still leaves it untouched).
pub fn overlay_live_fields_lite(idx: usize, node: &mut NetworkTreeLiteNode) {
    if !is_active() {
        return;
    }
    let Some(view) = tree_live_view() else {
        return;
    };
    apply_live_fields_lite(&view, idx, node);
}

/// Pure overlay body for a `NetworkJsonTransport`, split from the
/// actor-active gate so the mapping is unit-testable.
fn apply_live_fields(
    view: &TreeLiveView,
    idx: usize,
    transport: &mut lqos_config::NetworkJsonTransport,
) {
    if let Some(value) = view.current_throughput.get(&idx) {
        transport.current_throughput = (value.down, value.up);
    }
    // F3-B A1 (`docs/executive_rtt_rollup_card.md` §4c): do NOT replace
    // `transport.rtts` with the bridge per-IP p50 concat — the tree RTT now comes
    // from the restored tracker→`node.rtt_buffer` accumulation (the circuit-view
    // source). The E2 overlay keeps throughput/flows/retransmits/histograms.
    if let Some(&(down, up)) = view.current_rtt_flows.get(&idx) {
        transport.current_rtt_flows = (down, up);
    }
    if let Some(&(down, up)) = view.current_retransmits.get(&idx) {
        transport.current_retransmits = (down, up);
    }
    if let Some(&(down, up)) = view.current_tcp_retransmit_packets.get(&idx) {
        transport.current_tcp_retransmit_packets = (down, up);
    }
}

/// Pure overlay body for a lite tree node (the lite node carries no
/// `current_rtt_flows` field).
fn apply_live_fields_lite(view: &TreeLiveView, idx: usize, node: &mut NetworkTreeLiteNode) {
    if let Some(value) = view.current_throughput.get(&idx) {
        node.current_throughput = (value.down, value.up);
    }
    // F3-B A1: no `rtts` clobber (see `apply_live_fields`).
    if let Some(&(down, up)) = view.current_retransmits.get(&idx) {
        node.current_retransmits = (down, up);
    }
    if let Some(&(down, up)) = view.current_tcp_retransmit_packets.get(&idx) {
        node.current_tcp_retransmit_packets = (down, up);
    }
}

pub(crate) static CACHE: Lazy<ArcSwap<BridgeStatsSnapshot>> =
    Lazy::new(|| ArcSwap::new(Arc::new(BridgeStatsSnapshot::default())));
static STARTED: OnceLock<()> = OnceLock::new();
/// The analytics socket the actor pinned at start; QueueStats callers
/// (cake_watcher) connect to the SAME socket the snapshot poll uses.
static ANALYTICS_SOCKET: OnceLock<PathBuf> = OnceLock::new();

/// Test-only bridge-active override. The raw-queue tests need the active path
/// without setting the process-global `STARTED` OnceLock (which cannot be
/// reset and leaked into unrelated tests, making the raw-path test take the
/// empty bridge branch). Thread-local so parallel tests stay isolated.
/// Compiled out of production builds.
#[cfg(test)]
thread_local! {
    static TEST_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the E2 stats actor is active (endpoint source selection).
pub fn is_active() -> bool {
    #[cfg(test)]
    if TEST_ACTIVE.with(std::cell::Cell::get) {
        return true;
    }
    STARTED.get().is_some()
}

/// Test-only: set the bridge-active override on this test thread; returns the
/// previous value so a guard can restore it.
#[cfg(test)]
fn set_test_active(active: bool) -> bool {
    TEST_ACTIVE.with(|cell| {
        let previous = cell.get();
        cell.set(active);
        previous
    })
}

/// Current bridge-sourced snapshot (empty + stale before the first
/// successful poll).
pub fn snapshot() -> arc_swap::Guard<Arc<BridgeStatsSnapshot>> {
    CACHE.load()
}

/// 3b §1.1: the ALL-PROTOCOL SENT packets/second summed across the per-IP rows
/// (every transport, including `PacketTransportProtocol::Unknown`).
/// `GetCurrentThroughput.packets_per_second` reads this series; the tcp/udp/icmp
/// splits share its host source but bucket only the classified transports, so
/// their sum is ≤ this fold (equal iff no Unknown-transport traffic). PURE.
pub(crate) fn total_sent_packets_per_second(
    by_ip: &FxHashMap<IpAddr, IpLiveRow>,
) -> DownUpOrder<u64> {
    by_ip.values().fold(DownUpOrder::zeroed(), |mut acc, row| {
        acc.down = acc.down.saturating_add(row.packets_sent_per_second.down);
        acc.up = acc.up.saturating_add(row.packets_sent_per_second.up);
        acc
    })
}

/// 3b: the all-protocol ENQUEUED packets/second summed across the per-IP rows
/// (the batch payload's packet series). PURE.
pub(crate) fn total_enqueued_packets_per_second(
    by_ip: &FxHashMap<IpAddr, IpLiveRow>,
) -> DownUpOrder<u64> {
    by_ip.values().fold(DownUpOrder::zeroed(), |mut acc, row| {
        acc.down = acc.down.saturating_add(row.packets_per_second.down);
        acc.up = acc.up.saturating_add(row.packets_per_second.up);
        acc
    })
}

/// 3b: the live streaming-shaper rates from the bridge snapshot —
/// the fourth K3-orphan's bridge source. The headline is the DELIVERED (shaped)
/// bits/s (display semantics); packets are the enqueued fold (the batch's
/// companion). PURE.
pub(crate) fn live_shaper_rates(
    snapshot: &BridgeStatsSnapshot,
) -> (DownUpOrder<u64>, DownUpOrder<u64>, DownUpOrder<u64>) {
    let delivered = snapshot.bridge_shaped_bytes_per_second.to_bits_from_bytes();
    (
        delivered,
        delivered,
        total_enqueued_packets_per_second(&snapshot.by_ip),
    )
}

/// 3b: one circuit's per-device live rates from the actor's per-IP
/// rows, keyed by device hash. `bytes` is the DELIVERED (shaped) series (ruling
/// 2); `tcp_packets`/`tcp_retries` use the bridge down-only convention; `rtts`
/// are the per-IP RTT p50 samples (ms). The fifth orphan-class repoint (the
/// gateway `circuit_live` responder). PURE over the snapshot + catalog.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CircuitDeviceRates {
    pub bytes: DownUpOrder<u64>,
    pub tcp_packets: DownUpOrder<u64>,
    pub tcp_retries: DownUpOrder<u64>,
    pub rtts: Vec<f32>,
}

pub(crate) fn circuit_device_delivered_rates(
    by_ip: &FxHashMap<IpAddr, IpLiveRow>,
    catalog: &lqos_network_devices::NetworkDevicesCatalog,
    circuit_hash: i64,
) -> FxHashMap<i64, CircuitDeviceRates> {
    let mut out: FxHashMap<i64, CircuitDeviceRates> = FxHashMap::default();
    for (ip, row) in by_ip {
        let Some((_, device)) =
            catalog.device_longest_match_for_ip(&lqos_utils::XdpIpAddress::from_ip(*ip))
        else {
            continue;
        };
        if device.circuit_hash != circuit_hash {
            continue;
        }
        let entry = out.entry(device.device_hash).or_default();
        entry.bytes += row.shaped_bytes_per_second;
        entry.tcp_packets += DownUpOrder::new(row.tcp_packets, 0);
        entry.tcp_retries += DownUpOrder::new(row.tcp_retransmits, 0);
        if let Some(nanos) = row.rtt_current_p50_nanos {
            entry.rtts.push(nanos as f32 / 1_000_000.0);
        }
    }
    out
}

/// 3b §1.1 ruling c): per-protocol SENT packets/second, differenced
/// per flow with the same FlowId-keyed seed pattern as the byte rates. Only the
/// three CLASSIFIED transports are bucketed: TCP (6), UDP (17), ICMP (1). Every
/// other transport (e.g. ICMPv6, GRE, ESP, SCTP) is dropped here and reaches the
/// all-protocol host/`GetCurrentThroughput` fold only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProtocolPacketRates {
    pub tcp: DownUpOrder<u64>,
    pub udp: DownUpOrder<u64>,
    pub icmp: DownUpOrder<u64>,
}

impl ProtocolPacketRates {
    /// Adds one flow's interval packet delta (already per-second) to its
    /// protocol bucket.
    fn add(&mut self, protocol: u8, down: u64, up: u64) {
        let target = match protocol {
            6 => &mut self.tcp,
            17 => &mut self.udp,
            1 => &mut self.icmp,
            _ => return,
        };
        target.down = target.down.saturating_add(down);
        target.up = target.up.saturating_add(up);
    }

    /// The sum of the three CLASSIFIED (TCP/UDP/ICMP) buckets. This equals the
    /// all-protocol sent pps fold ONLY when no Unknown-transport traffic exists
    /// (the host fold counts every transport; the buckets do not). Oracle-only;
    /// the product reads the buckets.
    #[cfg(test)]
    pub(crate) fn total(&self) -> DownUpOrder<u64> {
        DownUpOrder::new(
            self.tcp
                .down
                .saturating_add(self.udp.down)
                .saturating_add(self.icmp.down),
            self.tcp
                .up
                .saturating_add(self.udp.up)
                .saturating_add(self.icmp.up),
        )
    }
}

/// Cluster-2: the latest published flow-history snapshot (PART A + PART B).
/// `None` when the actor is inactive (kernel mode) — callers then keep the
/// empty-shape fallback (the honest source-dispatch pattern).
static FLOW_HISTORY: Lazy<ArcSwap<flow_history::FlowHistorySnapshot>> =
    Lazy::new(|| ArcSwap::new(Arc::new(flow_history::FlowHistorySnapshot::default())));

pub fn flow_history_snapshot() -> Option<Arc<flow_history::FlowHistorySnapshot>> {
    if !is_active() {
        return None;
    }
    Some(FLOW_HISTORY.load_full())
}

/// 3a cluster-1 increment 2: the latest published circuit temporal-heatmap
/// snapshot. `None` when the actor is inactive (kernel mode) — callers then
/// keep the legacy `THROUGHPUT_TRACKER` source (the source-dispatch pattern).
static CIRCUIT_HEATMAP: Lazy<ArcSwap<circuit_heatmap::CircuitHeatmapSnapshot>> =
    Lazy::new(|| ArcSwap::new(Arc::new(circuit_heatmap::CircuitHeatmapSnapshot::default())));

pub fn circuit_heatmap_snapshot() -> Option<Arc<circuit_heatmap::CircuitHeatmapSnapshot>> {
    if !is_active() {
        return None;
    }
    Some(CIRCUIT_HEATMAP.load_full())
}

/// 3a group-1 increment 2: the latest published per-circuit QoO snapshot.
/// `None` when the actor is inactive (kernel mode) — callers then keep the
/// legacy `THROUGHPUT_TRACKER` source.
static CIRCUIT_QOO: Lazy<ArcSwap<circuit_qoo::CircuitQooSnapshot>> =
    Lazy::new(|| ArcSwap::new(Arc::new(circuit_qoo::CircuitQooSnapshot::default())));

pub fn circuit_qoo_snapshot() -> Option<Arc<circuit_qoo::CircuitQooSnapshot>> {
    if !is_active() {
        return None;
    }
    Some(CIRCUIT_QOO.load_full())
}

/// The subscriber-IP → product circuit-id join for the heatmap fold (the same
/// catalog longest-prefix join `join_by_circuit` performs).
fn resolve_circuit_id_for_host(ip: &IpAddr) -> Option<String> {
    let key = XdpIpAddress::from(*ip);
    let catalog = lqos_network_devices::network_devices_catalog();
    catalog
        .device_longest_match_for_ip(&key)
        .map(|(_, device)| device.circuit_id.clone())
        .filter(|id| !id.trim().is_empty())
}

/// 3b DEFECT 1: the shared circuit→parent-node attribution. The
/// catalog keys circuits by `circuit_id` (`c1`), but the network.json tree keys
/// nodes by NAME (`s10-a-ap0`); joining on the circuit id silently never
/// matches, which emptied the tree live-view, the site/global heatmaps, and the
/// LTS per-site. Map each circuit to the catalog device's `parent_node`
/// (several circuits may share a parent). One catalog build; callers resolve
/// the name against their network.json snapshot.
pub(crate) fn circuit_parent_node_by_id() -> FxHashMap<String, String> {
    let catalog = lqos_network_devices::network_devices_catalog();
    let devices = catalog
        .iter_all_devices()
        .map(|device| (device.circuit_id.clone(), device.parent_node.clone()));
    circuit_parent_node_by_id_from(devices, |circuit_id| {
        crate::shaped_devices_tracker::effective_parent_for_circuit(circuit_id)
            .map(|parent| parent.name)
    })
}

/// Pure core of [`circuit_parent_node_by_id`]: prefer the effective parent,
/// fall back to the catalog's configured `parent_node`. Extracted so the
/// precedence is unit-testable on a **differing** `(configured, effective)`
/// input — the live runtime catalog stores the effective parent in BOTH fields
/// (`runtime_inputs.rs`), so a live-only test can never exercise the preference
/// (the differing-input unit test below).
fn circuit_parent_node_by_id_from<I, F>(
    devices: I,
    effective_parent: F,
) -> FxHashMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
    F: Fn(&str) -> Option<String>,
{
    let mut parents: FxHashMap<String, String> = FxHashMap::default();
    for (circuit_id, configured_parent) in devices {
        let circuit_id = circuit_id.trim();
        if circuit_id.is_empty() {
            continue;
        }
        // §lqos-74t.40 pin 4: the applied (effective) attribution wins over the
        // catalog's configured `parent_node`, so a circuit cannot attach to a
        // stale/synthetic parent.
        let parent_node = effective_parent(circuit_id)
            .unwrap_or(configured_parent)
            .trim()
            .to_string();
        if parent_node.is_empty() {
            continue;
        }
        parents.entry(circuit_id.to_string()).or_insert(parent_node);
    }
    parents
}

/// §`lqos-74t.47.1.2` R3: the circuit ids whose parent node is the synthetic
/// ORPHAN umbrella (a non-entity). Excluded from the per-flow QoO pool, matching
/// the ring fold's exclusion.
fn orphans_umbrella_circuits(parent_by_circuit: &FxHashMap<String, String>) -> FxHashSet<String> {
    let orphan_parent = lqos_network_devices::with_network_json_read(|net_json| {
        net_json
            .get_nodes_when_ready()
            .iter()
            .find(|node| node.id.as_deref() == Some(lqos_dataplane_types::ORPHANS_UMBRELLA_NODE_ID))
            .map(|node| node.name.clone())
    });
    let Some(orphan_parent) = orphan_parent else {
        return FxHashSet::default();
    };
    parent_by_circuit
        .iter()
        .filter(|(_, parent)| **parent == orphan_parent)
        .map(|(circuit_id, _)| circuit_id.clone())
        .collect()
}

/// The circuit's tree-node names (self-inclusive parent chain), resolved via
/// the shared [`circuit_parent_node_by_id`] attribution. Used by the pooled QoO
/// node rollup.
pub(crate) fn circuit_ancestors_for(
    circuit_id: &str,
    parent_by_circuit: &FxHashMap<String, String>,
) -> Vec<String> {
    let Some(parent_node) = parent_by_circuit.get(circuit_id) else {
        return Vec::new();
    };
    lqos_network_devices::with_network_json_read(|net_json| {
        let nodes = net_json.get_nodes_when_ready();
        let Some(node_idx) = nodes.iter().position(|node| node.name == *parent_node) else {
            return Vec::new();
        };
        // §lqos-74t.40 pin 4 (PARTIAL row exclusion corrected): the synthetic
        // `orphans` umbrella is a NON-ENTITY — a circuit attributed to it
        // contributes to NO site. Hiding only its row left the ancestor-chain
        // fold to inflate the root (WestRedd) via the umbrella.
        if nodes[node_idx].id.as_deref() == Some(lqos_dataplane_types::ORPHANS_UMBRELLA_NODE_ID) {
            return Vec::new();
        }
        nodes[node_idx]
            .parents
            .iter()
            .filter_map(|&idx| nodes.get(idx).map(|node| node.name.clone()))
            .collect()
    })
}

/// §lqos-74t.40 pin 4: index each tree node to its member circuits via the
/// shared circuit→parent attribution + the node ancestor chain (`node.parents`,
/// self-inclusive). A circuit attributed to the synthetic `orphans` umbrella is
/// a **non-entity**: it is dropped entirely, so it can never inflate an ancestor
/// (notably the root / WestRedd) through the chain — hiding only the umbrella's
/// own row was insufficient (the ancestor-chain fold bounded item). Pure and
/// unit-gated (`root_excludes_the_orphan_umbrella`).
pub(crate) fn node_circuits_by_index<'a>(
    nodes: &[lqos_config::NetworkJsonNode],
    parent_by_circuit: &FxHashMap<String, String>,
    circuits: impl Iterator<Item = &'a str>,
) -> FxHashMap<usize, Vec<&'a str>> {
    let index_by_name: FxHashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| (node.name.as_str(), idx))
        .collect();
    let mut node_circuits: FxHashMap<usize, Vec<&'a str>> = FxHashMap::default();
    for circuit_id in circuits {
        let Some(parent_node) = parent_by_circuit.get(circuit_id) else {
            continue;
        };
        let Some(&node_idx) = index_by_name.get(parent_node.as_str()) else {
            continue;
        };
        if nodes[node_idx].id.as_deref() == Some(lqos_dataplane_types::ORPHANS_UMBRELLA_NODE_ID) {
            continue;
        }
        for &ancestor in &nodes[node_idx].parents {
            node_circuits.entry(ancestor).or_default().push(circuit_id);
        }
    }
    node_circuits
}

/// The per-FlowId cumulative seed for the interval deltas: the DELIVERED
/// (shaped) byte counters (the tab rate; 3b plus the sent packet
/// counters (the 3b protocol splits).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FlowSeed {
    download_bytes: u64,
    upload_bytes: u64,
    sent_pkts_down: u64,
    sent_pkts_up: u64,
}

/// Converts a cumulative-counter delta over `elapsed_ms` into a per-second
/// value.
fn per_second(delta: u64, elapsed_ms: u64) -> u64 {
    if elapsed_ms == 0 {
        return 0;
    }
    ((delta as u128 * 1_000) / elapsed_ms as u128).min(u64::MAX as u128) as u64
}

/// 3a fill follow-up + 3b §1.1: per-flow interval rates by differencing the
/// cumulative counters across polls, keyed by FlowId. Returns the DIRECTIONAL
/// per-flow bytes/second (down/up separately; `.65` — the Shaper Top ASN
/// widgets AND the circuit page Top-ASN/Flows tabs read this; first sight / a
/// counter reset yields no rate and a reader falls back to the lifetime
/// average), and the per-protocol packets/second aggregate (the
/// `GetCurrentThroughput` splits; ruling c — the same seed pattern, so
/// the CLASSIFIED split sum tracks the all-protocol sent pps fold up to the
/// Unknown-transport residual). Absent flows drop their seed.
fn compute_flow_rates(
    seeds: &mut FxHashMap<analytics_ipc::FlowId, FlowSeed>,
    flows: &[analytics_ipc::FlowRow],
    elapsed_ms: u64,
) -> (
    FxHashMap<analytics_ipc::FlowId, DownUpOrder<u64>>,
    ProtocolPacketRates,
) {
    let mut directional: FxHashMap<analytics_ipc::FlowId, DownUpOrder<u64>> =
        FxHashMap::default();
    let mut protocol = ProtocolPacketRates::default();
    let mut fresh: FxHashMap<analytics_ipc::FlowId, FlowSeed> = FxHashMap::default();
    for flow in flows {
        let current = FlowSeed {
 // 3b: difference the DELIVERED (shaped) per-flow bytes —
            // the rate column/tab read delivered; `actual` is the demand.
            download_bytes: flow.shaped.download_bytes,
            upload_bytes: flow.shaped.upload_bytes,
            sent_pkts_down: flow.sent_pkts_down,
            sent_pkts_up: flow.sent_pkts_up,
        };
        if elapsed_ms > 0
            && let Some(previous) = seeds.get(&flow.id)
        {
            let bytes = current
                .download_bytes
                .checked_sub(previous.download_bytes)
                .zip(current.upload_bytes.checked_sub(previous.upload_bytes));
            if let Some((down, up)) = bytes {
                // `.65`: the down/up split is the ONLY per-flow interval rate
                // (the Shaper Top ASN widgets AND the circuit page tabs read
                // it; the legacy combined map is retired).
                directional.insert(
                    flow.id.clone(),
                    DownUpOrder::new(per_second(down, elapsed_ms), per_second(up, elapsed_ms)),
                );
            }
            let packets = current
                .sent_pkts_down
                .checked_sub(previous.sent_pkts_down)
                .zip(current.sent_pkts_up.checked_sub(previous.sent_pkts_up));
            if let Some((down, up)) = packets {
                protocol.add(
                    flow.id.protocol,
                    per_second(down, elapsed_ms),
                    per_second(up, elapsed_ms),
                );
            }
        }
        fresh.insert(flow.id.clone(), current);
    }
    *seeds = fresh;
    (directional, protocol)
}

/// The GeoTable join for the flow-history fold (ASN + ISO/name + lat/lon).
/// Shared with the 3a cluster-3 active-flow tabs.
pub(crate) fn resolve_flow_class(ip: IpAddr) -> flow_history::ResolvedClass {
    let owners = crate::throughput_tracker::flow_data::get_asn_name_and_country(ip);
    let (lat, lon) = crate::throughput_tracker::flow_data::get_asn_lat_lon(ip);
    flow_history::ResolvedClass {
        asn: crate::throughput_tracker::flow_data::lookup_asn_id(ip),
        country_iso: owners.flag,
        country_name: owners.country,
        lat,
        lon,
    }
}

/// Resolves the analytics socket path: the `LIBREQOS_ANALYTICS_SOCKET`
/// environment variable wins (it is the same variable the bridge
/// launch reads — a paired deployment pins it once), then the config
/// field, then the documented default.
pub fn analytics_socket_path(config: &lqos_config::DataplaneBridgeConfig) -> PathBuf {
    std::env::var_os("LIBREQOS_ANALYTICS_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&config.analytics_socket))
}

/// The SINGLE OWNER of the "is the analytics socket active, and where" predicate
/// (the implementer `` R1): active iff `[dataplane_bridge]` is present OR
/// `LIBREQOS_ANALYTICS_SOCKET` is set; the env wins for the path. The broker, the
/// analytics actor, and the supervisor MUST all use this so their gates cannot
/// diverge again (the broker once skipped the env case ⇒ the adopt never ran).
pub fn analytics_socket_path_if_active(config: &lqos_config::Config) -> Option<PathBuf> {
    let env_socket = std::env::var_os("LIBREQOS_ANALYTICS_SOCKET").is_some();
    if config.dataplane_bridge.is_none() && !env_socket {
        return None;
    }
    let settings = config.dataplane_bridge.clone().unwrap_or_default();
    Some(analytics_socket_path(&settings))
}

/// Starts the bridge analytics actor (idempotent). Active only when the
/// `[dataplane_bridge]` section is present or the analytics socket env
/// variable is set; otherwise returns without spawning (kernel source).
/// K1 §1.4: touch the dynamic-circuit store's `last_seen` for circuits the
/// bridge observed active this poll. This is the liveness backstop for the
/// RADIUS-interim-stall class; the kernel-observation feed is a separate arm.
fn report_bridge_liveness(joined: &FxHashMap<String, CircuitLiveRow>) {
    let active: Vec<String> = joined
        .iter()
        .filter(|(_, row)| {
            row.bytes_per_second.down > 0
                || row.bytes_per_second.up > 0
                || row.shaped_bytes_per_second.down > 0
                || row.shaped_bytes_per_second.up > 0
        })
        .map(|(circuit_id, _)| circuit_id.clone())
        .collect();
    if !active.is_empty() {
        lqos_network_devices::report_bridge_activity(&active);
    }
}

pub fn start_bridge_analytics(
    config: &lqos_config::Config,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
) {
 // R1: the SINGLE shared active/socket predicate.
    let Some(socket) = analytics_socket_path_if_active(config) else {
        debug!("Bridge analytics actor inactive: no [dataplane_bridge] config");
        return;
    };
    if STARTED.set(()).is_err() {
        return;
    }
    let settings = config.dataplane_bridge.clone().unwrap_or_default();
    if config.dataplane_bridge.is_none() {
        info!(
            "Bridge analytics actor active via LIBREQOS_ANALYTICS_SOCKET \
             (no [dataplane_bridge] section; pinned defaults)"
        );
    }
    // Pin the socket for the QueueStats consumers (cake_watcher) — the same
    // socket the snapshot poll uses.
    let _ = ANALYTICS_SOCKET.set(socket.clone());
    let interval = Duration::from_millis(
        settings
            .stats_poll_interval_ms
            .max(MIN_POLL_INTERVAL.as_millis() as u64),
    );
 // 3a LTS trigger: the periodic Insight throughput submission
    // was orphaned when K3-A deleted the kernel throughput loop. Re-home it on
    // this cold-path actor at the configured collation cadence, guarded by
    // gather_stats + is_active() (the payload builders already re-check the
    // license + gather_stats).
    let bridge_active = is_active();
    let lts_enabled = config.long_term_stats.gather_stats && bridge_active;
    let lts_period =
        Duration::from_secs(u64::from(config.long_term_stats.collation_period_seconds.max(1)));
    let lts_trigger = LtsSubmitTrigger::new(lts_enabled, lts_period, system_usage_tx);
    // `lqos-74t.2.1` instrument (removal gate 2026-12-31, DRIVER-owned): the
    // arm-time submission decision — a disabled trigger never fires, so this is
    // the FIRST place a never-submitting box is visible.
    info!(target: "lqos_lts_volume",
        "LTS submit trigger armed: enabled={lts_enabled} (gather_stats={} bridge_analytics_active={bridge_active}) period={lts_period:?}",
        config.long_term_stats.gather_stats,
    );
    info!(
        "Bridge analytics actor polling {} every {:?} (full population ≤ {})",
        socket.display(),
        interval,
        FULL_POPULATION_LIMIT
    );
    let consumer_socket = socket.clone();
    std::thread::Builder::new()
        .name("bridge-analytics".into())
        .spawn(move || actor(socket, interval, lts_trigger))
        .map(|_| ())
        .unwrap_or_else(|err| {
            warn!("Failed to spawn bridge analytics actor: {err:?}");
        });
    // Flow-completion feed (`lqos-74t.2.x`): a DEDICATED drain thread (not the
    // 1 s snapshot tick) over the bridge's completion ring, with a durable
    // `(epoch, seq)` cursor. The placeholder bridge arm returns
    // `CollectorUnavailable`, so this idles and the snapshot export stays the
    // fallback (`ring_active()` gates the double export).
    let cursor_path = config
        .resolved_state_directory()
        .join("completion_cursor.json");
    completion_consumer::spawn(consumer_socket, cursor_path);
}

/// Cold-path cadence gate for the LTS/Insight throughput submission: armed on
/// the first tick, fires once per `period` thereafter.
#[derive(Debug)]
struct LtsSubmitCadence {
    period: Duration,
    last: Option<Instant>,
}

impl LtsSubmitCadence {
    fn new(period: Duration) -> Self {
        Self { period, last: None }
    }

    /// True when `now` is at least one period past the last armed point.
    /// The first call only arms (never fires immediately). PURE aside from
    /// `self`.
    fn due(&mut self, now: Instant) -> bool {
        match self.last {
            None => {
                self.last = Some(now);
                false
            }
            Some(last) if now.saturating_duration_since(last) >= self.period => {
                self.last = Some(now);
                true
            }
            _ => false,
        }
    }
}

/// The LTS/Insight throughput-submission trigger. `counter` advances once per
/// actor tick (the legacy ~1 Hz semantics that gate the 60-tick shaper
/// utilization payload). PURE decision via [`lts_submit_due`]; `on_tick` has
/// the spawn side effect.
struct LtsSubmitTrigger {
    enabled: bool,
    cadence: LtsSubmitCadence,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
    counter: u8,
}

impl LtsSubmitTrigger {
    fn new(
        enabled: bool,
        period: Duration,
        system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
    ) -> Self {
        Self {
            enabled,
            cadence: LtsSubmitCadence::new(period),
            system_usage_tx,
            counter: 0,
        }
    }

    /// One actor tick: advance the legacy counter and, when the guard + cadence
    /// allow, spawn the submission off the poll thread. Side effects: may spawn
    /// a named thread that enqueues the LTS payload.
    fn on_tick(&mut self, now: Instant) {
        self.counter = self.counter.wrapping_add(1);
        if !lts_submit_due(self.enabled, &mut self.cadence, now) {
            return;
        }
        let system_usage_tx = self.system_usage_tx.clone();
        let counter = self.counter;
        // The bridge snapshot is already normalized per-second, so the legacy
        // tick-normalization scale is 1.0 (the old 1 Hz caller passed ~1.0).
        let _ = std::thread::Builder::new()
            .name("Throughput Stats Submit".into())
            .spawn(move || {
                crate::throughput_tracker::stats_submission::submit_throughput_stats(
                    1.0,
                    counter,
                    system_usage_tx,
                );
            });
    }
}

/// The trigger decision (the guard + the cadence), factored pure for the clock
/// oracle: disabled never fires and never arms.
fn lts_submit_due(enabled: bool, cadence: &mut LtsSubmitCadence, now: Instant) -> bool {
    enabled && cadence.due(now)
}

/// The analytics socket pinned by the actor (bridge mode only).
pub fn analytics_socket() -> Option<PathBuf> {
    ANALYTICS_SOCKET.get().cloned()
}

/// `.115` (`docs/74t115_design.md`): the bounded interest/last-reply cache.
/// The API path is a pure in-process read of this map; it NEVER calls
/// `connect_queue_stats`. The existing actor bulk poll (`refresh_queue_stats`)
/// fills the entries for the circuits in the interest set.
const RAW_QUEUE_CACHE_DEFAULT_CAP: usize = 256;
const RAW_QUEUE_STALE_DEFAULT_MS: u64 = 2_000;

/// `.115`: the bounded interest/last-reply cache capacity. Documented and
/// configurable (`LQOS_RAW_QUEUE_CACHE_CAP`, default 256, clamp 1..=2048); the
/// API path is O(1) regardless.
fn raw_queue_cache_cap() -> usize {
    std::env::var(lqos_dataplane_types::ENV_RAW_QUEUE_CACHE_CAP)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(RAW_QUEUE_CACHE_DEFAULT_CAP)
        .clamp(1, 2_048)
}

static RAW_QUEUE_CACHE: Lazy<parking_lot::Mutex<RawQueueCache>> =
    Lazy::new(|| parking_lot::Mutex::new(RawQueueCache::new(raw_queue_cache_cap())));
/// The actor poll interval, stamped at actor start (the cache refresh bound).
static RAW_QUEUE_REFRESH_MS: AtomicU64 = AtomicU64::new(1_000);

struct RawQueueEntry {
    row: QdiscDetailRow,
    sampled_at_unix_ms: Option<u64>,
    elapsed_ms: Option<u64>,
    cadence_ms: u64,
    sample_epoch_unix_ms: u64,
    sample_seq: u64,
    actor_gap: bool,
    counter_reset: bool,
    received_at: Instant,
}

struct RawQueueCache {
    cap: usize,
    entries: FxHashMap<String, Arc<RawQueueEntry>>,
    order: VecDeque<String>,
    interest: FxHashSet<String>,
    interest_order: VecDeque<String>,
}

impl RawQueueCache {
    fn new(cap: usize) -> Self {
        Self {
            cap,
            entries: FxHashMap::default(),
            order: VecDeque::new(),
            interest: FxHashSet::default(),
            interest_order: VecDeque::new(),
        }
    }

    /// The circuits worth caching: cached entries + request interest.
    fn interesting_ids(&self) -> FxHashSet<String> {
        let mut ids: FxHashSet<String> = self.entries.keys().cloned().collect();
        ids.extend(self.interest.iter().cloned());
        ids
    }
}

impl RawQueueCache {
    fn get(&mut self, circuit_id: &str) -> Option<Arc<RawQueueEntry>> {
        let entry = self.entries.get(circuit_id).cloned()?;
        self.touch(circuit_id);
        Some(entry)
    }

    fn touch(&mut self, circuit_id: &str) {
        self.order.retain(|id| id != circuit_id);
        self.order.push_back(circuit_id.to_string());
    }

    fn note_interest(&mut self, circuit_id: &str) {
        if self.interest.insert(circuit_id.to_string()) {
            self.interest_order.push_back(circuit_id.to_string());
        }
        while self.interest.len() > self.cap {
            let Some(oldest) = self.interest_order.pop_front() else {
                break;
            };
            if oldest != circuit_id {
                self.interest.remove(&oldest);
            }
        }
    }

    fn put_entry(&mut self, circuit_id: &str, entry: Arc<RawQueueEntry>) {
        if !self.entries.contains_key(circuit_id) && !self.interest.contains(circuit_id) {
            return;
        }
        self.entries.insert(circuit_id.to_string(), entry);
        self.touch(circuit_id);
        while self.entries.len() > self.cap {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if oldest != circuit_id {
                self.entries.remove(&oldest);
            }
        }
    }

    fn retain_live(&mut self, live: &FxHashSet<String>) {
        self.entries.retain(|id, _| live.contains(id));
        self.order.retain(|id| live.contains(id));
        self.interest.retain(|id| live.contains(id));
        self.interest_order.retain(|id| live.contains(id));
    }
}

fn raw_queue_entry(
    row: &QdiscDetailRow,
    reply: &analytics_ipc::QueueStatsReply,
    received_at: Instant,
) -> Arc<RawQueueEntry> {
    Arc::new(RawQueueEntry {
        row: row.clone(),
        sampled_at_unix_ms: reply.sampled_at_unix_ms,
        elapsed_ms: reply.elapsed_ms,
        cadence_ms: reply.cadence_ms,
        sample_epoch_unix_ms: reply.sample_epoch_unix_ms,
        sample_seq: reply.sample_seq,
        actor_gap: reply.actor_gap,
        counter_reset: reply.counter_reset,
        received_at,
    })
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn raw_queue_stale_floor_ms() -> u64 {
    std::env::var(lqos_dataplane_types::ENV_RAW_QUEUE_STALE_MS)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(RAW_QUEUE_STALE_DEFAULT_MS)
        .clamp(500, 60_000)
}

fn raw_queue_direction(row: &analytics_ipc::QdiscDirectionRow) -> QueueSampleDirection {
    // The bridge wire carries cumulative sent bytes only per CAKE tin; a
    // non-CAKE direction has no cumulative counters (never a fake zero).
    let tins_available = !row.tins.is_empty();
    let counters_available = tins_available;
    QueueSampleDirection {
        kind: if tins_available { "cake" } else { "none" }.to_string(),
        tins_available,
        counters_available,
        sent_bytes: counters_available.then(|| {
            row.tins
                .iter()
                .fold(0_u64, |sum, tin| sum.saturating_add(tin.sent_bytes))
        }),
        drop_pkts: counters_available.then_some(row.drop_pkts),
        aqm_marks: counters_available.then_some(row.aqm_marks),
        backlog_bytes: row
            .tins
            .iter()
            .fold(0_u64, |sum, tin| sum.saturating_add(tin.backlog_bytes)),
        queue_depth_pkts: row.queue_depth_pkts,
        memory_used: row.memory_used,
        tins: row
            .tins
            .iter()
            .map(|tin| QueueSampleTin {
                sent_bytes: tin.sent_bytes,
                backlog_bytes: tin.backlog_bytes,
                sojourn_p99_ns: tin.sojourn_p99_ns,
            })
            .collect(),
    }
}

/// `.115`: a bridge-analytics snapshot for one circuit, read **only** from the
/// in-process last-reply cache. A miss records interest (bounded) and returns
/// None; it never opens the analytics socket. The next actor poll fills it.
pub fn raw_queue_snapshot(circuit_id: &str) -> Option<QueueStoreTransit> {
    if !is_active() {
        return None;
    }
    let entry = {
        let mut cache = RAW_QUEUE_CACHE.lock();
        match cache.get(circuit_id) {
            Some(entry) => entry,
            None => {
                cache.note_interest(circuit_id);
                return None;
            }
        }
    };
    // A reply without sample provenance (old bridge) is explicitly unavailable.
    let sampled_at = entry.sampled_at_unix_ms?;
    let now = unix_now_ms();
    let cadence_ms = entry.cadence_ms.max(1);
    let refresh_ms = RAW_QUEUE_REFRESH_MS.load(Ordering::Relaxed).max(cadence_ms);
    let observed_ms = entry.elapsed_ms.unwrap_or(cadence_ms);
    let stale_after_ms = (observed_ms.saturating_mul(2))
        .max(cadence_ms.saturating_mul(2))
        .max(refresh_ms.saturating_mul(2))
        .max(raw_queue_stale_floor_ms());
    let sample_age_ms = now.checked_sub(sampled_at);
    let cache_age_ms = u64::try_from(entry.received_at.elapsed().as_millis()).ok();
    let clock_step = now < sampled_at;
    let stale = clock_step
        || sample_age_ms.is_none_or(|age| age > stale_after_ms)
        || cache_age_ms.is_none_or(|age| age > stale_after_ms);
    let invalid_window = clock_step || entry.counter_reset;
    let down = raw_queue_direction(&entry.row.download);
    let up = raw_queue_direction(&entry.row.upload);
    Some(QueueStoreTransit {
        history: Vec::new(),
        history_head: 0,
        current_download: CakeTransit {
            memory_used: entry.row.download.memory_used,
        },
        current_upload: CakeTransit {
            memory_used: entry.row.upload.memory_used,
        },
        kind_down: down.kind.clone(),
        kind_up: up.kind.clone(),
        per_direction: true,
        snapshot: Some(QueueSampleSnapshot {
            sampled_at_unix_ms: sampled_at,
            now_unix_ms: now,
            sample_age_ms,
            cache_age_ms,
            clock_step,
            invalid_window,
            elapsed_ms: entry.elapsed_ms,
            cadence_ms,
            stale_after_ms,
            sample_epoch_unix_ms: entry.sample_epoch_unix_ms,
            sample_seq: entry.sample_seq,
            actor_gap: entry.actor_gap,
            stale,
            counter_reset: entry.counter_reset,
            download: down,
            upload: up,
        }),
        history_len: 0,
        watch_expires_at_unix_ms: None,
    })
}

/// K2 slice 2 consumers: one circuit's per-direction qdisc detail from the
/// bridge, resolved by PRODUCT circuit id through the actor's by-circuit
/// cache (the same IP-keyed join the live rates use). Bridge-only.
/// Returns `None` when the actor is inactive, the circuit is unknown, the
/// socket was never pinned, or the request fails — the caller renders
/// "unavailable" rather than fabricating a row.
pub fn queue_stats_for_circuit(circuit_id: &str) -> Option<QdiscDetailRow> {
    if !is_active() {
        return None;
    }
    let slot = snapshot()
        .by_circuit_id
        .get(circuit_id)
        .map(|row| row.bridge_circuit_id)?;
    let socket = ANALYTICS_SOCKET.get()?;
    connect_queue_stats(socket, &[slot])
        .ok()?
        .rows
        .into_iter()
        .next()
}

/// 3a A1: the wire's per-request slot bound (`analytics_ipc::MAX_QUEUE_STATS_IDS`).
/// A `connect_queue_stats` request may carry at most this many slot ids, and a
/// live box has far more circuits, so the bulk read CHUNKS into batches of this
/// size — still a bulk read, never a one-at-a-time `queue_stats_for_circuit` loop.
const QUEUE_STATS_CHUNK: usize = analytics_ipc::MAX_QUEUE_STATS_IDS;

/// 3a A1: the live `slot -> product circuit id` map from `snapshot().by_circuit_id`.
/// Slot 0 (`CircuitId::best_effort`) never has a leaf, so it is excluded (the
/// wire rejects it).
fn slot_to_circuit_id(snapshot: &BridgeStatsSnapshot) -> FxHashMap<u64, String> {
    snapshot
        .by_circuit_id
        .iter()
        .filter(|(_, row)| row.bridge_circuit_id != 0)
        .map(|(circuit_id, row)| (row.bridge_circuit_id, circuit_id.clone()))
        .collect()
}

/// 3a A1: fold one bulk reply's rows into the per-circuit CUMULATIVE drops/marks
/// (keyed by the LTS `circuit_hash`) + the observed queue-kind counts. The wire
/// carries no qdisc-kind field: a non-empty `tins` is the CAKE leaf marker, so
/// only the CAKE count is sourced (`htb`/`fq_codel` stay honest-empty, disclosed
/// in `docs/ga_release_notes.md`). Rows whose slot is unknown are skipped.
fn fold_queue_stats_rows(
    rows: &[QdiscDetailRow],
    slot_to_circuit: &FxHashMap<u64, String>,
) -> (Vec<(i64, DownUpOrder<u64>, DownUpOrder<u64>)>, QueueCounts) {
    let mut per_circuit = Vec::with_capacity(rows.len());
    let mut counts = QueueCounts::default();
    for row in rows {
        let Some(circuit_id) = slot_to_circuit.get(&row.slot_id) else {
            continue;
        };
        per_circuit.push((
            lqos_utils::hash_to_i64(circuit_id),
            DownUpOrder::new(row.download.drop_pkts, row.upload.drop_pkts),
            DownUpOrder::new(row.download.aqm_marks, row.upload.aqm_marks),
        ));
        if !row.download.tins.is_empty() {
            counts.cake += 1;
        }
        if !row.upload.tins.is_empty() {
            counts.cake += 1;
        }
    }
    (per_circuit, counts)
}

/// 3a A1: ONE batched bridge qdisc read over every live circuit
/// (`snapshot().by_circuit_id`), feeding BOTH the LTS `circuit_cake_{drops,marks}`
/// and B2's `ExecutiveSummaryHeader` queue counts from the SAME read. The
/// per-circuit counters land in `ALL_QUEUE_SUMMARY`, which keeps each circuit's
/// previous value so `iterate_queues` still yields the per-interval DELTA the LTS
/// contract wants; `queue_stats_stale` reflects this source (a failed or
/// empty-but-nonzero-slots read ⇒ `true`), never a hardwired `false`. Cold path:
/// the actor calls it once per poll; the requests are chunked to the wire bound.
/// §lqos-74t.8.1 localizer: the previous poll's raw cumulative drop/mark sums,
/// so one trace line shows whether the wire counters INCREASED (a lone
/// `raw_drop>0 && total_drop==0` is normal until the delta is non-zero).
static QUEUE_STATS_TRACE_PREV: Lazy<parking_lot::Mutex<Option<(u64, u64, u64, u64)>>> =
    Lazy::new(|| parking_lot::Mutex::new(None));

fn refresh_queue_stats(socket: &Path) {
    if !is_active() {
        return;
    }
    let snapshot = snapshot();
    let slot_to_circuit = slot_to_circuit_id(&snapshot);
    let mut slots: Vec<u64> = slot_to_circuit.keys().copied().collect();
    slots.sort_unstable();
    let mut per_circuit: Vec<(i64, DownUpOrder<u64>, DownUpOrder<u64>)> =
        Vec::with_capacity(slots.len());
    let mut counts = QueueCounts::default();
    let mut failed = false;
    // §lqos-74t.8.1 LIVE-GAP localizer (env-gated, diagnostic-only): the raw
    // cumulative `drop_pkts`/`aqm_marks`/`drop_pkts_queue_full` the bridge wire
    // carries vs the differenced `TOTAL_QUEUE_STATS` `GetQueueStatsTotal` reads.
    // Enable with `LIBREQOS_TRACE_QUEUE_STATS=1`; unset ⇒ no log.
    let trace = std::env::var_os("LIBREQOS_TRACE_QUEUE_STATS").is_some();
    let mut raw_drop_down: u64 = 0;
    let mut raw_drop_up: u64 = 0;
    let mut raw_mark_down: u64 = 0;
    let mut raw_mark_up: u64 = 0;
    let mut raw_queue_full: u64 = 0;
    let mut tin_directions: usize = 0;
    for chunk in slots.chunks(QUEUE_STATS_CHUNK) {
        match connect_queue_stats(socket, chunk) {
            Ok(reply) => {
                if trace {
                    for row in &reply.rows {
                        raw_drop_down = raw_drop_down.saturating_add(row.download.drop_pkts);
                        raw_drop_up = raw_drop_up.saturating_add(row.upload.drop_pkts);
                        raw_mark_down = raw_mark_down.saturating_add(row.download.aqm_marks);
                        raw_mark_up = raw_mark_up.saturating_add(row.upload.aqm_marks);
                        raw_queue_full = raw_queue_full
                            .saturating_add(row.download.drop_pkts_queue_full)
                            .saturating_add(row.upload.drop_pkts_queue_full);
                        if !row.download.tins.is_empty() {
                            tin_directions += 1;
                        }
                        if !row.upload.tins.is_empty() {
                            tin_directions += 1;
                        }
                    }
                }
                let (batch, batch_counts) = fold_queue_stats_rows(&reply.rows, &slot_to_circuit);
                counts.cake += batch_counts.cake;
                per_circuit.extend(batch);
                // `.115`: fill the bounded interest/last-reply cache from this
                // SAME reply (row + provenance). Build the Arc entries OUTSIDE
                // the cache mutex (no row clone under the lock), then insert.
                let interested = RAW_QUEUE_CACHE.lock().interesting_ids();
                let received_at = Instant::now();
                let mut to_cache: Vec<(String, Arc<RawQueueEntry>)> = Vec::new();
                for row in &reply.rows {
                    if let Some(circuit_id) = slot_to_circuit.get(&row.slot_id)
                        && interested.contains(circuit_id)
                    {
                        to_cache.push((
                            circuit_id.clone(),
                            raw_queue_entry(row, &reply, received_at),
                        ));
                    }
                }
                if !to_cache.is_empty() {
                    let mut cache = RAW_QUEUE_CACHE.lock();
                    for (circuit_id, entry) in to_cache {
                        cache.put_entry(&circuit_id, entry);
                    }
                }
            }
            Err(_) => failed = true,
        }
    }
    // A truncated read must not present as a full one.
    let read_empty = !slots.is_empty() && per_circuit.is_empty();
    let rows = per_circuit.len();
    // `.115`: drop cache entries/interest for circuits that left the snapshot.
    let live: FxHashSet<String> = slot_to_circuit.values().cloned().collect();
    RAW_QUEUE_CACHE.lock().retain_live(&live);
    ALL_QUEUE_SUMMARY.update(per_circuit, counts);
    set_queue_stats_stale(failed || read_empty);
    if trace {
        let (
            raw_drop_down_delta,
            raw_drop_up_delta,
            raw_mark_down_delta,
            raw_mark_up_delta,
        ) = {
            let mut prev = QUEUE_STATS_TRACE_PREV.lock();
            let delta = match *prev {
                Some((pd, pu, md, mu)) => (
                    raw_drop_down.saturating_sub(pd),
                    raw_drop_up.saturating_sub(pu),
                    raw_mark_down.saturating_sub(md),
                    raw_mark_up.saturating_sub(mu),
                ),
                None => (0, 0, 0, 0),
            };
            *prev = Some((raw_drop_down, raw_drop_up, raw_mark_down, raw_mark_up));
            delta
        };
        let total = &TOTAL_QUEUE_STATS;
        warn!(
            event = "A1_QUEUE_STATS_TRACE",
            slots = slots.len(),
            rows,
            failed,
            cake_leaves = counts.cake,
            tin_directions,
            raw_drop_down,
            raw_drop_down_delta,
            raw_drop_up,
            raw_drop_up_delta,
            raw_queue_full,
            raw_mark_down,
            raw_mark_down_delta,
            raw_mark_up,
            raw_mark_up_delta,
            total_drop_down = total.drops.get_down(),
            total_drop_up = total.drops.get_up(),
            total_mark_down = total.marks.get_down(),
            total_mark_up = total.marks.get_up(),
            "A1 queue-stats drop_pkts (per direction) vs TOTAL_QUEUE_STATS"
        );
    }
}

/// K2 slice 1: the top-level counter seed for per-poll differencing.
#[derive(Clone, Copy, Debug, Default)]
struct TopTotals {
    actual_down: u64,
    actual_up: u64,
    shaped_down: u64,
    shaped_up: u64,
    /// `.66.2`: the cumulative DEMAND class counters (`.66.1`).
    mapped_down: u64,
    mapped_up: u64,
    unmapped_down: u64,
    unmapped_up: u64,
    captured_millis: u64,
}

/// Per-poll rate memory: the previous valid interval delta per host (to bridge
/// a counter reset) and the last-good circuit rows (to bridge a missing host
/// row). Pure state, owned by the actor/poll caller so tests can drive it.
#[derive(Default)]
struct CircuitRateMemory {
    deltas: FxHashMap<HostKey, translate::HostDelta>,
    rows: FxHashMap<String, (CircuitLiveRow, u64)>,
}

fn actor(socket: PathBuf, interval: Duration, mut lts_trigger: LtsSubmitTrigger) {
    // `.115`: the cache refresh bound is the actor poll interval.
    RAW_QUEUE_REFRESH_MS.store(
        u64::try_from(interval.as_millis())
            .unwrap_or(u64::MAX)
            .max(1),
        Ordering::Relaxed,
    );
    let mut seeds = FxHashMap::default();
    let mut prev_captured_millis = None;
    let mut prev_totals = None;
    let mut history = flow_history::FlowHistory::new();
    let mut heatmap = circuit_heatmap::CircuitHeatmapAccumulator::new();
    let mut qoo = circuit_qoo::CircuitQooAccumulator::new();
    let mut flow_seeds: FxHashMap<analytics_ipc::FlowId, FlowSeed> = FxHashMap::default();
    let mut rate_memory = CircuitRateMemory::default();
    loop {
        let poll_started = Instant::now();
        poll_and_store(
            &socket,
            &mut seeds,
            &mut prev_captured_millis,
            &mut prev_totals,
            &mut history,
            &mut heatmap,
            &mut qoo,
            &mut flow_seeds,
            &mut rate_memory,
        );
        // 3b §1.3: the poll thread's own timing, the `GetLqosStats`
        // `time_to_poll_hosts` gauge (microseconds, the kernel-era unit).
        let elapsed_us = poll_started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        crate::stats::TIME_TO_POLL_HOSTS.store(elapsed_us, std::sync::atomic::Ordering::Relaxed);
        // 3a LTS trigger: a cold-path submission enqueued on the collation
        // cadence, off the poll thread.
        lts_trigger.on_tick(Instant::now());
        std::thread::sleep(interval);
    }
}

/// One poll cycle: fetch + translate + store (or mark stale on failure).
#[allow(clippy::too_many_arguments)]
fn poll_and_store(
    socket: &PathBuf,
    seeds: &mut FxHashMap<HostKey, translate::HostSeed>,
    prev_captured_millis: &mut Option<u64>,
    prev_totals: &mut Option<TopTotals>,
    history: &mut flow_history::FlowHistory,
    heatmap: &mut circuit_heatmap::CircuitHeatmapAccumulator,
    qoo: &mut circuit_qoo::CircuitQooAccumulator,
    flow_seeds: &mut FxHashMap<analytics_ipc::FlowId, FlowSeed>,
    rate_memory: &mut CircuitRateMemory,
) {
    poll_and_store_with(
        socket,
        seeds,
        prev_captured_millis,
        prev_totals,
        history,
        heatmap,
        qoo,
        flow_seeds,
        rate_memory,
        resolve_circuit_id_for_host,
        join_with_catalog,
    );
}

/// Testable core of one poll cycle (the join is injected so tests do
/// not depend on the global network-devices catalog).
/// §`lqos-74t.31` W1: the configured do-not-track CIDR table for one poll.
pub(crate) fn do_not_track_table() -> Option<ip_network_table::IpNetworkTable<bool>> {
    let config = lqos_config::load_config().ok()?;
    build_do_not_track_table(config.flows.as_ref()?.do_not_track_cidrs())
}

/// Pure builder (testable): parse CIDRs into a longest-match table. `None` when
/// the set is empty or no entry parses ⇒ track everything.
fn build_do_not_track_table(
    subnets: &[String],
) -> Option<ip_network_table::IpNetworkTable<bool>> {
    if subnets.is_empty() {
        return None;
    }
    let mut table = ip_network_table::IpNetworkTable::new();
    let mut any = false;
    for cidr in subnets {
        let Some((addr, prefix)) = cidr.trim().split_once('/') else {
            continue;
        };
        let Ok(prefix) = prefix.parse::<u8>() else {
            continue;
        };
        let inserted = if let Ok(ip) = addr.parse::<std::net::Ipv4Addr>() {
            ip_network::IpNetwork::new(ip, prefix).map(|net| table.insert(net, true))
        } else if let Ok(ip) = addr.parse::<std::net::Ipv6Addr>() {
            ip_network::IpNetwork::new(ip, prefix).map(|net| table.insert(net, true))
        } else {
            continue;
        };
        if inserted.is_ok() {
            any = true;
        }
    }
    if any { Some(table) } else { None }
}

/// §`lqos-74t.31.1`: drop every do-not-track host/flow row from the poll. Covers
/// the shaped host table, the flow tables (active + recent), AND the `.49.1`
/// best-effort/unmapped host class (the unknown-IPs page source) — so a
/// do-not-track IP never surfaces on the flow/RTT rollups or the unknown surface.
fn apply_do_not_track_filter(
    next: &mut analytics_ipc::AnalyticsSnapshot,
    block: &ip_network_table::IpNetworkTable<bool>,
) {
    next.hosts
        .retain(|row| block.longest_match(row.id.subscriber_ip).is_none());
    next.active_flows
        .retain(|flow| block.longest_match(flow.id.subscriber_ip).is_none());
    next.recent_flows
        .retain(|flow| block.longest_match(flow.id.subscriber_ip).is_none());
    next.best_effort_hosts
        .retain(|row| block.longest_match(row.id.subscriber_ip).is_none());
}

#[allow(clippy::too_many_arguments)]
fn poll_and_store_with(
    socket: &PathBuf,
    seeds: &mut FxHashMap<HostKey, translate::HostSeed>,
    prev_captured_millis: &mut Option<u64>,
    prev_totals: &mut Option<TopTotals>,
    history: &mut flow_history::FlowHistory,
    heatmap: &mut circuit_heatmap::CircuitHeatmapAccumulator,
    qoo: &mut circuit_qoo::CircuitQooAccumulator,
    flow_seeds: &mut FxHashMap<analytics_ipc::FlowId, FlowSeed>,
    rate_memory: &mut CircuitRateMemory,
    resolve_host: fn(&IpAddr) -> Option<String>,
    join: fn(
        &analytics_ipc::AnalyticsSnapshot,
        &FxHashMap<HostKey, translate::HostDelta>,
    ) -> FxHashMap<String, CircuitLiveRow>,
) {
    match connect_snapshot(socket, FULL_POPULATION_LIMIT) {
        Ok(next) => {
            // §`lqos-74t.31` W1: drop do-not-track subnets from the flow/RTT
            // rollups (the v2 "Do-Not-Track" set — e.g. speed-test hosts). The
            // matcher is built once per poll; `None` ⇒ track everything.
            let mut next = next;
            if let Some(block) = do_not_track_table() {
                apply_do_not_track_filter(&mut next, &block);
            }
            let poll_elapsed_ms = next
                .captured_at_millis
                .saturating_sub(prev_captured_millis.unwrap_or(next.captured_at_millis));
            let (deltas, fresh_seeds) = difference(
                seeds,
                prev_captured_millis.unwrap_or(next.captured_at_millis),
                &next,
            );
            *seeds = fresh_seeds;
            *prev_captured_millis = Some(next.captured_at_millis);
            let host_rows = next.hosts.len();
            let captured = next.captured_at_millis;
            let (flow_bytes_per_second_directional, protocol_packets_per_second) =
                compute_flow_rates(flow_seeds, &next.active_flows, poll_elapsed_ms);
            // P2 (`lqos-74t.117`): bridge counters are cumulative. A
            // counter reset (retention eviction) or a missing host row yields
            // no delta, which used to drop the circuit from the join. Carry
            // the previous interval delta for a present-but-reset host, then
            // carry the last-good circuit row when a circuit has no row at
            // all. Both are surfaced via `last_good_circuit_ids`; never a
            // silent zero/gap.
            let present_keys: FxHashSet<HostKey> = next
                .hosts
                .iter()
                .map(|row| (row.id.circuit_id, row.id.subscriber_ip))
                .collect();
            let mut effective_deltas = deltas.clone();
            let mut carried_keys: FxHashSet<HostKey> = FxHashSet::default();
            for (key, previous) in rate_memory.deltas.iter() {
                if present_keys.contains(key) && !effective_deltas.contains_key(key) {
                    effective_deltas.insert(*key, *previous);
                    carried_keys.insert(*key);
                }
            }
            rate_memory.deltas = deltas.clone();
            let mut joined = join(&next, &effective_deltas);
            let mut last_good_ids: FxHashSet<String> = FxHashSet::default();
            for key in &carried_keys {
                if let Some(circuit_id) = resolve_host(&key.1) {
                    last_good_ids.insert(circuit_id.to_string());
                }
            }
            for (circuit_id, (row, seen_at)) in rate_memory.rows.iter() {
                if captured.saturating_sub(*seen_at) > LAST_GOOD_WINDOW_MS {
                    continue;
                }
                if !joined.contains_key(circuit_id) {
                    joined.insert(circuit_id.clone(), row.clone());
                    last_good_ids.insert(circuit_id.clone());
                }
            }
            for (circuit_id, row) in &joined {
                if !last_good_ids.contains(circuit_id) {
                    rate_memory
                        .rows
                        .insert(circuit_id.clone(), (row.clone(), captured));
                }
            }
            rate_memory
                .rows
                .retain(|_, (_, seen_at)| captured.saturating_sub(*seen_at) <= LAST_GOOD_WINDOW_MS);
            let by_ip = build_by_ip_rates(&deltas, &next.hosts);
            // `.49.1`: the SEPARATE best-effort host map (never `by_ip`).
            let best_effort_by_ip =
                build_best_effort_by_ip(&next.best_effort_hosts, next.captured_at_millis);
            report_range_activity(&by_ip);
            let active_flow_ips = next
                .active_flows
                .iter()
                .map(|flow| flow.id.subscriber_ip)
                .collect();
            // K2 slice 1: difference the top-level totals over the poll
            // window (the bridge's own counters, not the truncated host table).
            let elapsed_ms = captured.saturating_sub(
                prev_totals
                    .map(|totals| totals.captured_millis)
                    .unwrap_or(captured),
            );
            let (
                bridge_actual_bytes_per_second,
                bridge_shaped_bytes_per_second,
                bridge_mapped_bytes_per_second,
                bridge_unmapped_bytes_per_second,
            ) = match (next.actual.as_ref(), next.shaped.as_ref()) {
                (Some(actual), Some(shaped)) => {
                    let (actual_bps, shaped_bps) = match *prev_totals {
                        Some(prev) if elapsed_ms > 0 => (
                            DownUpOrder::new(
                                interval_bytes_per_second(
                                    actual.download_bytes,
                                    prev.actual_down,
                                    elapsed_ms,
                                ),
                                interval_bytes_per_second(
                                    actual.upload_bytes,
                                    prev.actual_up,
                                    elapsed_ms,
                                ),
                            ),
                            DownUpOrder::new(
                                interval_bytes_per_second(
                                    shaped.download_bytes,
                                    prev.shaped_down,
                                    elapsed_ms,
                                ),
                                interval_bytes_per_second(
                                    shaped.upload_bytes,
                                    prev.shaped_up,
                                    elapsed_ms,
                                ),
                            ),
                        ),
                        _ => (DownUpOrder::default(), DownUpOrder::default()),
                    };
                    // `.66.2`: the DEMAND class split (mapped/unmapped), diffed
                    // from the cumulative `.66.1` counters on the SAME window/seed.
                    let (mapped_bps, unmapped_bps) = match *prev_totals {
                        Some(prev) if elapsed_ms > 0 => (
                            interval_rate(
                                next.mapped.as_ref(),
                                prev.mapped_down,
                                prev.mapped_up,
                                elapsed_ms,
                            ),
                            interval_rate(
                                next.unmapped.as_ref(),
                                prev.unmapped_down,
                                prev.unmapped_up,
                                elapsed_ms,
                            ),
                        ),
                        _ => (DownUpOrder::default(), DownUpOrder::default()),
                    };
                    *prev_totals = Some(TopTotals {
                        actual_down: actual.download_bytes,
                        actual_up: actual.upload_bytes,
                        shaped_down: shaped.download_bytes,
                        shaped_up: shaped.upload_bytes,
                        mapped_down: counter_down(next.mapped.as_ref()),
                        mapped_up: counter_up(next.mapped.as_ref()),
                        unmapped_down: counter_down(next.unmapped.as_ref()),
                        unmapped_up: counter_up(next.unmapped.as_ref()),
                        captured_millis: captured,
                    });
                    (actual_bps, shaped_bps, mapped_bps, unmapped_bps)
                }
                _ => {
                    *prev_totals = None;
                    (
                        DownUpOrder::default(),
                        DownUpOrder::default(),
                        DownUpOrder::default(),
                        DownUpOrder::default(),
                    )
                }
            };
            // Cluster-2: fold this tick into the flow-history ring + PART A
            // retention, then publish the immutable snapshot for readers.
            // The stale path below intentionally folds NOTHING (§8 fold 7).
            history.fold(&next, captured, resolve_flow_class);
            FLOW_HISTORY.store(Arc::new(history.publish()));
            // ASN heatmap: the flow-history rollup boundary produced the
            // per-ASN aggregate; write the store with the rollup's cycle id,
            // gated by the same config the reader (`asn_heatmaps`) uses.
            let asn_heatmap_enabled = lqos_config::load_config()
                .map(|cfg| cfg.enable_asn_heatmaps)
                .unwrap_or(true);
            // card v3 pin A: ONE per-tick sample per ASN (byte/packet/retransmit
            // deltas + the TRUE p50/p90), written on the store's TICK clock in
            // SECONDS — `EXPIRE_CYCLES = 900` s; this single clock domain is the
            // v1 millis-vs-seconds purge-bug fix.
            let asn_tick = history.drain_asn_heat();
            crate::throughput_tracker::flow_data::update_asn_heatmaps(
                asn_tick,
                captured / 1000,
                asn_heatmap_enabled,
            );
            // 3a cluster-1 increment 2: fold the raw host counters into the
            // per-circuit temporal heatmap (own differencing) and publish the
            // immutable view. The stale path below folds NOTHING (last-good
            // stays published), matching the flow-history discipline.
            heatmap.fold_tick(captured, &next.hosts, resolve_host);
            CIRCUIT_HEATMAP.store(Arc::new(heatmap.publish()));
            // 3a group-1 increment 2: score the active flows into the
            // per-circuit QoO accumulator (shared scorer + the three gates)
            // and publish. The stale path below folds nothing.
            // 3b DEFECT 1: build the shared circuit→parent-node attribution
            // ONCE per tick and reuse it for the QoO node rollup + the tree
            // live-view (the site heatmaps resolve it in their own reader).
            let parent_by_circuit = circuit_parent_node_by_id();
            let qoo_profile = lqos_config::active_qoo_profile().ok();
            // §`lqos-74t.40.1`: the per-cycle RTT snapshot now comes from the
            // BRIDGE distribution (`CircuitHeatmapSnapshot.rtt_samples_by_circuit`,
            // published just above). The dead `THROUGHPUT_TRACKER` is no longer
            // read — it has no production writer post-K3 — so the tree/site RTT
            // and the QoO p90 agree on ONE live source. The tree write
            // (`apply_tree_rtt_accumulation`) still follows later.
            let heatmap_snapshot = circuit_heatmap_snapshot()
                .unwrap_or_else(|| Arc::new(circuit_heatmap::CircuitHeatmapSnapshot::default()));
            let rtt_snapshot = snapshot_bridge_rtt(&heatmap_snapshot, &parent_by_circuit);
            // §`lqos-74t.47.1.2` R3: the per-flow QoO pool replaces the ring pool,
            // so re-apply the ORPHAN-umbrella exclusion the ring fold applied.
            let orphans_circuits = orphans_umbrella_circuits(&parent_by_circuit);
            qoo.fold_tick(
                &next.active_flows,
                resolve_host,
                |circuit_id| circuit_ancestors_for(circuit_id, &parent_by_circuit),
                &orphans_circuits,
                qoo_profile.as_deref(),
            );
            CIRCUIT_QOO.store(Arc::new(qoo.publish()));
            // 3a netflow + the LTS2 (Insight) per-flow export are the FALLBACK
            // when the completion-ring drain has not handed over (`lqos-74t.2.x`
            // Q6): once the first ring batch is acked the snapshot export is
            // DISABLED deterministically (no poll-to-poll race / double export).
            if completion_consumer::snapshot_export_enabled() {
                // 3a netflow: hand the completion events to the exporter's bounded
                // queue (a no-op when the exporter is not configured).
                crate::netflow::queue_completed_flows(&next.recent_flows);
                // Carve-out sweep #1: the LTS2 (Insight) per-flow export — the ONE
                // completion-gated consumer over the same recent rows (re-homes the
                // retired kernel finished-flow producer onto the flow_history source).
                lts2_flow_export::export_recent_flows(&next.recent_flows);
            }
            let active_flows = next.active_flows;
            CACHE.store(Arc::new(BridgeStatsSnapshot {
                by_circuit_id: joined.clone(),
                last_good_circuit_ids: last_good_ids.clone(),
                stale: false,
                captured_at_millis: captured,
                host_rows,
                by_ip,
                best_effort_by_ip,
                active_flow_ips,
                bridge_actual_bytes_per_second,
                bridge_shaped_bytes_per_second,
                bridge_mapped_bytes_per_second,
                bridge_unmapped_bytes_per_second,
                active_flows,
                flow_untracked_packets: next.diagnostics.flow_untracked_packets,
                flow_bytes_per_second_directional,
                protocol_packets_per_second,
            }));
            // 3a A1: ONE bulk bridge qdisc read → the LTS `circuit_cake_*` store
            // + B2's queue counts (shared source; `queue_stats_stale` reflects it).
            refresh_queue_stats(socket);
            report_bridge_liveness(&joined);
            // F3-B A1 (`docs/executive_rtt_rollup_card.md` §4c): restore the
            // tracker→tree RTT accumulation (the snapshot was taken above, before
            // the QoO fold — lock order — then clear+fold under the tree write),
            // and repoint the view's `rtts` to the tracker `median_latency()` source.
            apply_tree_rtt_accumulation(&rtt_snapshot);
            publish_tree_live_view(
                &joined,
                &parent_by_circuit,
                captured,
                false,
                &rtt_snapshot.median_ms_by_circuit,
            );
        }
        Err(error) => {
            debug!("Bridge analytics poll failed ({error:?}); serving last-good as stale");
            let mut current = (**CACHE.load()).clone();
            current.stale = true;
            CACHE.store(Arc::new(current));
            mark_tree_live_view_stale();
            // 3a A1: a failed poll means the bridge qdisc source is unavailable.
            set_queue_stats_stale(true);
        }
    }
}

/// Converts a cumulative-counter delta over `elapsed_ms` into BYTES/second.
///
/// 3b correction: this used to multiply by 8 and feed a
/// bytes-named field, so the LTS spike guard (which multiplies by 8 again)
/// fired at ~1/8 link and `to_bits_from_bytes` (×8) made the sparkline read
/// 8×. Bytes/second is the one unit every consumer agrees on.
fn interval_bytes_per_second(now: u64, before: u64, elapsed_ms: u64) -> u64 {
    if elapsed_ms == 0 {
        return 0;
    }
    ((now.saturating_sub(before)) as u128 * 1_000 / elapsed_ms as u128) as u64
}

/// `.66.2`: the per-direction `download_bytes` of an optional class counter (0
/// when the field is absent — legacy mode / an older bridge).
fn counter_down(counters: Option<&analytics_ipc::DirectionalCounters>) -> u64 {
    counters.map_or(0, |c| c.download_bytes)
}

/// `.66.2`: the per-direction `upload_bytes` of an optional class counter.
fn counter_up(counters: Option<&analytics_ipc::DirectionalCounters>) -> u64 {
    counters.map_or(0, |c| c.upload_bytes)
}

/// `.66.2`: one class's (mapped or unmapped) down/up interval rate from the
/// cumulative `.66.1` counters and the previous seed. An absent counter diffs
/// from zero; a counter reset saturates to 0 (never under-counts).
fn interval_rate(
    now: Option<&analytics_ipc::DirectionalCounters>,
    before_down: u64,
    before_up: u64,
    elapsed_ms: u64,
) -> DownUpOrder<u64> {
    DownUpOrder::new(
        interval_bytes_per_second(counter_down(now), before_down, elapsed_ms),
        interval_bytes_per_second(counter_up(now), before_up, elapsed_ms),
    )
}

fn build_by_ip_rates(
    deltas: &FxHashMap<HostKey, translate::HostDelta>,
    hosts: &[analytics_ipc::HostRow],
) -> FxHashMap<IpAddr, IpLiveRow> {
    let mut by_ip: FxHashMap<IpAddr, IpLiveRow> = FxHashMap::default();
    for (key, delta) in deltas {
        let entry = by_ip.entry(key.1).or_default();
        entry.bytes_per_second.down += delta.bytes_per_second.down;
        entry.bytes_per_second.up += delta.bytes_per_second.up;
        entry.shaped_bytes_per_second.down += delta.shaped_bytes_per_second.down;
        entry.shaped_bytes_per_second.up += delta.shaped_bytes_per_second.up;
        entry.packets_per_second.down += delta.enqueued_packets_per_second.down;
        entry.packets_per_second.up += delta.enqueued_packets_per_second.up;
        entry.packets_sent_per_second.down += delta.sent_packets_per_second.down;
        entry.packets_sent_per_second.up += delta.sent_packets_per_second.up;
        entry.tcp_packets += delta.tcp_packets;
        entry.tcp_retransmits += delta.tcp_retransmits;
    }
    // K2 slice 1: per-IP RTT p50 (nanos) from the host rows' raw samples.
    let mut samples: FxHashMap<IpAddr, Vec<u32>> = FxHashMap::default();
    for row in hosts {
        if !row.rtt_samples_us.is_empty() {
            samples
                .entry(row.id.subscriber_ip)
                .or_default()
                .extend_from_slice(&row.rtt_samples_us);
        }
    }
    for (ip, samples) in samples {
        if let Some(p50) = translate::percentile_nanos(&samples) {
            by_ip.entry(ip).or_default().rtt_current_p50_nanos = Some(p50);
        }
    }
    by_ip
}

/// `.49.1`: build the SEPARATE best-effort host map from the wire
/// `best_effort_hosts` rows (the unmapped class has NO flow deltas, so the rate
/// fields stay 0; the reader's contract is the host identity + the
/// `last_seen_nanos` age (300 s window backstop) + the sent/enqueued counts).
/// Never touches the shaped `by_ip`.
fn build_best_effort_by_ip(
    hosts: &[analytics_ipc::HostRow],
    captured_at_millis: u64,
) -> FxHashMap<IpAddr, IpLiveRow> {
    let mut out: FxHashMap<IpAddr, IpLiveRow> = FxHashMap::default();
    for row in hosts {
        let entry = out.entry(row.id.subscriber_ip).or_default();
        entry.last_seen_nanos = translate::age_nanos(captured_at_millis, row);
        // `.49.1` REQUIRED: populate BYTES so `.49.2` does not render 0 —
        // the wire `HostRow.actual` carries the cumulative per-direction bytes
        // (the class has no flow deltas, so the rate series stays 0).
        entry.total_bytes = DownUpOrder {
            down: row.actual.download_bytes,
            up: row.actual.upload_bytes,
        };
    }
    out
}

/// Dynamic-ranges §7 (SIGNAL): for each configured range, report per-IP
/// activity for in-range hosts the catalog cannot resolve — the unconfirmed
/// leaf's signature (the range leaf is transaction-stage and absent from the
/// catalog, so it is the only catalog-unresolved leaf carrying in-range
/// traffic). The actor owns the sustained-activity window and the write.
fn report_range_activity(by_ip: &FxHashMap<IpAddr, IpLiveRow>) {
    if by_ip.is_empty() {
        return;
    }
    let Ok(config) = lqos_config::load_config() else {
        return;
    };
    let Some(dynamic_cfg) = config.dynamic_circuits.as_ref() else {
        return;
    };
    if !dynamic_cfg.enabled
        || !dynamic_cfg.enable_unknown_ip_promotion
        || dynamic_cfg.ranges.is_empty()
    {
        return;
    }
    let catalog = lqos_network_devices::network_devices_catalog();
    for rule in &dynamic_cfg.ranges {
        let mut samples = Vec::new();
        for (ip, row) in by_ip {
            if !rule.ip_range.contains(*ip) {
                continue;
            }
            if catalog
                .device_longest_match_for_ip(&lqos_utils::XdpIpAddress::from_ip(*ip))
                .is_some()
            {
                continue;
            }
            let bytes = row
                .bytes_per_second
                .down
                .saturating_add(row.bytes_per_second.up);
            samples.push((*ip, bytes, row.tcp_packets));
        }
        if !samples.is_empty() {
            lqos_network_devices::report_range_host_activity(rule.name.trim(), &samples);
        }
    }
}

/// T2b: rolls the E2 circuit rows up the store's parents chains into a
/// fresh per-node live-view (one forward pass — the chains are
/// self-inclusive below root, so every index on the chain accumulates).
/// Circuit ids matching no store node are counted (rate-limited WARN —
/// at most one line per [`UNMATCHED_WARN_MIN_INTERVAL_SECS`] while the
/// mismatch persists; the state resets when it clears so the NEXT
/// incident warns immediately).
/// F3-B A1 (`docs/executive_rtt_rollup_card.md` §4c): the per-cycle snapshot of
/// the LIVE tracker RTT source. The v2 `node.rtt_buffer` accumulation died in v3
/// (`add_rtt_buffer_cycle` had no caller), so `node.rtt_buffer` was empty and the
/// E2 overlay substituted the bridge per-IP p50 concat (wrong source +
/// median-of-children). This re-points the tree to the source the CIRCUIT VIEW
/// reports: `recent_rtt_data` via `median_latency()`.
struct TrackerRttSnapshot {
    /// `(node chain, download samples nanos, upload samples nanos)` per
    /// qualified circuit — PER DIRECTION (`§.40.2`): the tree fold routes each
    /// to its own direction.
    folds: Vec<(Vec<usize>, Vec<u64>, Vec<u64>)>,
    /// `circuit_id -> merged median ms` — the circuit-view source for LTS2
    /// (one scalar; the direction split does not change it).
    median_ms_by_circuit: FxHashMap<String, f32>,
}

/// §`lqos-74t.40.1` (LOCK ORDER): the bridge heatmap snapshot is already
/// published (an `ArcSwap` load, no lock); the name→index/node-chain lookup
/// takes a tree READ that is released before `apply_tree_rtt_accumulation`
/// takes the tree WRITE. The dead `THROUGHPUT_TRACKER` is no longer read — it
/// has no production writer post-K3. Gated by the circuit-view rule (>=2
/// samples; the window's sub-1 Mb long-poll reject).
fn snapshot_bridge_rtt(
    heatmap: &circuit_heatmap::CircuitHeatmapSnapshot,
    parent_by_circuit: &FxHashMap<String, String>,
) -> TrackerRttSnapshot {
    let nodes = lqos_network_devices::with_network_json_read(|net_json| {
        net_json.get_nodes_when_ready().clone()
    });
    let index_by_name: FxHashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| (node.name.as_str(), idx))
        .collect();
    let mut folds = Vec::new();
    let mut median_ms_by_circuit = FxHashMap::default();
    for (circuit_id, samples) in &heatmap.rtt_samples_by_circuit {
        let sample_count = samples.down.len() + samples.up.len();
        if sample_count < 2 {
            continue;
        }
        // The circuit-view sub-1 Mb long-poll reject, over the window's bytes.
        let window_bytes: u64 = heatmap
            .by_circuit
            .get(circuit_id)
            .map(|minutes| {
                minutes
                    .iter()
                    .map(|minute| minute.down_bytes.saturating_add(minute.up_bytes))
                    .sum()
            })
            .unwrap_or(0);
        if window_bytes < 1_000_000 {
            continue;
        }
        let Some(parent_node) = parent_by_circuit.get(circuit_id) else {
            continue;
        };
        let Some(&node_idx) = index_by_name.get(parent_node.as_str()) else {
            continue;
        };
        // §lqos-74t.40 pin 4: an umbrella-attributed circuit is a non-entity.
        if nodes[node_idx].id.as_deref() == Some(lqos_dataplane_types::ORPHANS_UMBRELLA_NODE_ID) {
            continue;
        }
        // §`lqos-74t.40.2`: keep the directions SEPARATE — download samples feed
        // the Download consumers, upload samples the Upload (v2 scored
        // per-direction; the merged pool collapsed `QoOd == QoOu`).
        let down_nanos: Vec<u64> = samples
            .down
            .iter()
            .map(|us| u64::from(*us) * 1_000)
            .collect();
        let up_nanos: Vec<u64> = samples
            .up
            .iter()
            .map(|us| u64::from(*us) * 1_000)
            .collect();
        let merged: Vec<u64> = down_nanos
            .iter()
            .chain(up_nanos.iter())
            .copied()
            .collect();
        if let Some(median) = median_nanos_ms(&merged) {
            median_ms_by_circuit.insert(circuit_id.clone(), median);
        }
        folds.push((nodes[node_idx].parents.clone(), down_nanos, up_nanos));
    }
    TrackerRttSnapshot {
        folds,
        median_ms_by_circuit,
    }
}

/// The median of a nanosecond sample set in milliseconds (`None` when empty).
fn median_nanos_ms(nanos: &[u64]) -> Option<f32> {
    if nanos.is_empty() {
        return None;
    }
    let mut sorted: Vec<u64> = nanos.to_vec();
    sorted.sort_unstable();
    Some(sorted[sorted.len() / 2] as f32 / 1_000_000.0)
}

/// HAZARD 2 (DIRECTION) + CLEAR: clear every node's `rtt_buffer` (rotate the
/// `Current` window — `accumulate` does NOT), then fold each qualified circuit's
/// filtered samples into its node chain via `add_rtt_buffer_cycle`. EACH sample
/// is pushed into BOTH directions so the per-direction percentile equals the
/// circuit view's pooled p50.
fn apply_tree_rtt_accumulation(snapshot: &TrackerRttSnapshot) {
    lqos_network_devices::with_network_json_write(|tree| {
        // §lqos-74t.40 pin 4: the synthetic `orphans` umbrella is a non-entity —
        // its directly-attributed circuits must NOT fold their RTT into the
        // umbrella's ancestors (notably the root). Look the index up before the
        // clear (borrow ends), then skip a fold whose self index is the umbrella.
        let umbrella_idx = tree.get_nodes_when_ready().iter().position(|node| {
            node.id.as_deref() == Some(lqos_dataplane_types::ORPHANS_UMBRELLA_NODE_ID)
        });
        tree.clear_rtt_buffers();
        for (parents, down_samples, up_samples) in &snapshot.folds {
            if umbrella_idx.is_some_and(|idx| parents.last() == Some(&idx)) {
                continue;
            }
            // §`lqos-74t.40.2`: route each direction to ITS OWN buffer so the
            // site RTT down/up percentiles read their own distributions.
            let mut buffer = RttBuffer::default();
            for nanos in down_samples {
                buffer.push(
                    RttData::from_nanos(*nanos),
                    FlowbeeEffectiveDirection::Download,
                    0,
                );
            }
            for nanos in up_samples {
                buffer.push(
                    RttData::from_nanos(*nanos),
                    FlowbeeEffectiveDirection::Upload,
                    0,
                );
            }
            tree.add_rtt_buffer_cycle(parents, &buffer);
        }
    });
}

fn publish_tree_live_view(
    joined: &FxHashMap<String, CircuitLiveRow>,
    parent_by_circuit: &FxHashMap<String, String>,
    captured: u64,
    stale: bool,
    median_ms_by_circuit: &FxHashMap<String, f32>,
) {
    let view = build_tree_live_view(
        joined,
        parent_by_circuit,
        captured,
        stale,
        median_ms_by_circuit,
    );
    let unmatched = view.unmatched_circuits;
    TREE_LIVE_VIEW.store(Arc::new(Some(Arc::new(view))));
    let now = now_unix_estimate();
    let mut state = UNMATCHED_WARN_STATE.lock();
    if unmatched_warn_due(now, unmatched, &mut state) {
        warn!(
            "Tree live-view: {unmatched} circuit id(s) matched no store node this tick (zero contribution)"
        );
    }
}

/// Minimum seconds between unmatched-circuit WARN lines while a
/// mismatch persists (the analytics tick is 1s — a standing mismatch
/// must not spam the log).
const UNMATCHED_WARN_MIN_INTERVAL_SECS: u64 = 60;

/// Cold-path rate-limit state for the unmatched-circuit WARN.
#[derive(Default)]
struct UnmatchedWarnState {
    last_warn_unix: Option<u64>,
}

static UNMATCHED_WARN_STATE: Lazy<parking_lot::Mutex<UnmatchedWarnState>> =
    Lazy::new(|| parking_lot::Mutex::new(UnmatchedWarnState::default()));

/// True when the unmatched-circuit WARN should fire NOW: the first
/// sighting, or ≥ [`UNMATCHED_WARN_MIN_INTERVAL_SECS`] since the last
/// line while the mismatch persists. `count == 0` resets the state
/// (silently) so a fresh incident warns immediately. `now_unix == 0`
/// (clock unavailable) is always due and records nothing — today's
/// every-tick behavior; a missing clock is the louder problem. PURE —
/// unit-gated.
fn unmatched_warn_due(now_unix: u64, count: usize, state: &mut UnmatchedWarnState) -> bool {
    if count == 0 {
        *state = UnmatchedWarnState::default();
        return false;
    }
    if now_unix == 0 {
        return true;
    }
    let due = state
        .last_warn_unix
        .is_none_or(|last| now_unix.saturating_sub(last) >= UNMATCHED_WARN_MIN_INTERVAL_SECS);
    if due {
        state.last_warn_unix = Some(now_unix);
    }
    due
}

/// Wall-clock seconds, best-effort: 0 when the clock is unavailable
/// (the rate limiter then just warns on its own cadence — every tick
/// — which is today's behavior; clock loss is the louder problem).
fn now_unix_estimate() -> u64 {
    lqos_utils::unix_time::unix_now().unwrap_or(0)
}

fn build_tree_live_view(
    joined: &FxHashMap<String, CircuitLiveRow>,
    parent_by_circuit: &FxHashMap<String, String>,
    captured: u64,
    stale: bool,
    median_ms_by_circuit: &FxHashMap<String, f32>,
) -> TreeLiveView {
    let mut view = TreeLiveView {
        stale,
        captured_at_millis: captured,
        ..Default::default()
    };
    lqos_network_devices::with_network_json_read(|net_json| {
        let nodes = net_json.get_nodes_when_ready();
        let index_by_name: FxHashMap<&str, usize> = nodes
            .iter()
            .enumerate()
            .map(|(idx, node)| (node.name.as_str(), idx))
            .collect();
        for (circuit_id, row) in joined {
            // 3b DEFECT 1: attribute the circuit to its catalog PARENT NODE,
            // then that node's self-inclusive chain — never the circuit id.
            let Some(parent_node) = parent_by_circuit.get(circuit_id.as_str()) else {
                view.unmatched_circuits += 1;
                continue;
            };
            let Some(&node_idx) = index_by_name.get(parent_node.as_str()) else {
                view.unmatched_circuits += 1;
                continue;
            };
            // The chain includes the circuit's own index (self-inclusive
            // below root) — one pass covers the node and every ancestor.
            for &idx in &nodes[node_idx].parents {
                let entry = view
                    .current_throughput
                    .entry(idx)
                    .or_insert_with(DownUpOrder::zeroed);
                entry.down += row.shaped_bytes_per_second.down;
                entry.up += row.shaped_bytes_per_second.up;
                // K2 slice 1: RTT p50 (ms) + circuits-with-samples counts.
                // F3-B A1: the per-node RTT list is the CIRCUIT VIEW's source
                // (the tracker `median_latency()`), NOT the bridge per-IP p50
                // (wrong source + median-of-children). `median_latency()` is
                // direction-agnostic, so it counts both directions. The tree
                // `rtts` from `node.rtt_buffer` is the aggregate; this list is
                // for the LTS2 SiteRtt.
                let flows = view.current_rtt_flows.entry(idx).or_default();
                if let Some(median_ms) = median_ms_by_circuit.get(circuit_id.as_str()) {
                    view.rtts.entry(idx).or_default().push(*median_ms);
                    flows.0 += 1;
                    flows.1 += 1;
                }
                // K2 slice 2 729b: pool the circuit's locally-bucketed
                // histogram into the node.
                let histogram = view.rtt_histograms.entry(idx).or_default();
                for (bucket, count) in row.rtt_histogram.0.iter().enumerate() {
                    histogram.0[bucket] = histogram.0[bucket].saturating_add(*count);
                }
                let retransmits = view.current_retransmits.entry(idx).or_default();
                retransmits.0 += row.tcp_retransmit_sample.down.retransmits.get();
                retransmits.1 += row.tcp_retransmit_sample.up.retransmits.get();
                let packets = view.current_tcp_retransmit_packets.entry(idx).or_default();
                packets.0 += row.tcp_retransmit_sample.down.packets.get();
                packets.1 += row.tcp_retransmit_sample.up.packets.get();
            }
        }
    });
    // K2 slice 2 729b: derive the per-node passive p90 from the pooled
    // histogram (the pure compute shared via lqos_utils).
    for (idx, histogram) in &view.rtt_histograms {
        if let Some(p90) = lqos_utils::rtt::histogram_p90_ms(&histogram.0) {
            view.rtt_p90_ms.insert(*idx, p90 as f32);
        }
    }
    view
}

fn mark_tree_live_view_stale() {
    let current = TREE_LIVE_VIEW.load_full();
    let Some(view) = current.as_ref() else {
        return;
    };
    if view.stale {
        return;
    }
    let mut stale_view = (**view).clone();
    stale_view.stale = true;
    TREE_LIVE_VIEW.store(Arc::new(Some(Arc::new(stale_view))));
}

/// Joins host rows to circuit-id strings through the network-devices
/// catalog (longest-prefix match by subscriber IP — the same join the
/// kernel rollup performs). Unresolvable rows are skipped.
fn join_with_catalog(
    next: &analytics_ipc::AnalyticsSnapshot,
    deltas: &FxHashMap<HostKey, translate::HostDelta>,
) -> FxHashMap<String, CircuitLiveRow> {
    let catalog = lqos_network_devices::network_devices_catalog();
    join_by_circuit(next, deltas, |ip| {
        let key = XdpIpAddress::from(*ip);
        catalog
            .device_longest_match_for_ip(&key)
            .map(|(_, device)| device.circuit_id.as_str())
            .filter(|id| !id.trim().is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use analytics_ipc::{
        AnalyticsSnapshot, DirectionalCounters, DirectionalPackets, HostId, HostRow,
        SnapshotRequest, SnapshotService, health_channel, snapshot_channel, state_channel,
    };
    use std::net::IpAddr;
    use std::sync::mpsc::Receiver;
    use std::thread;

 /// R4 /: serialize the env-mutating tests.
    static ENV_SOCKET_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// R4 (1): with `[dataplane_bridge]` ABSENT and `LIBREQOS_ANALYTICS_SOCKET`
    /// set, the helper returns `Some` (so the broker's adopt RUNS). The
    /// section-only revert returns `None` ⇒ FAILS this (non-vacuous).
    #[test]
    fn analytics_socket_path_if_active_is_env_aware() {
        let _g = ENV_SOCKET_LOCK.lock().unwrap();
        let mut config = lqos_config::Config::default();
        config.dataplane_bridge = None;
        // neither section nor env => None
        unsafe {
            std::env::remove_var("LIBREQOS_ANALYTICS_SOCKET");
        }
        assert_eq!(
            analytics_socket_path_if_active(&config),
            None,
            "neither section nor env => None"
        );
        // env-only => Some (the broker's formerly-missing case)
        unsafe {
            std::env::set_var("LIBREQOS_ANALYTICS_SOCKET", "/dev/shm/r4-probe.sock");
        }
        assert_eq!(
            analytics_socket_path_if_active(&config),
            Some(PathBuf::from("/dev/shm/r4-probe.sock")),
            "env-only => Some (the adopt can run; a section-only gate FAILS here)"
        );
        // section-present (env unset) => Some(config); env wins when set
        unsafe {
            std::env::remove_var("LIBREQOS_ANALYTICS_SOCKET");
        }
        let mut dbb = lqos_config::DataplaneBridgeConfig::default();
        dbb.analytics_socket = "/run/r4-cfg.sock".to_string();
        config.dataplane_bridge = Some(dbb);
        assert_eq!(
            analytics_socket_path_if_active(&config),
            Some(PathBuf::from("/run/r4-cfg.sock")),
            "section-present => the configured path"
        );
        unsafe {
            std::env::set_var("LIBREQOS_ANALYTICS_SOCKET", "/dev/shm/r4-win.sock");
        }
        assert_eq!(
            analytics_socket_path_if_active(&config),
            Some(PathBuf::from("/dev/shm/r4-win.sock")),
            "the env wins over the config"
        );
        unsafe {
            std::env::remove_var("LIBREQOS_ANALYTICS_SOCKET");
        }
    }

    /// R4 (2) PARITY: the one owner resolves the SAME `Option<PathBuf>` the
    /// broker, the analytics actor, and the supervisor consume, across
    /// section-present / env-only / neither.
    #[test]
    fn analytics_socket_path_if_active_parity() {
        let _g = ENV_SOCKET_LOCK.lock().unwrap();
        let mut config = lqos_config::Config::default();
        let mut dbb = lqos_config::DataplaneBridgeConfig::default();
        dbb.analytics_socket = "/run/parity.sock".to_string();
        // section-present
        config.dataplane_bridge = Some(dbb.clone());
        unsafe {
            std::env::remove_var("LIBREQOS_ANALYTICS_SOCKET");
        }
        assert_eq!(
            analytics_socket_path_if_active(&config),
            Some(analytics_socket_path(&dbb)),
            "section-present: the shared owner == the raw resolved path"
        );
        // env-only
        config.dataplane_bridge = None;
        unsafe {
            std::env::set_var("LIBREQOS_ANALYTICS_SOCKET", "/dev/shm/parity-env.sock");
        }
        assert_eq!(
            analytics_socket_path_if_active(&config),
            Some(PathBuf::from("/dev/shm/parity-env.sock")),
            "env-only: the same env path for all three callers"
        );
        // neither
        unsafe {
            std::env::remove_var("LIBREQOS_ANALYTICS_SOCKET");
        }
        assert_eq!(
            analytics_socket_path_if_active(&config),
            None,
            "neither: all three inert"
        );
    }

    /// The retired COMBINED `flow_bytes_per_second` map is GONE: this returns
    /// the DIRECTIONAL split only. Each direction's floor rate comes from ITS
    /// OWN byte delta (no loss / no double-count), and `per_second` is
    /// non-additive — the old combined slot would read one bps higher here, so
    /// the oracle is stated on BYTES, never on the displayed bps.
    #[test]
    fn compute_flow_rates_splits_each_byte_into_one_direction() {
        let id = analytics_ipc::FlowId {
            circuit_id: 1,
            flow_hash: 7,
            subscriber_ip: "192.0.2.10".parse().expect("test subscriber IP"),
            remote_ip: "203.0.113.9".parse().expect("test remote IP"),
            subscriber_port: 1234,
            remote_port: 443,
            protocol: 6,
        };
        let flow = |shaped_down: u64, shaped_up: u64| analytics_ipc::FlowRow {
            id: id.clone(),
            shaped: analytics_ipc::DirectionalCounters {
                download_bytes: shaped_down,
                upload_bytes: shaped_up,
            },
            ..analytics_ipc::FlowRow::default()
        };
        let mut seeds: FxHashMap<analytics_ipc::FlowId, FlowSeed> = FxHashMap::default();
        // First poll: seed only (first sight yields no rate).
        let (directional, _protocol) = compute_flow_rates(&mut seeds, &[flow(0, 0)], 1_500);
        assert!(directional.is_empty(), "first sight yields no rate");
        // Second poll: +1,000 B down / +1,000 B up over a 1,500 ms window.
        let (directional, _protocol) =
            compute_flow_rates(&mut seeds, &[flow(1_000, 1_000)], 1_500);
        let split = directional.get(&id).copied().expect("a rate for the flow");
        assert_eq!(split.down, per_second(1_000, 1_500), "down from its own delta");
        assert_eq!(split.up, per_second(1_000, 1_500), "up from its own delta");
        // Floor division is non-additive: down+up (1,332) != the old combined
        // per_second(2,000, 1,500) (1,333). This is WHY the oracle is BYTES.
        assert_eq!(split.down + split.up, 1_332);
        assert_eq!(per_second(2_000, 1_500), 1_333);
        assert_ne!(split.down + split.up, per_second(2_000, 1_500));
    }

    /// Serializes tests that touch the process-global bridge-analytics state
    /// (CACHE here; TREE_LIVE_VIEW + the network store in `tree_live_tests`).
    pub(super) static TEST_LOCK: Lazy<std::sync::Mutex<()>> =
        Lazy::new(|| std::sync::Mutex::new(()));

    fn host_row(circuit: u64, ip: &str, down: u64, up: u64) -> HostRow {
        HostRow {
            id: HostId {
                circuit_id: circuit,
                subscriber_ip: ip.parse::<IpAddr>().expect("test invariant"),
            },
            actual: DirectionalCounters {
                download_bytes: down,
                upload_bytes: up,
            },
            shaped: DirectionalCounters::default(),
            enqueued_packets: DirectionalPackets::default(),
            sent_packets: DirectionalPackets::default(),
            rtt_samples_us: Vec::new(),
            dns_samples_us: Vec::new(),
            tcp_sent_packets: 0,
            tcp_retransmits: 0,
            last_seen_millis: 0,
            ..HostRow::default()
        }
    }

    fn snapshot_at(millis: u64, rows: Vec<HostRow>) -> AnalyticsSnapshot {
        AnalyticsSnapshot {
            captured_at_millis: millis,
            hosts: rows,
            ..Default::default()
        }
    }

    /// Feeds fixed snapshots per request (the collector side of the
    /// analytics socket). Exits after the last reply, dropping the
    /// receiver: the next request then fails with CollectorUnavailable.
    fn feed(rx: Receiver<SnapshotRequest>, snapshots: Vec<AnalyticsSnapshot>) {
        for snapshot in snapshots {
            match rx.recv() {
                Ok(request) => {
                    let _ = request.reply.try_send(snapshot);
                }
                Err(_) => return,
            }
        }
    }

    /// 110-circuit resolver: the last IPv4 octet is the numeric circuit id.
    fn resolve_110(ip: &IpAddr) -> Option<String> {
        let last = ip.to_string().rsplit('.').next()?.parse::<u32>().ok()?;
        Some(format!("c{last}"))
    }

    /// 110-circuit join: the last IPv4 octet is the numeric circuit id.
    fn join_110(
        next: &AnalyticsSnapshot,
        deltas: &FxHashMap<HostKey, translate::HostDelta>,
    ) -> FxHashMap<String, CircuitLiveRow> {
        join_by_circuit(next, deltas, |ip| {
            let last = ip.to_string().rsplit('.').next()?.parse::<u32>().ok()?;
            let leaked: &'static str = Box::leak(format!("c{last}").into_boxed_str());
            Some(leaked)
        })
    }

    /// Identity join for tests: every row joins to circuit "c1".
    fn join_fixed(
        next: &AnalyticsSnapshot,
        deltas: &FxHashMap<HostKey, translate::HostDelta>,
    ) -> FxHashMap<String, CircuitLiveRow> {
        join_by_circuit(next, deltas, |_| Some("c1"))
    }

    /// An analytics service on a unique temp socket serving the given
    /// snapshot sequence. The service thread serves forever (test
    /// lifetime); the feed thread replies then exits.
    struct TestService {
        socket: PathBuf,
        dir_unique: String,
    }

    impl Drop for TestService {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.socket);
            let _ = std::fs::remove_dir_all(std::env::temp_dir().join(&self.dir_unique));
        }
    }

    fn spawn_service(responses: Vec<AnalyticsSnapshot>) -> TestService {
        let unique = format!(
            "lqosd-bridge-analytics-test-{}-{:?}",
            std::process::id(),
            thread::current().id()
        );
        let dir = std::env::temp_dir().join(&unique);
        std::fs::create_dir_all(&dir).expect("test dir");
        let socket = dir.join("analytics.sock");
        let (requester, rx) = snapshot_channel(Duration::from_secs(2));
        let (health_requester, _hrx) = health_channel(Duration::from_secs(2));
        let (state_requester, _srx) = state_channel(Duration::from_secs(2));
        let service = SnapshotService::bind(&socket, requester, health_requester, state_requester)
            .expect("bind service");
        thread::spawn(move || {
            let _ = service.serve();
        });
        thread::spawn(move || feed(rx, responses));
        TestService {
            socket,
            dir_unique: unique,
        }
    }

    fn reset_cache() {
        CACHE.store(Arc::new(BridgeStatsSnapshot::default()));
    }

    #[test]
    fn full_pipe_polls_rates_and_marks_stale_on_absence() {
        let _guard = TEST_LOCK.lock().expect("test invariant");
        reset_cache();
        // Two successful polls (1s apart, +1000 bytes down), then the
        // feeds are exhausted (collector drops) → failure path.
        let service = spawn_service(vec![
            snapshot_at(1_000, vec![host_row(1, "192.0.2.10", 1_000, 500)]),
            snapshot_at(2_000, vec![host_row(1, "192.0.2.10", 2_000, 1_500)]),
        ]);
        let mut seeds = FxHashMap::default();
        let mut prev = None;
        let mut totals = None;
        let mut history = flow_history::FlowHistory::new();
        let mut heatmap = circuit_heatmap::CircuitHeatmapAccumulator::new();
        let mut qoo = circuit_qoo::CircuitQooAccumulator::new();
        let mut flow_seeds = FxHashMap::default();
        let mut rate_memory = CircuitRateMemory::default();
        poll_and_store_with(
            &service.socket,
            &mut seeds,
            &mut prev,
            &mut totals,
            &mut history,
            &mut heatmap,
            &mut qoo,
            &mut flow_seeds,
            &mut rate_memory,
            |_| Some("c1".to_string()),
            join_fixed,
        );
        {
            let cache = CACHE.load();
            assert!(!cache.stale);
            assert!(
                cache.by_circuit_id.is_empty(),
                "first poll seeds only (no rate yet)"
            );
            assert_eq!(cache.host_rows, 1);
        }
        poll_and_store_with(
            &service.socket,
            &mut seeds,
            &mut prev,
            &mut totals,
            &mut history,
            &mut heatmap,
            &mut qoo,
            &mut flow_seeds,
            &mut rate_memory,
            |_| Some("c1".to_string()),
            join_fixed,
        );
        {
            let cache = CACHE.load();
            assert!(!cache.stale);
            let row = cache.by_circuit_id.get("c1").expect("circuit joined");
            assert_eq!(row.bytes_per_second.down, 1_000);
            assert_eq!(row.bytes_per_second.up, 1_000);
        }
        // Cluster-1 increment 2: the fold populated the heatmap accumulator.
        let heatmap_before_stale = heatmap.publish();
        assert!(heatmap_before_stale.by_circuit.contains_key("c1"));
        assert_eq!(heatmap_before_stale.captured_at_millis, 2_000);
        // Third poll: feeds exhausted → stale last-good (never zeros).
        poll_and_store_with(
            &service.socket,
            &mut seeds,
            &mut prev,
            &mut totals,
            &mut history,
            &mut heatmap,
            &mut qoo,
            &mut flow_seeds,
            &mut rate_memory,
            |_| Some("c1".to_string()),
            join_fixed,
        );
        {
            let cache = CACHE.load();
            assert!(cache.stale, "absence marks stale");
            let row = cache.by_circuit_id.get("c1").expect("last-good preserved");
            assert_eq!(
                row.bytes_per_second.down, 1_000,
                "stale serves LAST-GOOD, never zeros"
            );
        }
        assert_eq!(
            heatmap.publish().captured_at_millis,
            2_000,
            "the stale path folds nothing (last-good heatmap preserved)"
        );
        reset_cache();
    }

    /// `lqos-74t.117`: a counter reset (retention eviction) and a missing
    /// host row must NOT drop/zero a continuously transmitting circuit. The
    /// frass-shaped A/B continuity: 110 circuits alternate between a
    /// counter reset (CAKE->FIFO rebuild) and a missing host row
    /// (FIFO->CAKE rebuild). The per-circuit live sample must remain present
    /// with a non-zero last-good rate on every poll after the first interval —
    /// no missing/zero graph gap.
    #[test]
    fn frass_shaped_110_circuits_flip_continuity_has_no_gap() {
        let _guard = TEST_LOCK.lock().expect("test invariant");
        reset_cache();
        let rows = |millis: u64, base: u64, flip: Option<bool>| {
            let mut out = Vec::new();
            for n in 1..=110u64 {
                let reset = flip.is_some_and(|even_reset| (n % 2 == 0) == even_reset);
                if flip.is_some() && reset {
                    // Counter reset for half the circuits.
                    out.push(host_row(n, &format!("192.0.2.{n}"), 100, 50));
                } else if flip.is_some() {
                    // Missing host row for the other half.
                } else {
                    out.push(host_row(n, &format!("192.0.2.{n}"), base + n, base / 2 + n));
                }
            }
            snapshot_at(millis, out)
        };
        let service = spawn_service(vec![
            rows(1_000, 1_000, None),
            rows(2_000, 2_000, None),
            rows(3_000, 0, Some(true)),  // even reset, odd missing
            rows(4_000, 4_000, None),
            rows(5_000, 0, Some(false)), // odd reset, even missing
            rows(6_000, 6_000, None),
        ]);
        let mut seeds = FxHashMap::default();
        let mut prev = None;
        let mut totals = None;
        let mut history = flow_history::FlowHistory::new();
        let mut heatmap = circuit_heatmap::CircuitHeatmapAccumulator::new();
        let mut qoo = circuit_qoo::CircuitQooAccumulator::new();
        let mut flow_seeds = FxHashMap::default();
        let mut rate_memory = CircuitRateMemory::default();
        for poll in 0..6 {
            poll_and_store_with(
                &service.socket,
                &mut seeds,
                &mut prev,
                &mut totals,
                &mut history,
                &mut heatmap,
                &mut qoo,
                &mut flow_seeds,
                &mut rate_memory,
                resolve_110,
                join_110,
            );
            let cache = CACHE.load();
            if poll >= 1 {
                assert_eq!(
                    cache.by_circuit_id.len(),
                    110,
                    "poll {poll}: no circuit may disappear on a flip"
                );
                if poll == 2 || poll == 4 {
                    assert!(
                        !cache.last_good_circuit_ids.is_empty(),
                        "poll {poll}: a flip must be surfaced as last-good, not a gap"
                    );
                }
                for (circuit, row) in cache.by_circuit_id.iter() {
                    assert!(
                        row.bytes_per_second.down > 0 && row.bytes_per_second.up > 0,
                        "poll {poll}: {circuit} must carry a last-good rate, not a zero gap"
                    );
                }
            }
        }
        reset_cache();
    }

    /// last interval delta is carried for a reset, the last-good row is
    /// carried for a missing row, and both are marked in
    /// `last_good_circuit_ids` (stale/last-good, never a silent gap).
    #[test]
    fn counter_reset_and_missing_host_row_serve_last_good() {
        let _guard = TEST_LOCK.lock().expect("test invariant");
        reset_cache();
        let service = spawn_service(vec![
            snapshot_at(1_000, vec![host_row(1, "192.0.2.10", 1_000, 500)]),
            snapshot_at(2_000, vec![host_row(1, "192.0.2.10", 2_000, 1_500)]),
            // Counter reset: both cumulative counters drop.
            snapshot_at(3_000, vec![host_row(1, "192.0.2.10", 100, 50)]),
            // Host row missing from the snapshot entirely.
            snapshot_at(4_000, vec![]),
        ]);
        let mut seeds = FxHashMap::default();
        let mut prev = None;
        let mut totals = None;
        let mut history = flow_history::FlowHistory::new();
        let mut heatmap = circuit_heatmap::CircuitHeatmapAccumulator::new();
        let mut qoo = circuit_qoo::CircuitQooAccumulator::new();
        let mut flow_seeds = FxHashMap::default();
        let mut rate_memory = CircuitRateMemory::default();
        for _ in 0..4 {
            poll_and_store_with(
                &service.socket,
                &mut seeds,
                &mut prev,
                &mut totals,
                &mut history,
                &mut heatmap,
                &mut qoo,
                &mut flow_seeds,
                &mut rate_memory,
                |_| Some("c1".to_string()),
                join_fixed,
            );
            let cache = CACHE.load();
            let poll = cache.captured_at_millis;
            if poll >= 3_000 {
                let row = cache
                    .by_circuit_id
                    .get("c1")
                    .unwrap_or_else(|| panic!("c1 dropped at poll {poll}"));
                assert_eq!(
                    row.bytes_per_second.down, 1_000,
                    "reset/missing poll {poll} must serve last-good, not zero"
                );
                assert_eq!(row.bytes_per_second.up, 1_000);
                assert!(
                    cache.last_good_circuit_ids.contains("c1"),
                    "poll {poll} must mark c1 as last-good"
                );
            }
        }
        reset_cache();
    }

    #[test]
    fn missing_socket_marks_stale_from_empty() {
        let _guard = TEST_LOCK.lock().expect("test invariant");
        reset_cache();
        let mut seeds = FxHashMap::default();
        let mut prev = None;
        let mut totals = None;
        let mut history = flow_history::FlowHistory::new();
        let mut heatmap = circuit_heatmap::CircuitHeatmapAccumulator::new();
        let mut qoo = circuit_qoo::CircuitQooAccumulator::new();
        let mut flow_seeds = FxHashMap::default();
        let mut rate_memory = CircuitRateMemory::default();
        let absent = std::env::temp_dir().join("lqosd-absent-analytics.sock");
        poll_and_store_with(
            &absent,
            &mut seeds,
            &mut prev,
            &mut totals,
            &mut history,
            &mut heatmap,
            &mut qoo,
            &mut flow_seeds,
            &mut rate_memory,
            |_| Some("c1".to_string()),
            join_fixed,
        );
        let cache = CACHE.load();
        assert!(cache.stale);
        assert!(cache.by_circuit_id.is_empty());
        reset_cache();
    }

    #[test]
    fn full_population_limit_is_the_bridge_maximum() {
        // §5.1.6: idle circuits must not vanish behind a display-sized
        // limit; the poll requests the full table.
        assert_eq!(FULL_POPULATION_LIMIT, 10_000);
        assert_eq!(FULL_POPULATION_LIMIT, analytics_ipc::DEFAULT_MAX_ROWS);
    }

    #[test]
    fn interval_bytes_per_second_scales_counter_deltas_over_the_window() {
 // K2 slice 1 top-level series; 3b: BYTES/second (the
        // sparkline calls to_bits_from_bytes, the LTS fields are bytes).
        assert_eq!(
            interval_bytes_per_second(1_000, 0, 1_000),
            1_000,
            "1000 B over 1 s = 1000 B/s"
        );
        assert_eq!(
            interval_bytes_per_second(0, 1_000, 1_000),
            0,
            "counter reset yields zero"
        );
        assert_eq!(
            interval_bytes_per_second(1_000, 0, 0),
            0,
            "zero window yields zero"
        );
    }

    /// `.66.2`: the DEMAND class split diffs like the actual/shaped totals; an
    /// absent counter (legacy / older bridge) yields zeros, and a counter reset
    /// saturates to 0 so the rate never under-counts.
    #[test]
    fn interval_rate_differences_class_counters_and_handles_absence() {
        let counters = |down: u64, up: u64| analytics_ipc::DirectionalCounters {
            download_bytes: down,
            upload_bytes: up,
        };
        // 1 s window: +1000 down / +500 up on the mapped class.
        assert_eq!(
            interval_rate(Some(&counters(2_000, 1_500)), 1_000, 1_000, 1_000),
            DownUpOrder::new(1_000, 500)
        );
        // Absent counters → zeros (no underflow from a 0 "now").
        assert_eq!(interval_rate(None, 1_000, 1_000, 1_000), DownUpOrder::zeroed());
        // Counter reset → saturating_sub → 0.
        assert_eq!(
            interval_rate(Some(&counters(0, 0)), 1_000, 1_000, 1_000),
            DownUpOrder::zeroed()
        );
        assert_eq!(counter_down(None), 0);
        assert_eq!(counter_up(None), 0);
        // Conservation: diffing mapped + unmapped equals the actual delta when the
        // source holds mapped+unmapped == actual.
        let mapped = interval_rate(Some(&counters(2_000, 1_500)), 1_000, 1_000, 1_000);
        let unmapped = interval_rate(Some(&counters(600, 300)), 500, 200, 1_000);
        assert_eq!(
            DownUpOrder::new(mapped.down + unmapped.down, mapped.up + unmapped.up),
            DownUpOrder::new(1_100, 600),
        );
    }

    #[test]
    fn real_snapshot_conservation_identity_holds() {
        // K2 slice 1 oracle: a checked-in real `.12` snapshot (pulled from
        // the heimdall bridge under the plain 100 Mbps fixture). The per-host
        // actual rows account for the whole-bridge top-level counters within
        // the tracking bound; the diagnostics disclose the gap (zero here).
        const FIXTURE: &[u8] = include_bytes!("testdata/snapshot_real.cbor");
        let snapshot: AnalyticsSnapshot =
            serde_cbor::from_slice(FIXTURE).expect("fixture decodes as an AnalyticsSnapshot");
        let top = snapshot.actual.as_ref().expect("top-level actual counters");
        assert!(
            !snapshot.hosts.is_empty(),
            "the fixture carries real host rows"
        );
        let host_down: u64 = snapshot.hosts.iter().map(|h| h.actual.download_bytes).sum();
        let host_up: u64 = snapshot.hosts.iter().map(|h| h.actual.upload_bytes).sum();
        let within = |host: u64, top: u64| {
            if top == 0 {
                return host == 0;
            }
            host.abs_diff(top) * 100 <= top * 2
        };
        assert!(
            within(host_down, top.download_bytes),
            "down: hosts {host_down} vs top {}",
            top.download_bytes
        );
        assert!(
            within(host_up, top.upload_bytes),
            "up: hosts {host_up} vs top {}",
            top.upload_bytes
        );
        assert_eq!(
            snapshot.diagnostics.host_capacity_drops, 0,
            "no host-capacity truncation in the fixture"
        );
    }

 /// the LTS/Insight trigger fires on the collation cadence, is
    /// armed (not fired) on its first tick, and never fires while disabled
    /// (the `gather_stats` + `is_active()` guard).
    #[test]
    fn lts_trigger_fires_on_the_collation_cadence_under_the_guard() {
        let mut cadence = LtsSubmitCadence::new(Duration::from_secs(10));
        let t0 = Instant::now();

        // Disabled never fires and never arms.
        assert!(!lts_submit_due(
            false,
            &mut cadence,
            t0 + Duration::from_secs(30)
        ));
        // Enabled: the first tick only arms.
        assert!(!lts_submit_due(true, &mut cadence, t0));
        // Inside the period: no fire.
        assert!(!lts_submit_due(
            true,
            &mut cadence,
            t0 + Duration::from_secs(9)
        ));
        // At the period: fire, and re-arm relative to the fire time.
        assert!(lts_submit_due(
            true,
            &mut cadence,
            t0 + Duration::from_secs(10)
        ));
        assert!(!lts_submit_due(
            true,
            &mut cadence,
            t0 + Duration::from_secs(11)
        ));
        assert!(lts_submit_due(
            true,
            &mut cadence,
            t0 + Duration::from_secs(20)
        ));
    }

    /// The trigger advances the legacy 1 Hz counter once per tick so the
    /// 60-tick shaper-utilization payload keeps its ~60 s cadence.
    #[test]
    fn lts_trigger_counter_advances_per_tick() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut trigger = LtsSubmitTrigger::new(false, Duration::from_secs(10), tx);
        let t0 = Instant::now();
        for offset in 0..60 {
            trigger.on_tick(t0 + Duration::from_secs(offset));
        }
        assert_eq!(trigger.counter, 60);
    }

    /// 3b §1.1: the all-protocol pps field folds the per-IP SENT rows.
    #[test]
    fn total_sent_packets_per_second_sums_the_per_ip_rows() {
        let mut by_ip: FxHashMap<IpAddr, IpLiveRow> = FxHashMap::default();
        by_ip.insert(
            "192.0.2.1".parse().expect("ip"),
            IpLiveRow {
                packets_sent_per_second: DownUpOrder::new(3, 4),
                ..IpLiveRow::default()
            },
        );
        by_ip.insert(
            "192.0.2.2".parse().expect("ip"),
            IpLiveRow {
                packets_sent_per_second: DownUpOrder::new(5, 6),
                ..IpLiveRow::default()
            },
        );
        assert_eq!(
            total_sent_packets_per_second(&by_ip),
            DownUpOrder::new(8, 10)
        );
    }

 /// 3b (the fourth K3-orphan): the live streaming-shaper rates come
    /// from the bridge snapshot when the actor is active — headline = the
    /// DELIVERED (shaped) bits/s, packets = the enqueued fold.
    #[test]
    fn live_shaper_rates_read_the_bridge_snapshot() {
        let mut by_ip: FxHashMap<IpAddr, IpLiveRow> = FxHashMap::default();
        by_ip.insert(
            "192.0.2.1".parse().expect("ip"),
            IpLiveRow {
                packets_per_second: DownUpOrder::new(3, 4),
                ..IpLiveRow::default()
            },
        );
        let snapshot = BridgeStatsSnapshot {
            bridge_shaped_bytes_per_second: DownUpOrder::new(250, 100),
            by_ip,
            ..BridgeStatsSnapshot::default()
        };
        assert_eq!(
            live_shaper_rates(&snapshot),
            (
                DownUpOrder::new(2_000, 800),
                DownUpOrder::new(2_000, 800),
                DownUpOrder::new(3, 4),
            )
        );
    }

 /// 3b (the fifth orphan-class repoint): the gateway `circuit_live`
    /// responder folds the bridge's per-IP rows into per-device DELIVERED
    /// rates (demand != delivered in the fixture).
    #[test]
    fn circuit_device_rates_use_the_delivered_series() {
        use lqos_config::{ConfigShapedDevices, ShapedDevice};
        use lqos_network_devices::{NetworkDevicesCatalog, ShapedDevicesCatalog};
        use std::net::Ipv4Addr;
        use std::sync::Arc;

        let mut shaped = ConfigShapedDevices::default();
        shaped.replace_with_new_data(vec![ShapedDevice {
            circuit_id: "c1".to_string(),
            circuit_name: "C1".to_string(),
            device_id: "d1".to_string(),
            parent_node: "ap".to_string(),
            ipv4: vec![(Ipv4Addr::new(100, 97, 0, 1), 32)],
            circuit_hash: lqos_utils::hash_to_i64("c1"),
            device_hash: lqos_utils::hash_to_i64("d1"),
            ..ShapedDevice::default()
        }]);
        let shaped_catalog = ShapedDevicesCatalog::from_shaped_devices(Arc::new(shaped));
        let catalog = NetworkDevicesCatalog::from_snapshots(shaped_catalog, Arc::new(Vec::new()));

        let mut by_ip: FxHashMap<IpAddr, IpLiveRow> = FxHashMap::default();
        by_ip.insert(
            "100.97.0.1".parse().expect("ip"),
            IpLiveRow {
                bytes_per_second: DownUpOrder::new(999_999, 999_999), // demand
                shaped_bytes_per_second: DownUpOrder::new(1_000, 500), // delivered
                tcp_packets: 40,
                tcp_retransmits: 2,
                rtt_current_p50_nanos: Some(2_000_000),
                ..IpLiveRow::default()
            },
        );
        let circuit_hash = lqos_utils::hash_to_i64("c1");
        let device_hash = lqos_utils::hash_to_i64("d1");
        let rates = circuit_device_delivered_rates(&by_ip, &catalog, circuit_hash);
        let entry = rates.get(&device_hash).expect("device present");
        assert_eq!(entry.bytes, DownUpOrder::new(1_000, 500));
        assert_eq!(entry.tcp_packets, DownUpOrder::new(40, 0));
        assert_eq!(entry.tcp_retries, DownUpOrder::new(2, 0));
        assert_eq!(entry.rtts, vec![2.0]);
        assert!(
            circuit_device_delivered_rates(&by_ip, &catalog, circuit_hash + 1).is_empty(),
            "a different circuit hash yields nothing"
        );
    }

 /// 3b §1.1 ruling c): the protocol splits are per-second deltas
    /// from the per-FlowId packet seed — not a lifetime tally — and their sum
    /// conserves against the all-protocol sent pps fold.
    #[test]
    fn flow_packet_seed_differences_protocol_rates_and_conserves() {
 // 3b sweep-fixture guard: the DEMAND is deliberately larger
        // than the DELIVERED series, so a per-flow rate built from `actual`
        // would fail the byte-rate assertion below.
        let flow = |protocol: u8, sent_down: u64, sent_up: u64| analytics_ipc::FlowRow {
            id: analytics_ipc::FlowId {
                protocol,
                ..analytics_ipc::FlowId::default()
            },
            sent_pkts_down: sent_down,
            sent_pkts_up: sent_up,
            actual: analytics_ipc::DirectionalCounters {
                download_bytes: sent_down + 10_000,
                upload_bytes: sent_up + 10_000,
            },
            shaped: analytics_ipc::DirectionalCounters {
                download_bytes: sent_down,
                upload_bytes: sent_up,
            },
            ..analytics_ipc::FlowRow::default()
        };
        let mut seeds: FxHashMap<analytics_ipc::FlowId, FlowSeed> = FxHashMap::default();
        // First poll seeds only (no rates).
        let (directional, protocol) = compute_flow_rates(
            &mut seeds,
            &[flow(6, 100, 0), flow(17, 0, 100), flow(1, 10, 0)],
            1_000,
        );
        assert!(
            directional.is_empty(),
            "first sight yields no directional rate"
        );
        assert_eq!(protocol.total(), DownUpOrder::zeroed());

        // One second later: TCP +100 down, UDP +50 up, ICMP +1 down.
        let (directional, protocol) = compute_flow_rates(
            &mut seeds,
            &[flow(6, 200, 0), flow(17, 0, 150), flow(1, 11, 0)],
            1_000,
        );
        assert_eq!(protocol.tcp, DownUpOrder::new(100, 0));
        assert_eq!(protocol.udp, DownUpOrder::new(0, 50));
        assert_eq!(protocol.icmp, DownUpOrder::new(1, 0));
        assert_eq!(protocol.total(), DownUpOrder::new(101, 50));
        // `.65`: the directional map is the only per-flow byte rate; it splits
        // the same seed deltas down/up (the retired combined map is gone).
        assert_eq!(directional.values().map(|row| row.down).sum::<u64>(), 101);
        assert_eq!(directional.values().map(|row| row.up).sum::<u64>(), 50);
        let tcp_id = analytics_ipc::FlowId {
            protocol: 6,
            ..analytics_ipc::FlowId::default()
        };
        assert_eq!(
            directional.get(&tcp_id).copied(),
            Some(DownUpOrder::new(100, 0)),
            "the TCP delta is down-only"
        );

        // Conservation: the same cumulative counters at the HOST level fold to
        // the same per-second all-protocol total.
        let host = |sent_down: u64, sent_up: u64| HostRow {
            id: HostId {
                circuit_id: 1,
                subscriber_ip: "192.0.2.1".parse().expect("ip"),
            },
            sent_packets: DirectionalPackets {
                download_packets: sent_down,
                upload_packets: sent_up,
            },
            ..HostRow::default()
        };
        let prev = AnalyticsSnapshot {
            captured_at_millis: 0,
            hosts: vec![host(110, 100)],
            ..AnalyticsSnapshot::default()
        };
        let (_, prev_seeds) = translate::difference(&FxHashMap::default(), 0, &prev);
        let next = AnalyticsSnapshot {
            captured_at_millis: 1_000,
            hosts: vec![host(211, 150)],
            ..AnalyticsSnapshot::default()
        };
        let (deltas, _) = translate::difference(&prev_seeds, 0, &next);
        let by_ip = build_by_ip_rates(&deltas, &next.hosts);
        assert_eq!(
            total_sent_packets_per_second(&by_ip),
            protocol.total(),
            "Σ protocol deltas == the HostRow sent pps fold"
        );
    }

    /// B1 residual pin: non-6/17/1 transports reach the ALL-PROTOCOL host fold
    /// but are DROPPED from the three classified buckets, so
    /// `Σ(tcp+udp+icmp) < all-protocol` whenever Unknown transport is present.
    #[test]
    fn unknown_transport_flows_reach_the_all_protocol_fold_only() {
        let flow = |protocol: u8, sent_down: u64, sent_up: u64| analytics_ipc::FlowRow {
            id: analytics_ipc::FlowId {
                protocol,
                ..analytics_ipc::FlowId::default()
            },
            sent_pkts_down: sent_down,
            sent_pkts_up: sent_up,
            ..analytics_ipc::FlowRow::default()
        };
        let mut seeds: FxHashMap<analytics_ipc::FlowId, FlowSeed> = FxHashMap::default();
        // Seed one classified TCP flow and one Unknown flow (58 = ICMPv6).
        let _ = compute_flow_rates(&mut seeds, &[flow(6, 100, 0), flow(58, 20, 0)], 1_000);
        // One second later: TCP +10 down, Unknown +7 down.
        let (_directional, protocol) =
            compute_flow_rates(&mut seeds, &[flow(6, 110, 0), flow(58, 27, 0)], 1_000);
        assert_eq!(
            protocol.total(),
            DownUpOrder::new(10, 0),
            "only the TCP delta is bucketed"
        );
        assert_eq!(protocol.udp, DownUpOrder::zeroed());
        assert_eq!(protocol.icmp, DownUpOrder::zeroed());

        // The host fold counts BOTH transports ⇒ it exceeds the classified sum.
        let host = |sent_down: u64| HostRow {
            id: HostId {
                circuit_id: 1,
                subscriber_ip: "192.0.2.1".parse().expect("ip"),
            },
            sent_packets: DirectionalPackets {
                download_packets: sent_down,
                upload_packets: 0,
            },
            ..HostRow::default()
        };
        let prev = AnalyticsSnapshot {
            captured_at_millis: 0,
            hosts: vec![host(120)],
            ..AnalyticsSnapshot::default()
        };
        let (_, prev_seeds) = translate::difference(&FxHashMap::default(), 0, &prev);
        let next = AnalyticsSnapshot {
            captured_at_millis: 1_000,
            hosts: vec![host(137)],
            ..AnalyticsSnapshot::default()
        };
        let (deltas, _) = translate::difference(&prev_seeds, 0, &next);
        let by_ip = build_by_ip_rates(&deltas, &next.hosts);
        let all_protocol = total_sent_packets_per_second(&by_ip);
        assert_eq!(
            all_protocol,
            DownUpOrder::new(17, 0),
            "host fold counts TCP + Unknown"
        );
        assert_eq!(
            all_protocol.down - protocol.total().down,
            7,
            "the 7 Unknown packets are the classified residual"
        );
    }
}

#[cfg(test)]
mod tree_live_tests {
    use super::*;
    use lqos_config::NetworkJsonNode;
    use lqos_utils::units::DownUpOrder;

    fn store_node(name: &str, parents: Vec<usize>, immediate: Option<usize>) -> NetworkJsonNode {
        NetworkJsonNode {
            name: name.to_string(),
            id: Some(name.to_string()),
            max_throughput: (0.0, 0.0),
            current_throughput: DownUpOrder::zeroed(),
            current_packets: DownUpOrder::zeroed(),
            current_tcp_packets: DownUpOrder::zeroed(),
            current_udp_packets: DownUpOrder::zeroed(),
            current_icmp_packets: DownUpOrder::zeroed(),
            current_tcp_retransmits: DownUpOrder::zeroed(),
            current_tcp_retransmit_packets: DownUpOrder::zeroed(),
            current_rtt_flows: DownUpOrder::zeroed(),
            current_marks: DownUpOrder::zeroed(),
            current_drops: DownUpOrder::zeroed(),
            rtt_buffer: Default::default(),
            parents,
            immediate_parent: immediate,
            node_type: None,
            latitude: None,
            longitude: None,
            active_attachment_name: None,
            heatmap: None,
        }
    }

    fn row(shaped_down: u64, shaped_up: u64) -> translate::CircuitLiveRow {
        translate::CircuitLiveRow {
            bytes_per_second: DownUpOrder::zeroed(),
            shaped_bytes_per_second: DownUpOrder {
                down: shaped_down,
                up: shaped_up,
            },
            rtt_current_p50_nanos: DownUpOrder {
                down: None,
                up: None,
            },
            rtt_histogram: Default::default(),
            tcp_retransmit_sample: Default::default(),
            last_seen_nanos: 0,
            bridge_circuit_id: 1,
        }
    }

    /// root -> site -> ap (ap's chain [0,1,2]). The CIRCUIT is a catalog row
    /// (`c1`) whose `parent_node` is `ap` — it is not a tree node.
    fn write_store() {
        lqos_network_devices::with_network_json_write(|net_json| {
            net_json.nodes.clear();
            net_json.nodes.push(store_node("Root", vec![], None));
            net_json.nodes.push(store_node("site", vec![0, 1], Some(0)));
            net_json
                .nodes
                .push(store_node("ap", vec![0, 1, 2], Some(1)));
        });
    }

    /// 3b DEFECT 1 oracle (rig-shaped): a catalog circuit `c1` with
    /// `parent_node=ap` lands under `ap` and every ancestor; unmatched is 0.
    #[test]
    fn circuit_rows_attribute_through_their_parent_node() {
        let _guard = super::tests::TEST_LOCK.lock().expect("test invariant");
        write_store();
        let mut parents: FxHashMap<String, String> = FxHashMap::default();
        parents.insert("c1".to_string(), "ap".to_string());
        let mut joined = FxHashMap::default();
        joined.insert("c1".to_string(), row(100, 50));

        let view = build_tree_live_view(&joined, &parents, 42, false, &FxHashMap::default());
        // ap's chain [0,1,2]: the circuit's shaped bytes roll up ap + ancestors.
        for idx in 0..=2 {
            let sums = view
                .current_throughput
                .get(&idx)
                .copied()
                .unwrap_or_default();
            assert_eq!(
                (sums.down, sums.up),
                (100, 50),
                "node {idx} must carry the rolled circuit rate"
            );
        }
        assert_eq!(view.unmatched_circuits, 0, "the parent join resolves");
        assert!(!view.stale);
        assert_eq!(view.captured_at_millis, 42);
    }

    /// F3-B A1 (`docs/executive_rtt_rollup_card.md` §4c): the per-node `rtts`
    /// list is the TRACKER `median_latency()` source, NOT the bridge per-IP p50
    /// (`rtt_current_p50_nanos`). A site whose circuits read <100 ms rolls up
    /// <100 ms even when the bridge p50 is high.
    #[test]
    fn tree_live_view_rtts_use_the_tracker_median_source() {
        let _guard = super::tests::TEST_LOCK.lock().expect("test invariant");
        write_store();
        let mut parents: FxHashMap<String, String> = FxHashMap::default();
        parents.insert("c1".to_string(), "ap".to_string());
        let mut joined = FxHashMap::default();
        let mut r = row(100, 50);
        r.rtt_current_p50_nanos = DownUpOrder {
            down: Some(639_000_000),
            up: Some(639_000_000),
        };
        joined.insert("c1".to_string(), r);
        let mut medians: FxHashMap<String, f32> = FxHashMap::default();
        medians.insert("c1".to_string(), 42.0);
        let view = build_tree_live_view(&joined, &parents, 1, false, &medians);
        // Every node on c1's chain carries the tracker median (42.0), never 639.
        for idx in 0..=2 {
            assert_eq!(
                view.rtts.get(&idx).map(Vec::as_slice),
                Some([42.0f32].as_slice()),
                "node {idx} must use the tracker median source"
            );
        }
    }

    /// A circuit whose id matches no catalog parent is counted, not silent.
    #[test]
    fn unmatched_circuit_without_a_parent_is_counted() {
        let _guard = super::tests::TEST_LOCK.lock().expect("test invariant");
        write_store();
        let mut parents: FxHashMap<String, String> = FxHashMap::default();
        parents.insert("c1".to_string(), "ap".to_string());
        let mut joined = FxHashMap::default();
        joined.insert("ghost".to_string(), row(999, 999));
        let view = build_tree_live_view(&joined, &parents, 1, false, &FxHashMap::default());
        assert_eq!(view.unmatched_circuits, 1, "ghost counted, not silent");
    }

    #[test]
    fn sibling_circuit_rows_accumulate() {
        let _guard = super::tests::TEST_LOCK.lock().expect("test invariant");
        write_store();
        let mut parents: FxHashMap<String, String> = FxHashMap::default();
        parents.insert("c1".to_string(), "ap".to_string());
        parents.insert("c2".to_string(), "ap".to_string());
        let mut joined = FxHashMap::default();
        joined.insert("c1".to_string(), row(100, 50));
        joined.insert("c2".to_string(), row(30, 20));
        let view = build_tree_live_view(&joined, &parents, 7, false, &FxHashMap::default());
        let ap = view.current_throughput.get(&2).copied().unwrap_or_default();
        assert_eq!((ap.down, ap.up), (130, 70), "siblings sum at the AP");
    }

    #[test]
    fn overlay_is_identity_when_inactive_or_never_built() {
        let _guard = super::tests::TEST_LOCK.lock().expect("test invariant");
        let mut transport = store_node("x", vec![], None).clone_to_transit();
        transport.current_throughput = (11, 22);
        // Tests never start the actor (is_active()=false) and no view
        // was published: the overlay must be a no-op (legacy identity).
        crate::bridge_analytics::overlay_live_fields(1, &mut transport);
        assert_eq!(transport.current_throughput, (11, 22));
    }

    fn lite_node() -> NetworkTreeLiteNode {
        NetworkTreeLiteNode {
            name: "x".to_string(),
            id: None,
            max_throughput: (0.0, 0.0),
            current_throughput: (1, 2),
            current_tcp_packets: (3, 4),
            current_tcp_retransmit_packets: (0, 0),
            current_retransmits: (0, 0),
            rtts: Vec::new(),
            qoo: (None, None),
            parents: Vec::new(),
            immediate_parent: None,
            node_type: None,
            latitude: None,
            longitude: None,
            subtree_circuit_count: 0,
        }
    }

    /// Cluster 5 (§2.5): the full-transport overlay maps every live field.
    #[test]
    fn apply_live_fields_maps_every_transport_field() {
        let mut view = TreeLiveView::default();
        view.current_throughput
            .insert(3, DownUpOrder { down: 11, up: 22 });
        view.rtts.insert(3, vec![5.0, 6.0]);
        view.current_rtt_flows.insert(3, (7, 8));
        view.current_retransmits.insert(3, (9, 10));
        view.current_tcp_retransmit_packets.insert(3, (101, 102));
        let mut transport = store_node("x", vec![], None).clone_to_transit();
        // F3-B A1: the overlay must NOT clobber `rtts` (the tree RTT now comes
        // from the restored tracker→`node.rtt_buffer` accumulation).
        transport.rtts = vec![9.0];
        apply_live_fields(&view, 3, &mut transport);
        assert_eq!(transport.current_throughput, (11, 22));
        assert_eq!(
            transport.rtts,
            vec![9.0],
            "the E2 overlay must not replace rtts (A1)"
        );
        assert_eq!(transport.current_rtt_flows, (7, 8));
        assert_eq!(transport.current_retransmits, (9, 10));
        assert_eq!(transport.current_tcp_retransmit_packets, (101, 102));
    }

    /// Cluster 5 (§2.5): the lite overlay maps the shared fields and leaves
    /// the kernel-exclusive ones (`current_tcp_packets`, `qoo`) untouched.
    #[test]
    fn apply_live_fields_lite_maps_shared_fields_and_leaves_kernel_only() {
        let mut view = TreeLiveView::default();
        view.current_throughput
            .insert(3, DownUpOrder { down: 11, up: 22 });
        view.rtts.insert(3, vec![5.0, 6.0]);
        view.current_retransmits.insert(3, (9, 10));
        view.current_tcp_retransmit_packets.insert(3, (101, 102));
        let mut node = lite_node();
        // F3-B A1: no `rtts` clobber.
        node.rtts = vec![9.0];
        apply_live_fields_lite(&view, 3, &mut node);
        assert_eq!(node.current_throughput, (11, 22));
        assert_eq!(
            node.rtts,
            vec![9.0],
            "the E2 overlay must not replace rtts (A1)"
        );
        assert_eq!(node.current_retransmits, (9, 10));
        assert_eq!(node.current_tcp_retransmit_packets, (101, 102));
        assert_eq!(node.current_tcp_packets, (3, 4), "no bridge source");
        assert_eq!(node.qoo, (None, None), "no bridge source");
    }

    /// Cluster 5 (§2.5): the lite overlay is identity when the actor is
    /// inactive (legacy kernel path unchanged).
    #[test]
    fn overlay_lite_is_identity_when_inactive_or_never_built() {
        let _guard = super::tests::TEST_LOCK.lock().expect("test invariant");
        let mut node = lite_node();
        node.current_throughput = (11, 22);
        crate::bridge_analytics::overlay_live_fields_lite(1, &mut node);
        assert_eq!(node.current_throughput, (11, 22));
    }

    #[test]
    fn stale_propagates_only_after_a_view_exists() {
        // The view is process-global; serialize against the publisher tests
        // (poll_and_store_with) AND reset it, so the never-built precondition
        // holds regardless of test order.
        let _guard = super::tests::TEST_LOCK.lock().expect("test invariant");
        TREE_LIVE_VIEW.store(Arc::new(None));
        // Never-built view: mark-stale is a no-op (stays None — the
        // never-loaded degrade, not fabricated data).
        mark_tree_live_view_stale();
        assert!(TREE_LIVE_VIEW.load_full().is_none());

        publish_tree_live_view(
            &FxHashMap::default(),
            &FxHashMap::default(),
            1,
            false,
            &FxHashMap::default(),
        );
        mark_tree_live_view_stale();
        let view = tree_live_view().expect("view exists after publish");
        assert!(view.stale, "last-good keeps serving, flagged stale");
    }

    #[test]
    fn by_ip_rates_fold_per_ip_from_deltas() {
        use std::net::IpAddr;
        let mut deltas = FxHashMap::default();
        let ip_a: IpAddr = "100.97.0.1".parse().expect("test invariant");
        let ip_b: IpAddr = "100.97.0.2".parse().expect("test invariant");
        deltas.insert(
            (1, ip_a),
            translate::HostDelta {
                bytes_per_second: DownUpOrder { down: 10, up: 20 },
                shaped_bytes_per_second: DownUpOrder { down: 9, up: 18 },
                enqueued_packets_per_second: DownUpOrder { down: 5, up: 7 },
                sent_packets_per_second: DownUpOrder { down: 4, up: 6 },
                tcp_packets: 5,
                tcp_retransmits: 1,
            },
        );
        deltas.insert(
            (2, ip_a),
            translate::HostDelta {
                bytes_per_second: DownUpOrder { down: 1, up: 2 },
                shaped_bytes_per_second: DownUpOrder { down: 1, up: 2 },
                enqueued_packets_per_second: DownUpOrder { down: 3, up: 2 },
                sent_packets_per_second: DownUpOrder { down: 2, up: 1 },
                tcp_packets: 3,
                tcp_retransmits: 0,
            },
        );
        deltas.insert(
            (3, ip_b),
            translate::HostDelta {
                bytes_per_second: DownUpOrder { down: 7, up: 8 },
                shaped_bytes_per_second: DownUpOrder { down: 6, up: 7 },
                enqueued_packets_per_second: DownUpOrder { down: 1, up: 1 },
                sent_packets_per_second: DownUpOrder { down: 1, up: 1 },
                tcp_packets: 2,
                tcp_retransmits: 0,
            },
        );
        let by_ip = build_by_ip_rates(&deltas, &[]);
        let a = by_ip.get(&ip_a).expect("ip a present");
        assert_eq!(a.bytes_per_second.down, 11, "two hosts on one IP sum");
        assert_eq!(a.shaped_bytes_per_second.up, 20);
        assert_eq!(a.packets_per_second.down, 8, "enqueued pps fold per IP");
        assert_eq!(a.packets_per_second.up, 9);
        let b = by_ip.get(&ip_b).expect("ip b present");
        assert_eq!(b.shaped_bytes_per_second.down, 6);
        assert_eq!(b.packets_per_second.down, 1);
    }

    #[test]
    fn unmatched_warn_rate_limit_covers_incident_cadence_and_reset() {
        // The T2-closure nit: a standing circuit-id mismatch ticks every
        // 1s — the WARN must fire once per window, not once per tick,
        // reset when the mismatch clears, and never suppress on a lost
        // clock.
        let mut state = UnmatchedWarnState::default();

        // First sighting warns immediately...
        assert!(unmatched_warn_due(1_000, 3, &mut state));
        // ...then every tick inside the window is suppressed...
        assert!(!unmatched_warn_due(1_005, 3, &mut state));
        assert!(!unmatched_warn_due(1_030, 3, &mut state));
        // ...and the line returns once the window elapses.
        assert!(unmatched_warn_due(
            1_000 + UNMATCHED_WARN_MIN_INTERVAL_SECS,
            3,
            &mut state
        ));

        // The mismatch clears: state resets silently, no warning.
        assert!(!unmatched_warn_due(1_100, 0, &mut state));
        // A FRESH incident warns immediately again (not treated as
        // part of the old window).
        assert!(unmatched_warn_due(1_105, 2, &mut state));

        // Clock unavailable (now == 0): always due, records nothing —
        // today's every-tick behavior, and a recovered clock starts a
        // clean window instead of inheriting a bogus one.
        let mut clockless = UnmatchedWarnState::default();
        assert!(unmatched_warn_due(0, 4, &mut clockless));
        assert_eq!(clockless.last_warn_unix, None);
        assert!(unmatched_warn_due(0, 4, &mut clockless));
    }

    /// §lqos-74t.40 pin 4 END-TO-END: the synthetic `orphans` umbrella is a
    /// non-entity — a circuit attributed to it
    /// must NOT inflate the ROOT's RTT (the A1 `node.rtt_buffer` fold) NOR its
    /// retransmit rollup (the site `node_circuits` index). The umbrella ROW
    /// exclusion alone was partial: the ancestor chain still reached the root.
    #[test]
    fn root_excludes_the_orphan_umbrella() {
        use lqos_utils::rtt::RttBucket;

        let _guard = super::tests::TEST_LOCK.lock().expect("test invariant");
        const ORPHANS: &str = lqos_dataplane_types::ORPHANS_UMBRELLA_NODE_ID;
        // Root(0) -> WestRedd(1) -> {orphans umbrella(2), real_site(3)}.
        lqos_network_devices::with_network_json_write(|net_json| {
            net_json.nodes.clear();
            net_json.nodes.push(store_node("Root", vec![], None));
            net_json
                .nodes
                .push(store_node("WestRedd", vec![0, 1], Some(0)));
            net_json
                .nodes
                .push(store_node(ORPHANS, vec![0, 1, 2], Some(1)));
            net_json
                .nodes
                .push(store_node("real_site", vec![0, 1, 3], Some(1)));
        });

        // Retransmit rollup: the umbrella circuit is dropped; the real circuit
        // still lands on the root's chain.
        let mut parents: FxHashMap<String, String> = FxHashMap::default();
        parents.insert("c_orphan".to_string(), ORPHANS.to_string());
        parents.insert("c_real".to_string(), "real_site".to_string());
        let nodes = lqos_network_devices::with_network_json_read(|net_json| {
            net_json.get_nodes_when_ready().clone()
        });
        let node_circuits =
            node_circuits_by_index(&nodes, &parents, ["c_orphan", "c_real"].into_iter());
        assert_eq!(
            node_circuits.get(&0).map(Vec::as_slice),
            Some(["c_real"].as_slice()),
            "the ROOT excludes the orphan circuit from its retransmit rollup"
        );
        assert!(
            node_circuits.get(&2).is_none(),
            "the umbrella is a non-entity (no row, no members)"
        );
        assert_eq!(
            node_circuits.get(&3).map(Vec::as_slice),
            Some(["c_real"].as_slice()),
            "a real applied descendant keeps its own circuit"
        );

        // RTT fold: the orphan's 500 ms sample must not reach the root's
        // `rtt_buffer`; only the real site's samples do — and the two directions
        // are split (`§.40.2`), so the Upload pool reads its OWN samples.
        let snapshot = TrackerRttSnapshot {
            folds: vec![
                (vec![0, 1, 2], vec![500_000_000], Vec::new()),
                (
                    vec![0, 1, 3],
                    vec![10_000_000, 10_000_000],
                    vec![30_000_000, 30_000_000],
                ),
            ],
            median_ms_by_circuit: FxHashMap::default(),
        };
        apply_tree_rtt_accumulation(&snapshot);
        lqos_network_devices::with_network_json_read(|net_json| {
            let nodes = net_json.get_nodes_when_ready();
            let root = &nodes[0];
            assert_eq!(
                root.rtt_buffer
                    .sample_count(RttBucket::Current, FlowbeeEffectiveDirection::Download),
                2,
                "the ROOT's DOWNLOAD pool carries only the real circuit's two samples"
            );
            assert_eq!(
                root.rtt_buffer
                    .sample_count(RttBucket::Current, FlowbeeEffectiveDirection::Upload),
                2,
                "the ROOT's UPLOAD pool carries only the real circuit's two samples"
            );
            let p90 = root
                .rtt_buffer
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 90)
                .expect("the real samples are present");
            assert!(
                p90.as_millis() < 100.0,
                "the ROOT p90 excludes the orphan's 500 ms: {}",
                p90.as_millis()
            );
            let up_p90 = root
                .rtt_buffer
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 90)
                .expect("the upload samples are present");
            assert!(
                up_p90.as_millis() > 20.0,
                "the Upload pool reads its OWN samples, not the merged download: {}",
                up_p90.as_millis()
            );
        });
    }

    /// §lqos-74t.40 pin 4 (REQUIRED): the effective-parent
    /// preference is NON-VACUOUS. On a **differing** input the effective parent
    /// wins; where the effective map has no entry the configured catalog parent
    /// is kept. (The live runtime catalog sets both fields equal by construction,
    /// which is why pin 4 is a no-op live and cannot be proven there.)
    #[test]
    fn circuit_parent_prefers_the_effective_parent_on_a_differing_input() {
        let devices = vec![
            ("c1".to_string(), "configured_parent".to_string()),
            ("c2".to_string(), "configured_parent".to_string()),
        ];
        let effective = |circuit_id: &str| match circuit_id {
            "c1" => Some("effective_parent".to_string()),
            _ => None,
        };
        let parents = circuit_parent_node_by_id_from(devices, effective);
        assert_eq!(
            parents.get("c1").map(String::as_str),
            Some("effective_parent"),
            "the EFFECTIVE parent wins over the configured catalog parent"
        );
        assert_eq!(
            parents.get("c2").map(String::as_str),
            Some("configured_parent"),
            "an absent effective parent falls back to the configured parent"
        );
    }

    /// §lqos-74t.8 A1: the bulk read maps slot → product circuit → `circuit_hash`,
    /// folds each direction's cumulative drops/marks, and counts CAKE leaves from
    /// `tins` presence. An unknown slot is skipped (never mis-attributed).
    #[test]
    fn queue_stats_bulk_maps_slots_and_counts_cake() {
        use analytics_ipc::{QdiscDirectionRow, QdiscTinRow};
        let mut by_circuit_id: FxHashMap<String, CircuitLiveRow> = FxHashMap::default();
        by_circuit_id.insert(
            "c1".to_string(),
            CircuitLiveRow {
                bridge_circuit_id: 7,
                ..Default::default()
            },
        );
        by_circuit_id.insert(
            "c2".to_string(),
            CircuitLiveRow {
                bridge_circuit_id: 9,
                ..Default::default()
            },
        );
        let snapshot = BridgeStatsSnapshot {
            by_circuit_id,
            ..Default::default()
        };
        let slot_to_circuit = slot_to_circuit_id(&snapshot);
        assert_eq!(slot_to_circuit.get(&7).map(String::as_str), Some("c1"));
        assert_eq!(slot_to_circuit.get(&9).map(String::as_str), Some("c2"));

        let cake = |drop_pkts: u64, marks: u64| QdiscDirectionRow {
            tins: vec![QdiscTinRow::default()],
            drop_pkts,
            aqm_marks: marks,
            ..Default::default()
        };
        let fifo = QdiscDirectionRow::default(); // no tins == not CAKE
        let rows = vec![
            QdiscDetailRow {
                slot_id: 7,
                download: cake(11, 2),
                upload: fifo.clone(),
                ..Default::default()
            },
            QdiscDetailRow {
                slot_id: 9,
                download: fifo.clone(),
                upload: fifo.clone(),
                ..Default::default()
            },
            QdiscDetailRow {
                slot_id: 999,
                download: cake(1, 1),
                upload: fifo.clone(),
                ..Default::default()
            },
        ];
        let (per_circuit, counts) = fold_queue_stats_rows(&rows, &slot_to_circuit);
        assert_eq!(per_circuit.len(), 2, "an unknown slot is skipped");
        let (_, drops, marks) = per_circuit
            .iter()
            .find(|(hash, _, _)| *hash == lqos_utils::hash_to_i64("c1"))
            .expect("c1 folded");
        assert_eq!(drops.down, 11);
        assert_eq!(marks.down, 2);
        assert_eq!(counts.cake, 1, "only c1's download direction is a CAKE leaf");
    }

    /// §lqos-74t.8 A1: slot 0 (`best_effort`) never has a leaf and is excluded.
    #[test]
    fn slot_map_excludes_the_best_effort_slot() {
        let mut by_circuit_id: FxHashMap<String, CircuitLiveRow> = FxHashMap::default();
        by_circuit_id.insert(
            "c0".to_string(),
            CircuitLiveRow {
                bridge_circuit_id: 0,
                ..Default::default()
            },
        );
        let snapshot = BridgeStatsSnapshot {
            by_circuit_id,
            ..Default::default()
        };
        assert!(slot_to_circuit_id(&snapshot).is_empty());
    }

    /// §lqos-74t.40 pin 4 (bounded item): `circuit_ancestors_for` drops an
    /// umbrella-attributed circuit entirely (no ancestor chain), so the
    /// QoO node rollup never folds it into the root.
    #[test]
    fn circuit_ancestors_drop_an_umbrella_attributed_circuit() {
        let _guard = super::tests::TEST_LOCK.lock().expect("test invariant");
        const ORPHANS: &str = lqos_dataplane_types::ORPHANS_UMBRELLA_NODE_ID;
        lqos_network_devices::with_network_json_write(|net_json| {
            net_json.nodes.clear();
            net_json.nodes.push(store_node("Root", vec![], None));
            net_json
                .nodes
                .push(store_node("WestRedd", vec![0, 1], Some(0)));
            net_json.nodes.push(store_node(ORPHANS, vec![0, 1, 2], Some(1)));
            net_json
                .nodes
                .push(store_node("real_site", vec![0, 1, 3], Some(1)));
        });
        let mut parents: FxHashMap<String, String> = FxHashMap::default();
        parents.insert("c_orphan".to_string(), ORPHANS.to_string());
        parents.insert("c_real".to_string(), "real_site".to_string());
        assert!(
            circuit_ancestors_for("c_orphan", &parents).is_empty(),
            "an umbrella-attributed circuit has NO ancestor chain"
        );
        assert_eq!(
            circuit_ancestors_for("c_real", &parents),
            vec![
                "Root".to_string(),
                "WestRedd".to_string(),
                "real_site".to_string()
            ]
        );
    }

    /// §`lqos-74t.40.1` (bridge-driven, no tracker fixture): the tree/site RTT
    /// source is the BRIDGE per-circuit distribution. Only the bridge snapshot is
    /// populated; `snapshot_bridge_rtt` must fold the member samples into the
    /// node chain and hand the SAME pool to the QoO — gating out sub-1 Mb /
    /// single-sample circuits and the umbrella.
    #[test]
    fn bridge_rtt_source_folds_member_samples_and_gates() {
        use crate::bridge_analytics::circuit_heatmap::{
            CircuitHeatmapSnapshot, CircuitRttSamples, HeatmapMinute,
        };
        let _guard = super::tests::TEST_LOCK.lock().expect("test invariant");
        const ORPHANS: &str = lqos_dataplane_types::ORPHANS_UMBRELLA_NODE_ID;
        lqos_network_devices::with_network_json_write(|net_json| {
            net_json.nodes.clear();
            net_json.nodes.push(store_node("Root", vec![], None));
            net_json
                .nodes
                .push(store_node("WestRedd", vec![0, 1], Some(0)));
            net_json.nodes.push(store_node(ORPHANS, vec![0, 1, 2], Some(1)));
        });
        let minute = |bytes: u64| HeatmapMinute {
            start_millis: 0,
            down_bytes: bytes,
            ..Default::default()
        };
        let samples = |down: Vec<u32>, up: Vec<u32>| CircuitRttSamples { down, up };
        let mut heap = CircuitHeatmapSnapshot::default();
        // Qualifying: >= 2 samples and >= 1 MB in the window.
        heap.rtt_samples_by_circuit
            .insert("c1".to_string(), samples(vec![10_000, 20_000], Vec::new()));
        heap.by_circuit.insert("c1".to_string(), vec![minute(2_000_000)]);
        // Low volume: gated out (sub-1 Mb long-poll reject).
        heap.rtt_samples_by_circuit
            .insert("c_low".to_string(), samples(vec![5_000, 5_000], Vec::new()));
        heap.by_circuit.insert("c_low".to_string(), vec![minute(10)]);
        // Single sample: gated out.
        heap.rtt_samples_by_circuit
            .insert("c_one".to_string(), samples(vec![7_000], Vec::new()));
        heap.by_circuit.insert("c_one".to_string(), vec![minute(2_000_000)]);
        // Umbrella-attributed: non-entity.
        heap.rtt_samples_by_circuit.insert(
            "c_orphan".to_string(),
            samples(vec![500_000, 500_000], Vec::new()),
        );
        heap.by_circuit
            .insert("c_orphan".to_string(), vec![minute(2_000_000)]);
        let mut parents: FxHashMap<String, String> = FxHashMap::default();
        parents.insert("c1".to_string(), "WestRedd".to_string());
        parents.insert("c_low".to_string(), "WestRedd".to_string());
        parents.insert("c_one".to_string(), "WestRedd".to_string());
        parents.insert("c_orphan".to_string(), ORPHANS.to_string());

        let snap = snapshot_bridge_rtt(&heap, &parents);
        assert_eq!(snap.folds.len(), 1, "only the qualifying circuit folds");
        assert_eq!(snap.folds[0].1.len(), 2, "download samples route to Download");
        assert!(
            snap.folds[0].2.is_empty(),
            "c1 has no upload samples (never merged into Download)"
        );
        assert_eq!(snap.median_ms_by_circuit.get("c1"), Some(&20.0));
    }
}

#[cfg(test)]
mod w1_do_not_track_tests {
    use super::build_do_not_track_table;
    use std::net::IpAddr;

    fn ips(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    /// §`lqos-74t.31.1` W1 oracle: a configured do-not-track CIDR MATCHES the IPs
    /// inside it and NOT the ones outside (the flow/RTT suppression predicate).
    #[test]
    fn a_do_not_track_cidr_matches_inside_and_not_outside() {
        let table = build_do_not_track_table(&ips(&["203.0.113.0/24", "2001:db8::/32"]))
            .expect("a parsed set builds a table");
        let inside_v4: IpAddr = "203.0.113.7".parse().unwrap();
        let outside_v4: IpAddr = "198.51.100.7".parse().unwrap();
        let inside_v6: IpAddr = "2001:db8::1".parse().unwrap();
        assert!(table.longest_match(inside_v4).is_some(), "inside V4 is blocked");
        assert!(table.longest_match(inside_v6).is_some(), "inside V6 is blocked");
        assert!(
            table.longest_match(outside_v4).is_none(),
            "outside the CIDR is unaffected"
        );
    }

    #[test]
    fn an_empty_or_unparseable_do_not_track_set_is_none() {
        assert!(build_do_not_track_table(&[]).is_none());
        assert!(
            build_do_not_track_table(&ips(&["not-a-cidr", "10.0.0.0"])).is_none(),
            "no parseable entry ⇒ track everything"
        );
    }

    #[test]
    fn a_single_host_cidr_matches_only_that_host() {
        let table = build_do_not_track_table(&ips(&["192.0.2.10/32"])).unwrap();
        let inside: IpAddr = "192.0.2.10".parse().unwrap();
        let outside: IpAddr = "192.0.2.11".parse().unwrap();
        assert!(table.longest_match(inside).is_some());
        assert!(table.longest_match(outside).is_none());
    }

    /// §`lqos-74t.31.1`: the poll filter drops a do-not-track IP from EVERY host
    /// and flow vector — including the `.49.1` best-effort/unmapped class (the
    /// unknown-IPs page source) — and leaves a non-matching IP in place.
    #[test]
    fn the_poll_filter_drops_do_not_track_hosts_and_flows() {
        use analytics_ipc::{AnalyticsSnapshot, FlowId, FlowRow, HostId, HostRow};
        let blocked: IpAddr = "203.0.113.7".parse().unwrap();
        let kept: IpAddr = "198.51.100.7".parse().unwrap();
        let host = |ip: IpAddr| HostRow {
            id: HostId {
                subscriber_ip: ip,
                ..Default::default()
            },
            ..Default::default()
        };
        let flow = |ip: IpAddr| FlowRow {
            id: FlowId {
                subscriber_ip: ip,
                remote_ip: kept,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut snap = AnalyticsSnapshot {
            hosts: vec![host(blocked), host(kept)],
            active_flows: vec![flow(blocked), flow(kept)],
            recent_flows: vec![flow(blocked), flow(kept)],
            best_effort_hosts: vec![host(blocked), host(kept)],
            ..Default::default()
        };
        let table = super::build_do_not_track_table(&ips(&["203.0.113.0/24"])).unwrap();
        super::apply_do_not_track_filter(&mut snap, &table);
        assert_eq!(snap.hosts.len(), 1);
        assert_eq!(snap.hosts[0].id.subscriber_ip, kept);
        assert_eq!(snap.active_flows.len(), 1);
        assert_eq!(snap.recent_flows.len(), 1);
        assert_eq!(
            snap.best_effort_hosts.len(),
            1,
            "the unknown-IPs (best-effort) source is filtered too"
        );
        assert_eq!(snap.best_effort_hosts[0].id.subscriber_ip, kept);
    }
}

#[cfg(test)]
mod raw_queue_tests {
    use super::*;
    use analytics_ipc::{QdiscDetailRow, QdiscDirectionRow, QdiscTinRow, QueueStatsReply};

    /// The cache is process-global; serialize the tests that seed/evict it.
    static RAW_QUEUE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        RAW_QUEUE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Scoped bridge-active override for the raw-queue tests. It replaces the
    /// old `STARTED.set(())` (a process-global OnceLock that could not be
    /// reset) so these tests keep bridge-active coverage without leaking the
    /// active state into the rest of the test binary.
    struct TestActiveGuard(bool);

    impl Drop for TestActiveGuard {
        fn drop(&mut self) {
            super::set_test_active(self.0);
        }
    }

    fn active_for_test() -> TestActiveGuard {
        TestActiveGuard(super::set_test_active(true))
    }

    fn row(sent: u64, tins: bool) -> QdiscDetailRow {
        QdiscDetailRow {
            slot_id: 7,
            download: QdiscDirectionRow {
                tins: if tins {
                    vec![QdiscTinRow {
                        sent_bytes: sent,
                        backlog_bytes: 11,
                        sojourn_p99_ns: 22,
                    }]
                } else {
                    Vec::new()
                },
                memory_used: 33,
                queue_depth_pkts: 4,
                drop_pkts: 5,
                drop_pkts_queue_full: 1,
                aqm_marks: 6,
            },
            upload: QdiscDirectionRow::default(),
            rtt_histogram: Default::default(),
        }
    }

    fn reply(sampled_at: Option<u64>, seq: u64, sent: u64, tins: bool) -> QueueStatsReply {
        QueueStatsReply {
            rows: vec![row(sent, tins)],
            sampled_at_unix_ms: sampled_at,
            elapsed_ms: Some(200),
            cadence_ms: 200,
            sample_epoch_unix_ms: 1,
            sample_seq: seq,
            actor_gap: false,
            counter_reset: false,
        }
    }

    fn seed(circuit_id: &str, reply: &QueueStatsReply) {
        let entry = raw_queue_entry(&reply.rows[0], reply, Instant::now());
        let mut cache = RAW_QUEUE_CACHE.lock();
        cache.note_interest(circuit_id);
        cache.put_entry(circuit_id, entry);
    }

    #[test]
    fn snapshot_is_cache_only_zero_socket_on_hit_and_miss() {
        let _active = active_for_test();
        let _guard = test_lock();
        let before = analytics_ipc::queue_stats_socket_calls();
        assert!(raw_queue_snapshot("cold-circuit").is_none());
        assert_eq!(
            analytics_ipc::queue_stats_socket_calls(),
            before,
            "a miss must not call the analytics socket"
        );
        assert!(
            RAW_QUEUE_CACHE.lock().interest.contains("cold-circuit"),
            "a miss records bounded interest for the next actor poll"
        );
        let now = unix_now_ms();
        let r = reply(Some(now.saturating_sub(100)), 42, 4_242, true);
        seed("c1", &r);
        let transit = raw_queue_snapshot("c1").expect("cache hit");
        assert_eq!(
            analytics_ipc::queue_stats_socket_calls(),
            before,
            "a hit must not call the analytics socket"
        );
        let snapshot = transit.snapshot.expect("snapshot");
        assert_eq!(snapshot.sample_seq, 42, "row+metadata from the same reply");
        assert_eq!(snapshot.download.sent_bytes, Some(4_242));
        assert!(snapshot.download.counters_available);
        assert!(snapshot.download.tins_available);
        assert_eq!(snapshot.download.kind, "cake");
        assert!(transit.history.is_empty());
        assert_eq!(transit.history_len, 0);
        assert_eq!(transit.history_head, 0);
        assert!(transit.watch_expires_at_unix_ms.is_none());
        assert!(!snapshot.stale);
        assert!(!snapshot.clock_step);
    }

    #[test]
    fn non_cake_counters_are_none_not_zero() {
        let _active = active_for_test();
        let _guard = test_lock();
        let r = reply(Some(unix_now_ms().saturating_sub(10)), 7, 0, false);
        seed("fifo", &r);
        let snapshot = raw_queue_snapshot("fifo").unwrap().snapshot.unwrap();
        let down = &snapshot.download;
        assert_eq!(down.kind, "none");
        assert!(!down.tins_available);
        assert!(!down.counters_available);
        assert_eq!(down.sent_bytes, None);
        assert_eq!(
            down.drop_pkts, None,
            "non-CAKE drops are not exposed as valid"
        );
        assert_eq!(down.aqm_marks, None);
        assert_eq!(snapshot.upload.sent_bytes, None);
        // Gauges stay point-in-time.
        assert_eq!(down.memory_used, 33);
    }

    #[test]
    fn backward_clock_step_is_explicit_not_zero_age() {
        let _active = active_for_test();
        let _guard = test_lock();
        let r = reply(Some(unix_now_ms().saturating_add(60_000)), 9, 10, true);
        seed("future", &r);
        let snapshot = raw_queue_snapshot("future").unwrap().snapshot.unwrap();
        assert!(snapshot.clock_step);
        assert_eq!(snapshot.sample_age_ms, None);
        assert!(snapshot.invalid_window);
        assert!(snapshot.stale);
    }

    #[test]
    fn a_reply_without_sample_time_is_unavailable() {
        let _active = active_for_test();
        let _guard = test_lock();
        let r = reply(None, 1, 10, true);
        seed("old-bridge", &r);
        assert!(
            raw_queue_snapshot("old-bridge").is_none(),
            "an old bridge with no provenance is explicit unavailable"
        );
    }

    #[test]
    fn entries_and_interest_are_dropped_when_a_circuit_leaves() {
        let _active = active_for_test();
        let _guard = test_lock();
        let r = reply(Some(unix_now_ms()), 3, 1, true);
        seed("gone", &r);
        RAW_QUEUE_CACHE.lock().retain_live(&FxHashSet::default());
        assert!(
            !RAW_QUEUE_CACHE.lock().entries.contains_key("gone"),
            "the entry is dropped when the circuit leaves the snapshot"
        );
        assert!(
            !RAW_QUEUE_CACHE.lock().interest.contains("gone"),
            "the interest is dropped too"
        );
        // A later request is an explicit miss (and re-records interest).
        assert!(raw_queue_snapshot("gone").is_none());
    }

    #[test]
    fn the_last_reply_cache_is_bounded() {
        let _active = active_for_test();
        let _guard = test_lock();
        let r = reply(Some(unix_now_ms()), 1, 1, true);
        let cap = raw_queue_cache_cap();
        for index in 0..(cap + 50) {
            seed(&format!("cap-{index}"), &r);
        }
        assert!(RAW_QUEUE_CACHE.lock().entries.len() <= cap);
        assert!(RAW_QUEUE_CACHE.lock().interest.len() <= cap);
    }

    #[test]
    fn an_old_payload_decodes_with_snapshot_none() {
        let old = r#"{"history":[],"history_head":0,
            "current_download":{"memory_used":1},
            "current_upload":{"memory_used":2},
            "kind_down":"cake","kind_up":"cake","per_direction":true}"#;
        let transit: QueueStoreTransit =
            serde_json::from_str(&old.replace('\n', "")).expect("old payload decodes");
        assert!(transit.snapshot.is_none());
        assert_eq!(transit.history_len, 0);
        assert!(transit.watch_expires_at_unix_ms.is_none());
    }
}
