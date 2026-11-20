use crate::node_manager::local_api::executive::{
    ExecutiveDashboardMetricRow, ExecutiveDashboardSummary, ExecutiveEntityKind,
    ExecutiveHeatmapPageRow, ExecutiveLeaderboardKind, ExecutiveLeaderboardRow, ExecutiveMetric,
    ExecutiveOversubscribedSiteRow, ExecutiveRttBlocks, ExecutiveScalarBlocks,
    ExecutiveSplitBlocks, ExecutiveTopAsnRow, ExecutiveTreeLocator,
};
use crate::throughput_tracker::{
    THROUGHPUT_TRACKER, asn_heatmaps, circuit_heatmaps, executive_summary_header, global_heatmap,
    site_heatmaps,
};
use arc_swap::ArcSwap;
use fxhash::{FxHashMap, FxHashSet};
use lqos_bus::{
    AsnHeatmapData, BusResponse, CircuitHeatmapData, ExecutiveSummaryHeader, SiteHeatmapData,
};
use lqos_utils::{
    HeatmapBlocks,
    qoq_heatmap::{QoqHeatmapBlocks, TemporalQoqHeatmap},
    temporal_heatmap::TemporalHeatmap,
};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const EXECUTIVE_TOP_LIMIT: usize = 10;
const EXECUTIVE_OVERSUBSCRIBED_LIMIT: usize = 20;
const MIN_SAMPLES_FOR_ALERTS: usize = 10;
const DUE_UPGRADE_THRESHOLD: f32 = 80.0;
const COUNT_WEIGHT: f32 = 100.0;
const MIN_HEATMAP_SAMPLES: usize = 3;
const HEATMAP_BLOCK_SECONDS: u64 = 60;
const BITS_PER_MEGABIT: f32 = 1_000_000.0;
/// Head-room for the id half of a cached `row_key` (see `prefixed_row_key`).
const ROW_KEY_ID_HINT: usize = 16;

#[derive(Clone, Debug)]
pub(crate) struct ExecutiveEntitySnapshot {
    pub row_key: String,
    pub entity_kind: ExecutiveEntityKind,
    pub label: String,
    pub circuit_id: Option<String>,
    pub asn: Option<u32>,
    pub tree: Option<ExecutiveTreeLocator>,
    pub heatmap: HeatmapBlocks,
    pub qoq_blocks: Option<QoqHeatmapBlocks>,
}

/// Shared once-per-second executive cache used by dashboard summary and detail pages.
#[derive(Clone, Debug)]
pub(crate) struct ExecutiveCacheSnapshot {
    pub generated_at_unix_ms: u64,
    pub dashboard: ExecutiveDashboardSummary,
    pub entities: Vec<ExecutiveEntitySnapshot>,
    pub site_entity_index: FxHashMap<String, usize>,
    pub circuit_entity_index: FxHashMap<String, usize>,
    pub leaderboard_rows: FxHashMap<ExecutiveLeaderboardKind, Vec<ExecutiveLeaderboardRow>>,
}

impl Default for ExecutiveCacheSnapshot {
    fn default() -> Self {
        Self {
            generated_at_unix_ms: 0,
            dashboard: ExecutiveDashboardSummary {
                generated_at_unix_ms: 0,
                header: ExecutiveSummaryHeader::default(),
                global: TemporalHeatmap::new().blocks(),
                global_qoq: TemporalQoqHeatmap::new().blocks(),
                top_download: Vec::new(),
                top_upload: Vec::new(),
                top_download_sites: Vec::new(),
                top_upload_sites: Vec::new(),
                top_download_circuits: Vec::new(),
                top_upload_circuits: Vec::new(),
                top_retransmit: Vec::new(),
                top_retransmit_sites: Vec::new(),
                top_rtt: Vec::new(),
                top_rtt_sites: Vec::new(),
                oversubscribed_sites: Vec::new(),
                top_asns: Vec::new(),
            },
            entities: Vec::new(),
            site_entity_index: FxHashMap::default(),
            circuit_entity_index: FxHashMap::default(),
            leaderboard_rows: FxHashMap::default(),
        }
    }
}

impl ExecutiveCacheSnapshot {
    /// Returns the cached entity row for a site name, using case-insensitive exact matching.
    pub(crate) fn site_entity(&self, site_name: &str) -> Option<&ExecutiveEntitySnapshot> {
        let normalized_site_name = site_name.trim().to_lowercase();
        if normalized_site_name.is_empty() {
            return None;
        }
        self.site_entity_index
            .get(&normalized_site_name)
            .and_then(|idx| self.entities.get(*idx))
    }

    /// Returns the cached entity row for an exact circuit ID.
    pub(crate) fn circuit_entity(&self, circuit_id: &str) -> Option<&ExecutiveEntitySnapshot> {
        let circuit_id = circuit_id.trim();
        if circuit_id.is_empty() {
            return None;
        }
        self.circuit_entity_index
            .get(circuit_id)
            .and_then(|idx| self.entities.get(*idx))
    }

    /// Builds transport rows for one requested metric from cached raw executive entities.
    pub(crate) fn heatmap_rows_for_metric(
        &self,
        metric: &ExecutiveMetric,
    ) -> Vec<ExecutiveHeatmapPageRow> {
        self.entities
            .iter()
            .map(|entity| ExecutiveHeatmapPageRow {
                row_key: entity.row_key.clone(),
                entity_kind: entity.entity_kind.clone(),
                label: entity.label.clone(),
                circuit_id: entity.circuit_id.clone(),
                asn: entity.asn,
                tree: entity.tree.clone(),
                scalar_blocks: heatmap_scalar_blocks(metric, &entity.heatmap),
                split_blocks: match metric {
                    ExecutiveMetric::Retransmit => Some(ExecutiveSplitBlocks {
                        download: array_to_vec(&entity.heatmap.retransmit_down),
                        upload: array_to_vec(&entity.heatmap.retransmit_up),
                    }),
                    ExecutiveMetric::Download | ExecutiveMetric::Upload => {
                        Some(ExecutiveSplitBlocks {
                            download: array_to_vec(&entity.heatmap.download),
                            upload: array_to_vec(&entity.heatmap.upload),
                        })
                    }
                    ExecutiveMetric::Rtt => None,
                },
                rtt_blocks: match metric {
                    ExecutiveMetric::Rtt => Some(heatmap_to_rtt_blocks(&entity.heatmap)),
                    _ => None,
                },
            })
            .collect()
    }
}

pub(crate) fn build_entity_indexes(
    entities: &[ExecutiveEntitySnapshot],
) -> (FxHashMap<String, usize>, FxHashMap<String, usize>) {
    let mut site_entity_index = FxHashMap::default();
    let mut circuit_entity_index = FxHashMap::default();
    for (idx, entity) in entities.iter().enumerate() {
        match entity.entity_kind {
            ExecutiveEntityKind::Site => {
                site_entity_index
                    .entry(entity.label.trim().to_lowercase())
                    .or_insert(idx);
            }
            ExecutiveEntityKind::Circuit => {
                if let Some(circuit_id) = entity.circuit_id.as_ref() {
                    circuit_entity_index
                        .entry(circuit_id.trim().to_string())
                        .or_insert(idx);
                }
            }
            ExecutiveEntityKind::Asn => {}
        }
    }
    (site_entity_index, circuit_entity_index)
}

static EXECUTIVE_CACHE_SNAPSHOT: Lazy<ArcSwap<ExecutiveCacheSnapshot>> =
    Lazy::new(|| ArcSwap::new(Arc::new(ExecutiveCacheSnapshot::default())));
static EXECUTIVE_CACHE_LAST_REFRESH_SECS: AtomicU64 = AtomicU64::new(0);
static EXECUTIVE_CACHE_REFRESH_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn current_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn array_to_vec<const N: usize>(values: &[Option<f32>; N]) -> Vec<Option<f32>> {
    values.to_vec()
}

fn heatmap_scalar_blocks(
    metric: &ExecutiveMetric,
    heatmap: &HeatmapBlocks,
) -> Option<ExecutiveScalarBlocks> {
    let values = match metric {
        ExecutiveMetric::Download => Some(array_to_vec(&heatmap.download)),
        ExecutiveMetric::Upload => Some(array_to_vec(&heatmap.upload)),
        ExecutiveMetric::Retransmit => Some(array_to_vec(&heatmap.retransmit)),
        ExecutiveMetric::Rtt => Some(array_to_vec(&heatmap.rtt)),
    }?;
    Some(ExecutiveScalarBlocks { values })
}

fn heatmap_to_rtt_blocks(blocks: &HeatmapBlocks) -> ExecutiveRttBlocks {
    ExecutiveRttBlocks {
        rtt: array_to_vec(&blocks.rtt),
        dl_p50: array_to_vec(&blocks.rtt_p50_down),
        dl_p90: array_to_vec(&blocks.rtt_p90_down),
        ul_p50: array_to_vec(&blocks.rtt_p50_up),
        ul_p90: array_to_vec(&blocks.rtt_p90_up),
    }
}

fn latest_value(values: &[Option<f32>]) -> Option<f32> {
    values.iter().rev().flatten().copied().next()
}

fn non_null_count(values: &[Option<f32>]) -> usize {
    values.iter().filter(|value| value.is_some()).count()
}

fn sort_score_for_metric(entity: &ExecutiveEntitySnapshot, metric: &ExecutiveMetric) -> f32 {
    match metric {
        ExecutiveMetric::Download => {
            latest_value(&entity.heatmap.download).unwrap_or(f32::NEG_INFINITY)
        }
        ExecutiveMetric::Upload => {
            latest_value(&entity.heatmap.upload).unwrap_or(f32::NEG_INFINITY)
        }
        ExecutiveMetric::Retransmit => {
            let latest = latest_value(&entity.heatmap.retransmit).unwrap_or(f32::NEG_INFINITY);
            let count = non_null_count(&entity.heatmap.retransmit);
            let penalty = if count < MIN_HEATMAP_SAMPLES {
                1000.0
            } else {
                0.0
            };
            latest + COUNT_WEIGHT * (count as f32 / 15.0) - penalty
        }
        ExecutiveMetric::Rtt => {
            // `.64.2.4.2`: rank on the LATEST non-null value (like the other
            // panels); a no-RTT row scores `-INF` and sorts LAST, so the panel
            // fills to the limit without a filter.
            let latest = latest_value(&entity.heatmap.rtt).unwrap_or(f32::NEG_INFINITY);
            let count = non_null_count(&entity.heatmap.rtt);
            let penalty = if count < MIN_HEATMAP_SAMPLES {
                1000.0
            } else {
                0.0
            };
            latest + COUNT_WEIGHT * (count as f32 / 15.0) - penalty
        }
    }
}

fn metric_row_from_entity(
    entity: &ExecutiveEntitySnapshot,
    metric: ExecutiveMetric,
) -> ExecutiveDashboardMetricRow {
    ExecutiveDashboardMetricRow {
        row_key: entity.row_key.clone(),
        entity_kind: entity.entity_kind.clone(),
        metric: metric.clone(),
        label: entity.label.clone(),
        circuit_id: entity.circuit_id.clone(),
        asn: entity.asn,
        tree: entity.tree.clone(),
        scalar_blocks: heatmap_scalar_blocks(&metric, &entity.heatmap),
        split_blocks: match metric {
            ExecutiveMetric::Download | ExecutiveMetric::Upload => Some(ExecutiveSplitBlocks {
                download: array_to_vec(&entity.heatmap.download),
                upload: array_to_vec(&entity.heatmap.upload),
            }),
            ExecutiveMetric::Retransmit => Some(ExecutiveSplitBlocks {
                download: array_to_vec(&entity.heatmap.retransmit_down),
                upload: array_to_vec(&entity.heatmap.retransmit_up),
            }),
            ExecutiveMetric::Rtt => None,
        },
        rtt_blocks: match metric {
            ExecutiveMetric::Rtt => Some(heatmap_to_rtt_blocks(&entity.heatmap)),
            _ => None,
        },
    }
}

/// `.64.3.2`: the entity-kind scope for a ranked metric list. `Site` covers Sites
/// AND APs (APs are `entity_kind=Site`); `Asn` is never included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MetricScope {
    All,
    Site,
    Circuit,
}

fn scope_allows(scope: MetricScope, kind: &ExecutiveEntityKind) -> bool {
    match scope {
        MetricScope::All => matches!(
            kind,
            ExecutiveEntityKind::Site | ExecutiveEntityKind::Circuit
        ),
        MetricScope::Site => matches!(kind, ExecutiveEntityKind::Site),
        MetricScope::Circuit => matches!(kind, ExecutiveEntityKind::Circuit),
    }
}

fn top_metric_rows(
    entities: &[ExecutiveEntitySnapshot],
    metric: ExecutiveMetric,
    scope: MetricScope,
    limit: usize,
) -> Vec<ExecutiveDashboardMetricRow> {
    // `.64.2.4.2` PARITY: every dashboard metric list is UNFILTERED. The Rtt arm
    // ranks on the latest non-null value with a `-INF` (rank-LAST) fallback, so
    // the pool is every Site|Circuit entity and the panel yields
    // `min(limit, count)` rows, with the no-RTT rows at the BOTTOM (blank cells).
    // The Rtt arm keeps COUNT_WEIGHT + the sparse-sample penalty
    // (retransmit-style; utilization has no penalty). It deliberately RE-ACCEPTS
    // blank rows (parity over no-blanks — the DRIVER's call).
    let mut ranked = entities
        .iter()
        .filter(|entity| scope_allows(scope, &entity.entity_kind))
        .cloned()
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        sort_score_for_metric(right, &metric)
            .partial_cmp(&sort_score_for_metric(left, &metric))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.label.cmp(&right.label))
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|entity| metric_row_from_entity(&entity, metric.clone()))
        .collect()
}

fn total_bytes_from_heatmap(blocks: &HeatmapBlocks) -> u64 {
    let total_mbps_minutes: f32 = blocks
        .download
        .iter()
        .chain(blocks.upload.iter())
        .flatten()
        .copied()
        .sum();
    ((total_mbps_minutes * BITS_PER_MEGABIT / 8.0) * HEATMAP_BLOCK_SECONDS as f32) as u64
}

fn median_value(values: &[Option<f32>]) -> Option<f32> {
    let mut numeric = values.iter().flatten().copied().collect::<Vec<_>>();
    if numeric.is_empty() {
        return None;
    }
    numeric.sort_by(|left, right| left.total_cmp(right));
    let midpoint = numeric.len() / 2;
    if numeric.len() % 2 == 1 {
        Some(numeric[midpoint])
    } else {
        Some((numeric[midpoint - 1] + numeric[midpoint]) / 2.0)
    }
}

fn average_with_count(values: &[Option<f32>]) -> (Option<f32>, usize) {
    let numeric = values.iter().flatten().copied().collect::<Vec<_>>();
    if numeric.is_empty() {
        return (None, 0);
    }
    let sum = numeric.iter().sum::<f32>();
    (Some(sum / numeric.len() as f32), numeric.len())
}

fn build_top_asn_rows(entities: &[ExecutiveEntitySnapshot]) -> Vec<ExecutiveTopAsnRow> {
    let mut rows = entities
        .iter()
        .filter(|entity| entity.entity_kind == ExecutiveEntityKind::Asn)
        .map(|entity| ExecutiveTopAsnRow {
            row_key: entity.row_key.clone(),
            asn: entity.asn.unwrap_or_default(),
            asn_name: match entity.label.as_str() {
                label if label.starts_with("ASN ") => None,
                label => Some(label.to_string()),
            },
            total_bytes_15m: total_bytes_from_heatmap(&entity.heatmap),
            median_rtt_ms: median_value(&entity.heatmap.rtt),
            median_retransmit_pct: median_value(&entity.heatmap.retransmit),
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| std::cmp::Reverse(row.total_bytes_15m));
    rows.truncate(EXECUTIVE_TOP_LIMIT);
    rows
}

fn build_leaderboard_rows(
    entities: &[ExecutiveEntitySnapshot],
    oversubscribed_sites: &[ExecutiveOversubscribedSiteRow],
) -> FxHashMap<ExecutiveLeaderboardKind, Vec<ExecutiveLeaderboardRow>> {
    let mut rows = FxHashMap::default();

    let mut worst_sites = entities
        .iter()
        .filter(|entity| entity.entity_kind == ExecutiveEntityKind::Site)
        .filter_map(|entity| {
            let median_rtt_ms = median_value(&entity.heatmap.rtt)?;
            let (avg_down_util, _) = average_with_count(&entity.heatmap.download);
            let (avg_up_util, _) = average_with_count(&entity.heatmap.upload);
            Some(ExecutiveLeaderboardRow::WorstSiteByRtt {
                row_key: entity.row_key.clone(),
                site_name: entity.label.clone(),
                tree: entity.tree.clone(),
                median_rtt_ms,
                avg_down_util,
                avg_up_util,
            })
        })
        .collect::<Vec<_>>();
    worst_sites.sort_by(|left, right| match (left, right) {
        (
            ExecutiveLeaderboardRow::WorstSiteByRtt {
                median_rtt_ms: left,
                ..
            },
            ExecutiveLeaderboardRow::WorstSiteByRtt {
                median_rtt_ms: right,
                ..
            },
        ) => right.total_cmp(left),
        _ => std::cmp::Ordering::Equal,
    });
    rows.insert(ExecutiveLeaderboardKind::WorstSitesByRtt, worst_sites);

    rows.insert(
        ExecutiveLeaderboardKind::OversubscribedSites,
        oversubscribed_sites
            .iter()
            .cloned()
            .map(|row| ExecutiveLeaderboardRow::OversubscribedSite {
                row_key: row.row_key,
                site_name: row.site_name,
                tree: row.tree,
                ratio_down: row.ratio_down,
                ratio_up: row.ratio_up,
                ratio_max: row.ratio_max,
                cap_down: row.cap_down,
                cap_up: row.cap_up,
                sub_down: row.sub_down,
                sub_up: row.sub_up,
                avg_down_util: row.avg_down_util,
                avg_up_util: row.avg_up_util,
                median_rtt_ms: row.median_rtt_ms,
            })
            .collect(),
    );

    let mut sites_due_upgrade = entities
        .iter()
        .filter(|entity| entity.entity_kind == ExecutiveEntityKind::Site)
        .filter_map(|entity| {
            let (avg_down_util, down_count) = average_with_count(&entity.heatmap.download);
            let (avg_up_util, up_count) = average_with_count(&entity.heatmap.upload);
            let (Some(avg_down_util), Some(avg_up_util)) = (avg_down_util, avg_up_util) else {
                return None;
            };
            if down_count < MIN_SAMPLES_FOR_ALERTS
                || up_count < MIN_SAMPLES_FOR_ALERTS
                || avg_down_util < DUE_UPGRADE_THRESHOLD
                || avg_up_util < DUE_UPGRADE_THRESHOLD
            {
                return None;
            }
            Some(ExecutiveLeaderboardRow::SiteDueUpgrade {
                row_key: entity.row_key.clone(),
                site_name: entity.label.clone(),
                tree: entity.tree.clone(),
                avg_down_util,
                avg_up_util,
            })
        })
        .collect::<Vec<_>>();
    sites_due_upgrade.sort_by(|left, right| match (left, right) {
        (
            ExecutiveLeaderboardRow::SiteDueUpgrade {
                avg_down_util: left_down,
                avg_up_util: left_up,
                ..
            },
            ExecutiveLeaderboardRow::SiteDueUpgrade {
                avg_down_util: right_down,
                avg_up_util: right_up,
                ..
            },
        ) => (right_down + right_up)
            .partial_cmp(&(left_down + left_up))
            .unwrap_or(std::cmp::Ordering::Equal),
        _ => std::cmp::Ordering::Equal,
    });
    rows.insert(ExecutiveLeaderboardKind::SitesDueUpgrade, sites_due_upgrade);

    let mut circuits_due_upgrade = entities
        .iter()
        .filter(|entity| entity.entity_kind == ExecutiveEntityKind::Circuit)
        .filter_map(|entity| {
            let (avg_down_util, down_count) = average_with_count(&entity.heatmap.download);
            let (avg_up_util, up_count) = average_with_count(&entity.heatmap.upload);
            let (Some(avg_down_util), Some(avg_up_util)) = (avg_down_util, avg_up_util) else {
                return None;
            };
            if down_count < MIN_SAMPLES_FOR_ALERTS
                || up_count < MIN_SAMPLES_FOR_ALERTS
                || avg_down_util < DUE_UPGRADE_THRESHOLD
                || avg_up_util < DUE_UPGRADE_THRESHOLD
            {
                return None;
            }
            Some(ExecutiveLeaderboardRow::CircuitDueUpgrade {
                row_key: entity.row_key.clone(),
                circuit_id: entity.circuit_id.clone().unwrap_or_default(),
                circuit_name: entity.label.clone(),
                avg_down_util,
                avg_up_util,
            })
        })
        .collect::<Vec<_>>();
    circuits_due_upgrade.sort_by(|left, right| match (left, right) {
        (
            ExecutiveLeaderboardRow::CircuitDueUpgrade {
                avg_down_util: left_down,
                avg_up_util: left_up,
                ..
            },
            ExecutiveLeaderboardRow::CircuitDueUpgrade {
                avg_down_util: right_down,
                avg_up_util: right_up,
                ..
            },
        ) => (right_down + right_up)
            .partial_cmp(&(left_down + left_up))
            .unwrap_or(std::cmp::Ordering::Equal),
        _ => std::cmp::Ordering::Equal,
    });
    rows.insert(
        ExecutiveLeaderboardKind::CircuitsDueUpgrade,
        circuits_due_upgrade,
    );

    let mut top_asns = build_top_asn_rows(entities)
        .into_iter()
        .map(|row| ExecutiveLeaderboardRow::TopAsnByTraffic {
            row_key: row.row_key,
            asn: row.asn,
            asn_name: row.asn_name,
            total_bytes_15m: row.total_bytes_15m,
            median_rtt_ms: row.median_rtt_ms,
            median_retransmit_pct: row.median_retransmit_pct,
        })
        .collect::<Vec<_>>();
    top_asns.sort_by(|left, right| match (left, right) {
        (
            ExecutiveLeaderboardRow::TopAsnByTraffic {
                total_bytes_15m: left,
                ..
            },
            ExecutiveLeaderboardRow::TopAsnByTraffic {
                total_bytes_15m: right,
                ..
            },
        ) => right.cmp(left),
        _ => std::cmp::Ordering::Equal,
    });
    rows.insert(ExecutiveLeaderboardKind::TopAsnsByTraffic, top_asns);

    rows
}

fn heatmap_blocks_from_response(response: BusResponse) -> HeatmapBlocks {
    match response {
        BusResponse::GlobalHeatmap(blocks) => blocks,
        _ => TemporalHeatmap::new().blocks(),
    }
}

fn header_from_response(response: BusResponse) -> ExecutiveSummaryHeader {
    match response {
        BusResponse::ExecutiveSummaryHeader(header) => header,
        _ => ExecutiveSummaryHeader::default(),
    }
}

fn circuit_heatmap_rows() -> Vec<CircuitHeatmapData> {
    match circuit_heatmaps() {
        BusResponse::CircuitHeatmaps(rows) => rows,
        _ => Vec::new(),
    }
}

fn site_heatmap_rows() -> Vec<SiteHeatmapData> {
    match site_heatmaps() {
        BusResponse::SiteHeatmaps(rows) => rows,
        _ => Vec::new(),
    }
}

fn asn_heatmap_rows() -> Vec<AsnHeatmapData> {
    match asn_heatmaps() {
        BusResponse::AsnHeatmaps(rows) => rows,
        _ => Vec::new(),
    }
}

fn node_path_names(idx: usize, nodes: &[lqos_config::NetworkJsonNode]) -> Option<Vec<String>> {
    let node = nodes.get(idx)?;
    if node.name == "Root" {
        return Some(vec!["Root".to_string()]);
    }
    let parent_indexes = if node.parents.is_empty() {
        vec![node.immediate_parent.unwrap_or(0)]
    } else {
        node.parents.clone()
    };
    let mut names = Vec::new();
    for parent_idx in parent_indexes {
        names.push(nodes.get(parent_idx)?.name.clone());
    }
    if names.last() != Some(&node.name) {
        names.push(node.name.clone());
    }
    Some(names)
}

fn node_locator_map() -> FxHashMap<String, ExecutiveTreeLocator> {
    lqos_network_devices::with_network_json_read(|net_json| {
        let nodes = net_json.get_nodes_when_ready();
        let mut locators = FxHashMap::default();
        for (idx, node) in nodes.iter().enumerate() {
            if node.name == "Root" || locators.contains_key(&node.name) {
                continue;
            }
            locators.insert(
                node.name.clone(),
                ExecutiveTreeLocator {
                    node_id: node.id.clone(),
                    node_path: node_path_names(idx, nodes),
                    parent_index: Some(idx),
                },
            );
        }
        locators
    })
}

/// The client-visible identity of one cached row, in ONE buffer: `format!` writes
/// through a growable formatter and billed a second allocation for keys of this
/// shape (measured in `entity_snapshots_bills_one_key_buffer_per_entity`), and the
/// once-per-second rebuild pays that per entity. An id longer than the hint costs
/// one growth reallocation — still one buffer, no clone.
fn prefixed_row_key(prefix: &str, id: impl std::fmt::Display) -> String {
    let mut key = String::with_capacity(prefix.len() + ROW_KEY_ID_HINT);
    key.push_str(prefix);
    let _ = write!(key, "{id}"); // Writing into a `String` cannot fail.
    key
}

/// One site row's client-visible key, computed from a BORROW of the locator map.
///
/// Non-unique node names are a SUPPORTED runtime case (`NetworkJson` explicitly
/// counts names unique in both trees, `network.json`'s `id` is optional), and the
/// dashboard collapses rows that share a key. Computing the key before any
/// locator is moved out keeps the pre-W4 key form for EVERY duplicate row, so a
/// duplicated site still collapses instead of surfacing a second entry.
fn site_row_key(locator: Option<&ExecutiveTreeLocator>, site_name: &str) -> String {
    locator
        .and_then(|locator| {
            locator
                .node_id
                .as_ref()
                .map(|node_id| prefixed_row_key("site:", node_id))
                .or_else(|| {
                    // The Debug form of a path is the one key that cannot be
                    // sized up front; it stays a `format!`.
                    locator
                        .node_path
                        .as_ref()
                        .map(|path| format!("site:{:?}", path))
                })
        })
        .unwrap_or_else(|| prefixed_row_key("site:", site_name))
}

/// Builds the cached per-entity rows, CONSUMING the heatmap rows and the locator
/// map: the caller hands over ownership, so every name and tree that lands in a
/// cached row is moved, never cloned.
fn entity_snapshots(
    circuits: Vec<CircuitHeatmapData>,
    sites: Vec<SiteHeatmapData>,
    asns: Vec<AsnHeatmapData>,
) -> Vec<ExecutiveEntitySnapshot> {
    entity_snapshots_with_locators(circuits, sites, asns, node_locator_map())
}

/// Pure core of [`entity_snapshots`]: one `String` per row for the client-visible
/// `row_key`, and no copies of the names or locators the caller already owns.
///
/// Two declared edges of the move rather than clone:
/// - an empty circuit name still needs the id twice (`label` + `circuit_id`), so
///   that one row keeps a single copy;
/// - the locator itself moves out once, so only the first row of a repeated site
///   name carries the tree. Its KEY does not follow the locator: keys are
///   computed from a borrow first (see [`site_row_key`]), so every duplicate row
///   keeps the shared key the pre-W4 pass gave it and the dashboard collapses
///   them, as before.
fn entity_snapshots_with_locators(
    circuits: Vec<CircuitHeatmapData>,
    sites: Vec<SiteHeatmapData>,
    asns: Vec<AsnHeatmapData>,
    mut locators: FxHashMap<String, ExecutiveTreeLocator>,
) -> Vec<ExecutiveEntitySnapshot> {
    let mut rows = Vec::with_capacity(sites.len() + circuits.len() + asns.len());
    // Pass 1: keys, borrowing the locator map (one key buffer per site, exactly
    // the buffer its row ends up owning).
    let mut site_keys = Vec::with_capacity(sites.len());
    for site in &sites {
        site_keys.push(site_row_key(locators.get(&site.site_name), &site.site_name));
    }
    // Pass 2: row bodies, moving the locators and names out.
    for (site, row_key) in sites.into_iter().zip(site_keys) {
        let SiteHeatmapData {
            site_name,
            blocks: heatmap,
            qoq_blocks,
            ..
        } = site;
        let tree = locators.remove(&site_name);
        rows.push(ExecutiveEntitySnapshot {
            row_key,
            entity_kind: ExecutiveEntityKind::Site,
            label: site_name,
            circuit_id: None,
            asn: None,
            tree,
            heatmap,
            qoq_blocks,
        });
    }
    for circuit in circuits {
        let CircuitHeatmapData {
            circuit_id,
            circuit_name,
            blocks: heatmap,
            qoq_blocks,
            ..
        } = circuit;
        let row_key = prefixed_row_key("circuit:", &circuit_id);
        let label = if circuit_name.trim().is_empty() {
            circuit_id.clone()
        } else {
            circuit_name
        };
        rows.push(ExecutiveEntitySnapshot {
            row_key,
            entity_kind: ExecutiveEntityKind::Circuit,
            label,
            circuit_id: Some(circuit_id),
            asn: None,
            tree: None,
            heatmap,
            qoq_blocks,
        });
    }
    for asn in asns {
        let AsnHeatmapData {
            asn: asn_number,
            asn_name,
            blocks: heatmap,
        } = asn;
        rows.push(ExecutiveEntitySnapshot {
            row_key: prefixed_row_key("asn:", asn_number),
            entity_kind: ExecutiveEntityKind::Asn,
            // Same one-buffer rule as the keys: an unnamed ASN's label is built
            // with the prefix helper instead of a `format!` that re-bills.
            label: asn_name.unwrap_or_else(|| prefixed_row_key("ASN ", asn_number)),
            circuit_id: None,
            asn: Some(asn_number),
            tree: None,
            heatmap,
            qoq_blocks: None,
        });
    }
    rows
}

fn build_oversubscribed_sites(
    entities: &[ExecutiveEntitySnapshot],
) -> Vec<ExecutiveOversubscribedSiteRow> {
    let mut circuit_caps =
        lqos_network_devices::shaped_devices_catalog().circuit_rate_caps_by_circuit_id();
    for circuit in lqos_network_devices::dynamic_circuits_snapshot().iter() {
        let shaped = &circuit.shaped;
        circuit_caps
            .entry(shaped.circuit_id.clone())
            .or_insert_with(|| lqos_network_devices::CircuitRateCaps {
                parent_node: shaped.parent_node.clone(),
                download_max_mbps: shaped.download_max_mbps,
                upload_max_mbps: shaped.upload_max_mbps,
            });
    }

    let entity_map = entities
        .iter()
        .filter(|entity| entity.entity_kind == ExecutiveEntityKind::Site)
        .map(|entity| (entity.label.clone(), entity))
        .collect::<FxHashMap<_, _>>();

    lqos_network_devices::with_network_json_read(|net_json| {
        let nodes = net_json.get_nodes_when_ready();
        if nodes.is_empty() {
            return Vec::new();
        }

        let mut children: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
        for (idx, node) in nodes.iter().enumerate() {
            if let Some(parent) = node.immediate_parent
                && parent < children.len()
            {
                children[parent].push(idx);
            }
        }

        let mut results = Vec::new();
        for (idx, node) in nodes.iter().enumerate() {
            if node.name == "Root" {
                continue;
            }
            let node_type = node.node_type.as_deref().unwrap_or("").to_ascii_lowercase();
            if node_type != "site" && !node_type.is_empty() && node_type != "ap" {
                continue;
            }

            let mut stack = vec![idx];
            let mut visited = FxHashSet::default();
            let mut descendant_names = FxHashSet::default();
            while let Some(next_idx) = stack.pop() {
                if !visited.insert(next_idx) {
                    continue;
                }
                if let Some(name) = nodes.get(next_idx).map(|entry| entry.name.clone()) {
                    descendant_names.insert(name);
                }
                if let Some(child_nodes) = children.get(next_idx) {
                    for child in child_nodes {
                        stack.push(*child);
                    }
                }
            }

            let cap_down = node.max_throughput.0 as f32;
            let cap_up = node.max_throughput.1 as f32;

            let mut sub_down = 0.0_f32;
            let mut sub_up = 0.0_f32;
            for caps in circuit_caps.values() {
                if descendant_names.contains(&caps.parent_node) {
                    sub_down += caps.download_max_mbps;
                    sub_up += caps.upload_max_mbps;
                }
            }

            let Some(stats) = entity_map.get(&node.name) else {
                continue;
            };
            let (avg_down_util, _) = average_with_count(&stats.heatmap.download);
            let (avg_up_util, _) = average_with_count(&stats.heatmap.upload);
            let median_rtt_ms = median_value(&stats.heatmap.rtt);
            let ratio_down = (cap_down > 0.0).then_some(sub_down / cap_down);
            let ratio_up = (cap_up > 0.0).then_some(sub_up / cap_up);
            let ratio_max = match (ratio_down, ratio_up) {
                (Some(down), Some(up)) => Some(down.max(up)),
                (Some(down), None) => Some(down),
                (None, Some(up)) => Some(up),
                (None, None) => None,
            };

            results.push(ExecutiveOversubscribedSiteRow {
                row_key: stats.row_key.clone(),
                site_name: node.name.clone(),
                tree: stats.tree.clone(),
                ratio_down,
                ratio_up,
                ratio_max,
                cap_down,
                cap_up,
                sub_down,
                sub_up,
                avg_down_util,
                avg_up_util,
                median_rtt_ms,
            });
        }

        results.sort_by(|left, right| {
            right
                .ratio_max
                .unwrap_or(0.0)
                .total_cmp(&left.ratio_max.unwrap_or(0.0))
        });
        results.truncate(EXECUTIVE_OVERSUBSCRIBED_LIMIT);
        results
    })
}

/// Invalidates the shared executive cache so the next read rebuilds immediately.
pub(crate) fn invalidate_executive_cache_snapshot() {
    EXECUTIVE_CACHE_LAST_REFRESH_SECS.store(0, Ordering::Release);
}

/// Rebuilds the once-per-second executive cache snapshot.
pub(crate) fn rebuild_executive_cache_snapshot() -> Arc<ExecutiveCacheSnapshot> {
    let generated_at_unix_ms = current_unix_ms();
    let header = header_from_response(executive_summary_header());
    let global = heatmap_blocks_from_response(global_heatmap());
    // 3a fill: bridge boxes use the pooled-distribution global QoO.
    let global_qoq = if let Some(snapshot) = crate::bridge_analytics::circuit_qoo_snapshot() {
        snapshot.global_qoq.clone()
    } else {
        THROUGHPUT_TRACKER.global_qoq_heatmap.lock().blocks()
    };
    let entities = entity_snapshots(
        circuit_heatmap_rows(),
        site_heatmap_rows(),
        asn_heatmap_rows(),
    );
    let (site_entity_index, circuit_entity_index) = build_entity_indexes(&entities);
    let oversubscribed_sites = build_oversubscribed_sites(&entities);
    let top_download =
        top_metric_rows(&entities, ExecutiveMetric::Download, MetricScope::All, EXECUTIVE_TOP_LIMIT);
    let top_upload =
        top_metric_rows(&entities, ExecutiveMetric::Upload, MetricScope::All, EXECUTIVE_TOP_LIMIT);
    let top_retransmit =
        top_metric_rows(&entities, ExecutiveMetric::Retransmit, MetricScope::All, EXECUTIVE_TOP_LIMIT);
    let top_rtt =
        top_metric_rows(&entities, ExecutiveMetric::Rtt, MetricScope::All, EXECUTIVE_TOP_LIMIT);
    let top_retransmit_sites =
        top_metric_rows(&entities, ExecutiveMetric::Retransmit, MetricScope::Site, EXECUTIVE_TOP_LIMIT);
    let top_rtt_sites =
        top_metric_rows(&entities, ExecutiveMetric::Rtt, MetricScope::Site, EXECUTIVE_TOP_LIMIT);
    // `.64.3.2`: kind-scoped utilization lists for the dashboard widgets (Sites+APs
    // share the `Site` kind; `Asn` is excluded).
    let top_download_sites =
        top_metric_rows(&entities, ExecutiveMetric::Download, MetricScope::Site, EXECUTIVE_TOP_LIMIT);
    let top_upload_sites =
        top_metric_rows(&entities, ExecutiveMetric::Upload, MetricScope::Site, EXECUTIVE_TOP_LIMIT);
    let top_download_circuits =
        top_metric_rows(&entities, ExecutiveMetric::Download, MetricScope::Circuit, EXECUTIVE_TOP_LIMIT);
    let top_upload_circuits =
        top_metric_rows(&entities, ExecutiveMetric::Upload, MetricScope::Circuit, EXECUTIVE_TOP_LIMIT);
    let top_asns = build_top_asn_rows(&entities);
    let leaderboard_rows = build_leaderboard_rows(&entities, &oversubscribed_sites);

    let snapshot = Arc::new(ExecutiveCacheSnapshot {
        generated_at_unix_ms,
        dashboard: ExecutiveDashboardSummary {
            generated_at_unix_ms,
            header,
            global,
            global_qoq,
            top_download,
            top_upload,
            top_download_sites,
            top_upload_sites,
            top_download_circuits,
            top_upload_circuits,
            top_retransmit,
            top_retransmit_sites,
            top_rtt,
            top_rtt_sites,
            oversubscribed_sites,
            top_asns,
        },
        entities,
        site_entity_index,
        circuit_entity_index,
        leaderboard_rows,
    });
    EXECUTIVE_CACHE_SNAPSHOT.store(snapshot.clone());
    EXECUTIVE_CACHE_LAST_REFRESH_SECS.store(current_epoch_secs(), Ordering::Release);
    snapshot
}

/// Returns the current executive cache snapshot, rebuilding it if stale.
pub(crate) fn fresh_executive_cache_snapshot() -> Arc<ExecutiveCacheSnapshot> {
    let now_secs = current_epoch_secs();
    if EXECUTIVE_CACHE_LAST_REFRESH_SECS.load(Ordering::Acquire) == now_secs {
        return EXECUTIVE_CACHE_SNAPSHOT.load_full();
    }
    let _guard = EXECUTIVE_CACHE_REFRESH_LOCK.lock();
    if EXECUTIVE_CACHE_LAST_REFRESH_SECS.load(Ordering::Acquire) == now_secs {
        return EXECUTIVE_CACHE_SNAPSHOT.load_full();
    }
    rebuild_executive_cache_snapshot()
}

/// TEST-ONLY allocation counter for the W4 pin. It counts allocations made by
/// the CALLING THREAD inside an armed window, so tests that run in parallel on
/// other threads cannot inflate a measurement (a global counter plus `min` over
/// repetitions, the W1+W2 shape, would only approximate that). Disarmed, it
/// costs one thread-local flag check per allocation and forwards everything to
/// `System`.
///
/// The ARMED flag is THREAD-LOCAL for the same reason the counter is: a second
/// test arming a global flag while this one measures would disarm this window
/// mid-run, and the measurement would silently come in low.
// One `#[global_allocator]` per crate, so the harness is crate-visible: W4's
// pins and W5's `bridge_ranking_tests` share this counter.
#[cfg(test)]
pub(crate) mod allocation_counter {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static ARMED: Cell<bool> = const { Cell::new(false) };
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }

    /// Counts the allocations `op` makes on this thread. Disarms even if `op`
    /// panics, so a failing test cannot leave the counter running.
    pub fn count_allocations(op: impl FnOnce()) -> usize {
        struct Disarm;

        impl Drop for Disarm {
            fn drop(&mut self) {
                let _ = ARMED.try_with(|armed| armed.set(false));
            }
        }

        ALLOCATIONS.with(|count| count.set(0));
        ARMED.with(|armed| armed.set(true));
        let disarm = Disarm;
        op();
        drop(disarm);
        ALLOCATIONS.with(Cell::get)
    }

    #[inline]
    fn record() {
        let _ = ARMED.try_with(|armed| {
            if armed.get() {
                let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
            }
        });
    }

    struct CountingAllocator;

    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            record();
            unsafe { System.alloc(layout) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            record();
            unsafe { System.alloc_zeroed(layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            record();
            unsafe { System.realloc(ptr, layout, new_size) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[global_allocator]
    static GLOBAL: CountingAllocator = CountingAllocator;
}

#[cfg(test)]
mod tests {
    use super::{
        ExecutiveEntitySnapshot, MetricScope, entity_snapshots, entity_snapshots_with_locators,
        top_metric_rows,
    };
    use crate::node_manager::local_api::executive::{
        ExecutiveEntityKind, ExecutiveMetric, ExecutiveTreeLocator,
    };
    use fxhash::FxHashMap;
    use lqos_bus::{AsnHeatmapData, CircuitHeatmapData, SiteHeatmapData};
    use lqos_utils::temporal_heatmap::TemporalHeatmap;

    fn rtt_entity(label: &str, rtt_blocks: &[(usize, f32)]) -> ExecutiveEntitySnapshot {
        let mut heatmap = TemporalHeatmap::new().blocks();
        for &(idx, value) in rtt_blocks {
            heatmap.rtt[idx] = Some(value);
        }
        ExecutiveEntitySnapshot {
            row_key: format!("site:{label}"),
            entity_kind: ExecutiveEntityKind::Site,
            label: label.to_string(),
            circuit_id: None,
            asn: None,
            tree: None,
            heatmap,
            qoq_blocks: None,
        }
    }

    #[test]
    fn top_rtt_fills_to_the_limit_and_ranks_no_rtt_rows_last() {
        // `.64.2.4.2` PARITY: the Median RTT panel has NO filter and fills to the
        // limit like the other panels, even when fewer than `limit` entities have
        // RTT. A no-RTT row is INCLUDED and ranked LAST (renders blank at the bottom).
        //
        // `.64.2.4.1` (R4): the >=limit-adjacent pool is IN-WINDOW-BUT-NOT-CURRENT
        // (all at block 5). The OLD `current_rtt` filter required the CURRENT block
        // and would DROP every one of these rows (and the no-RTT rows), starving
        // the panel to 0 -- so `labels.len() == 10` FAILS under the old code and
        // the fill assertion is non-vacuous.
        let mut entities: Vec<ExecutiveEntitySnapshot> = Vec::new();
        // 8 RTT-bearing rows, none in the current block.
        for i in 0..8 {
            entities.push(rtt_entity(&format!("In Window {i:02}"), &[(5, 10.0 + i as f32)]));
        }
        // 2 rows with NO RTT in the window (included, ranked last) -> total = limit.
        for i in 0..2 {
            entities.push(rtt_entity(&format!("No RTT {i}"), &[]));
        }

        let rows = top_metric_rows(&entities, ExecutiveMetric::Rtt, MetricScope::All, 10);

        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        // FILLS to the limit: 8 in-window rows + 2 no-RTT rows = the limit. Under
        // the OLD `current_rtt` filter the 8 non-current rows were dropped => this
        // would be 0, not 10 (non-vacuous).
        assert_eq!(labels.len(), 10, "the panel must fill to the limit: {labels:?}");
        // The no-RTT rows are INCLUDED and sit at the BOTTOM.
        assert!(labels[8].starts_with("No RTT"), "no-RTT rows must rank LAST: {labels:?}");
        assert!(labels[9].starts_with("No RTT"), "no-RTT rows must rank LAST: {labels:?}");
        // The highest-RTT in-window row ranks WORST-first.
        assert_eq!(labels[0], "In Window 07");
    }

    fn util_entity(
        label: &str,
        kind: ExecutiveEntityKind,
        download_blocks: &[(usize, f32)],
        upload_blocks: &[(usize, f32)],
    ) -> ExecutiveEntitySnapshot {
        let mut heatmap = TemporalHeatmap::new().blocks();
        for &(idx, value) in download_blocks {
            heatmap.download[idx] = Some(value);
        }
        for &(idx, value) in upload_blocks {
            heatmap.upload[idx] = Some(value);
        }
        ExecutiveEntitySnapshot {
            row_key: format!("{kind:?}:{label}"),
            entity_kind: kind,
            label: label.to_string(),
            circuit_id: None,
            asn: None,
            tree: None,
            heatmap,
            qoq_blocks: None,
        }
    }

    #[test]
    fn scoped_utilization_lists_fill_and_exclude_the_other_kind() {
        // `.64.3.2`: the Site scope covers Sites (and APs, which are `Site`); the
        // Circuit scope covers circuits. Each list fills to `min(limit, count)` and
        // EXCLUDES the other kind.
        let mut entities: Vec<ExecutiveEntitySnapshot> = Vec::new();
        for i in 0..12 {
            entities.push(util_entity(
                &format!("Site {i:02}"),
                ExecutiveEntityKind::Site,
                &[(14, 10.0 + i as f32)],
                &[(14, 5.0 + i as f32)],
            ));
        }
        for i in 0..12 {
            entities.push(util_entity(
                &format!("Circuit {i:02}"),
                ExecutiveEntityKind::Circuit,
                &[(14, 100.0 + i as f32)],
                &[(14, 50.0 + i as f32)],
            ));
        }

        let sites = top_metric_rows(&entities, ExecutiveMetric::Download, MetricScope::Site, 10);
        let circuits =
            top_metric_rows(&entities, ExecutiveMetric::Download, MetricScope::Circuit, 10);

        // Each fills to the limit (>= limit entities of the kind).
        assert_eq!(sites.len(), 10, "site list must fill to the limit");
        assert_eq!(circuits.len(), 10, "circuit list must fill to the limit");
        // Kinds are excluded (no cross-contamination).
        assert!(sites.iter().all(|row| row.entity_kind == ExecutiveEntityKind::Site));
        assert!(circuits.iter().all(|row| row.entity_kind == ExecutiveEntityKind::Circuit));
        assert!(!sites.iter().any(|row| row.label.starts_with("Circuit")));
        assert!(!circuits.iter().any(|row| row.label.starts_with("Site")));
    }

    #[test]
    fn scoped_utilization_lists_fill_to_min_limit_count() {
        // Fewer than `limit` sites => the site list is exactly the site count.
        let entities = vec![
            util_entity("Site A", ExecutiveEntityKind::Site, &[(14, 10.0)], &[]),
            util_entity("Site B", ExecutiveEntityKind::Site, &[(14, 20.0)], &[]),
            util_entity("Circuit X", ExecutiveEntityKind::Circuit, &[(14, 30.0)], &[]),
        ];
        let sites = top_metric_rows(&entities, ExecutiveMetric::Download, MetricScope::Site, 10);
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[0].label, "Site B"); // highest download ranks first
        assert_eq!(sites[1].label, "Site A");
    }

    fn site_heatmap_row(site_name: &str) -> SiteHeatmapData {
        SiteHeatmapData {
            site_name: site_name.to_string(),
            node_type: None,
            depth: 0,
            blocks: TemporalHeatmap::new().blocks(),
            qoq_blocks: None,
        }
    }

    fn circuit_heatmap_row(circuit_id: &str, circuit_name: &str) -> CircuitHeatmapData {
        CircuitHeatmapData {
            circuit_hash: 0,
            circuit_id: circuit_id.to_string(),
            circuit_name: circuit_name.to_string(),
            blocks: TemporalHeatmap::new().blocks(),
            qoq_blocks: None,
        }
    }

    fn asn_heatmap_row(asn: u32, asn_name: Option<&str>) -> AsnHeatmapData {
        AsnHeatmapData {
            asn,
            asn_name: asn_name.map(str::to_string),
            blocks: TemporalHeatmap::new().blocks(),
        }
    }

    fn tree_locator(node_id: Option<&str>, node_path: Option<&[&str]>) -> ExecutiveTreeLocator {
        ExecutiveTreeLocator {
            node_id: node_id.map(str::to_string),
            node_path: node_path.map(|path| path.iter().map(|name| name.to_string()).collect()),
            parent_index: None,
        }
    }

    #[test]
    fn entity_snapshots_keeps_the_client_row_key_contract() {
        // `row_key` is the identity the executive heatmap panels de-duplicate on
        // (`js_build/src/dashlets/executive_heatmap_panels.js`), so consuming the
        // heatmap rows must not change one key or label: node-id site, path-only
        // site, locator-with-neither-field site, unmatched site, circuit (named
        // and blank-named), and ASN with and without a name.
        let mut locators = FxHashMap::default();
        locators.insert(
            "Alpha".to_string(),
            tree_locator(Some("node-1"), Some(&["Root", "Alpha"])),
        );
        locators.insert("Beta".to_string(), tree_locator(None, Some(&["Root", "Beta"])));
        // A locator that carries neither a node id nor a path (REV-T4): the row
        // must fall back to its name key and still keep the tree.
        locators.insert("Delta".to_string(), tree_locator(None, None));

        let rows = entity_snapshots_with_locators(
            vec![
                circuit_heatmap_row("c-1", "Circuit One"),
                circuit_heatmap_row("c-2", ""),
            ],
            vec![
                site_heatmap_row("Alpha"),
                site_heatmap_row("Beta"),
                site_heatmap_row("Gamma"),
                site_heatmap_row("Delta"),
            ],
            vec![asn_heatmap_row(64500, Some("Example")), asn_heatmap_row(64501, None)],
            locators,
        );

        let keys: Vec<&str> = rows.iter().map(|row| row.row_key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "site:node-1",
                "site:[\"Root\", \"Beta\"]",
                "site:Gamma",
                "site:Delta",
                "circuit:c-1",
                "circuit:c-2",
                "asn:64500",
                "asn:64501",
            ]
        );
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Alpha",
                "Beta",
                "Gamma",
                "Delta",
                "Circuit One",
                "c-2",
                "Example",
                "ASN 64501",
            ]
        );
        assert_eq!(
            rows[0].tree.as_ref().and_then(|tree| tree.node_id.as_deref()),
            Some("node-1")
        );
        // REV-T4: neither `node_id` nor `node_path` set => name key, tree kept.
        assert!(
            rows[3].tree.is_some(),
            "a locator with no id and no path must still travel into the row"
        );
        assert_eq!(rows[4].circuit_id.as_deref(), Some("c-1"));
        assert_eq!(rows[6].asn, Some(64500));
    }

    #[test]
    fn entity_snapshots_moves_names_and_locators_instead_of_cloning() {
        // W4's PRIMARY effect, proven without an allocator: a moved `String`
        // keeps its heap buffer, so buffer identity distinguishes a move from a
        // clone. Every field below is charged to the row the tracker handed over.
        let mut locators = FxHashMap::default();
        let site_locator = tree_locator(Some("node-1"), Some(&["Root", "Alpha"]));
        let node_id_buffer = site_locator.node_id.as_ref().expect("node id").as_ptr();
        let node_path_buffer = site_locator.node_path.as_ref().expect("path").as_ptr();
        locators.insert("Alpha".to_string(), site_locator);

        let site = site_heatmap_row("Alpha");
        let site_label_buffer = site.site_name.as_ptr();
        let circuit = circuit_heatmap_row("c-1", "Circuit One");
        let circuit_label_buffer = circuit.circuit_name.as_ptr();
        let circuit_id_buffer = circuit.circuit_id.as_ptr();
        let asn = asn_heatmap_row(64500, Some("Example"));
        let asn_label_buffer = asn.asn_name.as_ref().expect("asn name").as_ptr();

        let rows = entity_snapshots_with_locators(vec![circuit], vec![site], vec![asn], locators);

        assert_eq!(
            rows[0].label.as_ptr(),
            site_label_buffer,
            "the site name must be moved into the row, not cloned"
        );
        let tree = rows[0].tree.as_ref().expect("site tree");
        assert_eq!(
            tree.node_id.as_ref().expect("node id").as_ptr(),
            node_id_buffer,
            "the locator's node id must be moved out of the locator map"
        );
        assert_eq!(
            tree.node_path.as_ref().expect("node path").as_ptr(),
            node_path_buffer,
            "the locator's node path must be moved out of the locator map"
        );
        assert_eq!(
            rows[1].label.as_ptr(),
            circuit_label_buffer,
            "the circuit name must be moved into the row, not cloned"
        );
        assert_eq!(
            rows[1].circuit_id.as_deref().map(str::as_ptr),
            Some(circuit_id_buffer),
            "the circuit id must be moved into the row, not cloned"
        );
        assert_eq!(
            rows[2].label.as_ptr(),
            asn_label_buffer,
            "the ASN name must be moved into the row, not cloned"
        );
    }

    #[test]
    fn entity_snapshots_bills_one_key_buffer_per_entity() {
        // W4 PIN: the once-per-second snapshot pass allocates one buffer per
        // entity (its `row_key`), plus the one declared blank-name copy and the
        // one `format!`-built path key; labels, ids, locators and heatmaps are
        // moved out of the heatmap rows the tracker already built.
        //
        // The world below is 13 entities and exercises every allocating branch:
        // 3 node-id sites + 1 path-only site (its key is still a `format!`) + 1
        // unmatched site; 4 named circuits + 1 blank-named circuit (the declared
        // single copy); 2 named ASNs + 1 unnamed ASN (its label is one buffer).
        // A reintroduced `label.clone()`, `locators.get(..).cloned()`, or a second
        // per-entity `format!` bills at least one more per entity and turns this
        // red.
        let world = || {
            let mut locators = FxHashMap::default();
            for index in 0..3 {
                locators.insert(
                    format!("Site {index}"),
                    tree_locator(Some(&format!("node-{index}")), None),
                );
            }
            locators.insert("Path Only".to_string(), tree_locator(None, Some(&["Root"])));
            let mut circuits = (0..4)
                .map(|index| circuit_heatmap_row(&format!("c-{index}"), &format!("Circuit {index}")))
                .collect::<Vec<_>>();
            circuits.push(circuit_heatmap_row("c-blank", ""));
            let mut sites = (0..4).map(|index| site_heatmap_row(&format!("Site {index}"))).collect::<Vec<_>>();
            sites.push(site_heatmap_row("Path Only"));
            sites.push(site_heatmap_row("Unmatched"));
            let mut asns = (0..2u32)
                .map(|index| asn_heatmap_row(64_500 + index, Some("Example")))
                .collect::<Vec<_>>();
            asns.push(asn_heatmap_row(64_502, None));
            (circuits, sites, asns, locators)
        };
        // Worlds are built OUTSIDE every counted window; `pop` allocates nothing.
        let mut worlds = (0..3).map(|_| world()).collect::<Vec<_>>();
        let mut measured = Vec::new();
        while let Some((circuits, sites, asns, locators)) = worlds.pop() {
            let entities = circuits.len() + sites.len() + asns.len();
            measured.push((
                entities,
                super::allocation_counter::count_allocations(|| {
                    std::hint::black_box(entity_snapshots_with_locators(
                        circuits, sites, asns, locators,
                    ));
                }),
            ));
        }
        for (entities, allocations) in measured {
            // Term-by-term budget: one key buffer per entity, plus the rows `Vec`,
            // the sites' key pass `Vec`, the blank-named circuit's second buffer,
            // and one extra buffer for each `format!` still in the pass (the
            // path-only site's key, the unnamed ASN's label).
            let expected = entities
                + 1 // rows `Vec::with_capacity`
                + 1 // the sites' key pass `Vec::with_capacity`
                + 1 // blank-named circuit: `label` copies `circuit_id`
                + 1 // path-only site key: `format!("site:{:?}", path)`
                + 1; // unnamed ASN label: its own buffer beside the row key
            assert_eq!(
                allocations, expected,
                "the snapshot pass must bill one key buffer per entity plus its declared \
                 extras: {entities} entities => {expected} allocations, measured {allocations}. \
                 A per-entity name/tree clone or a second formatted key is back."
            );
        }
    }

    #[test]
    fn duplicate_site_names_keep_the_shared_key_and_move_the_locator_once() {
        // Duplicate node names are a SUPPORTED runtime case (`network.json` counts
        // names unique in both trees; `node_locator_map` is first-wins) and the
        // dashboard collapses rows that share a key. Both rows must therefore keep
        // the pre-W4 shared key form — REV-C1 — while the locator itself still
        // moves out to the first row only.
        let mut locators = FxHashMap::default();
        locators.insert("Alpha".to_string(), tree_locator(Some("node-1"), None));

        let rows = entity_snapshots_with_locators(
            Vec::new(),
            vec![site_heatmap_row("Alpha"), site_heatmap_row("Alpha")],
            Vec::new(),
            locators,
        );

        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].row_key, "site:node-1",
            "the first duplicate keeps the locator's key"
        );
        assert_eq!(
            rows[1].row_key, "site:node-1",
            "the second duplicate shares the SAME key, so the dashboard collapses \
             the pair instead of showing the site twice"
        );
        assert!(
            rows[0].tree.is_some(),
            "the locator moves out to the first row"
        );
        assert!(
            rows[1].tree.is_none(),
            "and is not cloned for the duplicate"
        );
        assert_eq!(rows[0].label, "Alpha");
        assert_eq!(rows[1].label, "Alpha");
    }

    /// Smallest possible network tree for the entry-point wiring test: `Root`
    /// plus one named, id-bearing site.
    fn network_json_node(
        name: &str,
        id: Option<&str>,
        parents: Vec<usize>,
        immediate_parent: Option<usize>,
    ) -> lqos_config::NetworkJsonNode {
        lqos_config::NetworkJsonNode {
            name: name.to_string(),
            id: id.map(str::to_string),
            max_throughput: (0.0, 0.0),
            current_throughput: lqos_utils::units::DownUpOrder::zeroed(),
            current_packets: lqos_utils::units::DownUpOrder::zeroed(),
            current_tcp_packets: lqos_utils::units::DownUpOrder::zeroed(),
            current_udp_packets: lqos_utils::units::DownUpOrder::zeroed(),
            current_icmp_packets: lqos_utils::units::DownUpOrder::zeroed(),
            current_tcp_retransmits: lqos_utils::units::DownUpOrder::zeroed(),
            current_tcp_retransmit_packets: lqos_utils::units::DownUpOrder::zeroed(),
            current_rtt_flows: lqos_utils::units::DownUpOrder::zeroed(),
            current_marks: lqos_utils::units::DownUpOrder::zeroed(),
            current_drops: lqos_utils::units::DownUpOrder::zeroed(),
            rtt_buffer: Default::default(),
            parents,
            immediate_parent,
            node_type: None,
            latitude: None,
            longitude: None,
            active_attachment_name: None,
            heatmap: None,
        }
    }

    /// Installs a published network tree for the duration of a test and restores
    /// the previous nodes on drop. Holds the crate-wide `network.json` lock for the
    /// same lifetime: `bridge_analytics` publishes trees in its own tests, and its
    /// module lock does not exclude this one, so the tree is pinned here.
    struct InstalledTree {
        _network_guard: std::sync::MutexGuard<'static, ()>,
        previous: Vec<lqos_config::NetworkJsonNode>,
    }

    impl Drop for InstalledTree {
        fn drop(&mut self) {
            let previous = std::mem::take(&mut self.previous);
            lqos_network_devices::with_network_json_write(|net_json| {
                net_json.nodes = previous;
            });
        }
    }

    fn install_tree(nodes: Vec<lqos_config::NetworkJsonNode>) -> InstalledTree {
        let network_guard = crate::test_support::network_json_test_lock()
            .lock()
            .expect("network.json test lock should not be poisoned");
        let previous = lqos_network_devices::with_network_json_write(|net_json| {
            std::mem::replace(&mut net_json.nodes, nodes)
        });
        InstalledTree {
            _network_guard: network_guard,
            previous,
        }
    }

    #[test]
    fn entity_snapshots_locates_site_rows_at_the_production_entry_point() {
        // REV-C2: every other test calls `entity_snapshots_with_locators`, so
        // replacing `node_locator_map()` with an empty map inside the production
        // `entity_snapshots` would keep the suite green while every site row lost
        // its tree and its node-id key. This goes through the real entry point
        // with a real tree, so that regression turns red.
        let _runtime = crate::test_support::runtime_config_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _tree = install_tree(vec![
            network_json_node("Root", Some("root"), Vec::new(), None),
            network_json_node("Entry Site", Some("node-entry"), vec![0], Some(0)),
        ]);

        let rows = entity_snapshots(Vec::new(), vec![site_heatmap_row("Entry Site")], Vec::new());

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].row_key, "site:node-entry",
            "the entry point must key the row off the tree node id, not the site name"
        );
        let tree = rows[0]
            .tree
            .as_ref()
            .expect("the production entry point must locate the site row");
        assert_eq!(tree.node_id.as_deref(), Some("node-entry"));
        assert_eq!(tree.parent_index, Some(1));
        assert_eq!(
            tree.node_path.as_deref(),
            Some(&["Root".to_string(), "Entry Site".to_string()][..]),
            "the tree path comes from the published network tree"
        );
    }
}
