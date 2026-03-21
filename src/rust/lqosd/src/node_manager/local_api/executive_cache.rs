use crate::node_manager::local_api::executive::{
    ExecutiveDashboardMetricRow, ExecutiveDashboardSummary, ExecutiveEntityKind,
    ExecutiveHeatmapPageRow, ExecutiveLeaderboardKind, ExecutiveLeaderboardRow, ExecutiveMetric,
    ExecutiveOversubscribedSiteRow, ExecutiveRttBlocks, ExecutiveScalarBlocks,
    ExecutiveSplitBlocks, ExecutiveTopAsnRow, ExecutiveTreeLocator,
};
use crate::shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICES};
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
                top_retransmit: Vec::new(),
                top_rtt: Vec::new(),
                top_qoo: Vec::new(),
                oversubscribed_sites: Vec::new(),
                top_asns: Vec::new(),
            },
            entities: Vec::new(),
            leaderboard_rows: FxHashMap::default(),
        }
    }
}

impl ExecutiveCacheSnapshot {
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
                    ExecutiveMetric::Qoo => entity.qoq_blocks.as_ref().map(qoq_to_split_blocks),
                    ExecutiveMetric::Download | ExecutiveMetric::Upload => Some(ExecutiveSplitBlocks {
                        download: array_to_vec(&entity.heatmap.download),
                        upload: array_to_vec(&entity.heatmap.upload),
                    }),
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
        ExecutiveMetric::Qoo => None,
    }?;
    Some(ExecutiveScalarBlocks { values })
}

fn qoq_to_split_blocks(blocks: &QoqHeatmapBlocks) -> ExecutiveSplitBlocks {
    ExecutiveSplitBlocks {
        download: array_to_vec(&blocks.download_total),
        upload: array_to_vec(&blocks.upload_total),
    }
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

fn latest_qoo(blocks: Option<&QoqHeatmapBlocks>) -> Option<f32> {
    let blocks = blocks?;
    let mut present = Vec::new();
    if let Some(download) = latest_value(&blocks.download_total) {
        present.push(download);
    }
    if let Some(upload) = latest_value(&blocks.upload_total) {
        present.push(upload);
    }
    if present.is_empty() {
        None
    } else {
        Some(present.iter().sum::<f32>() / present.len() as f32)
    }
}

fn qoo_sample_count(blocks: Option<&QoqHeatmapBlocks>) -> usize {
    let Some(blocks) = blocks else {
        return 0;
    };
    non_null_count(&blocks.download_total).max(non_null_count(&blocks.upload_total))
}

fn sort_score_for_metric(entity: &ExecutiveEntitySnapshot, metric: &ExecutiveMetric) -> f32 {
    match metric {
        ExecutiveMetric::Download => latest_value(&entity.heatmap.download).unwrap_or(f32::NEG_INFINITY),
        ExecutiveMetric::Upload => latest_value(&entity.heatmap.upload).unwrap_or(f32::NEG_INFINITY),
        ExecutiveMetric::Retransmit => {
            let latest = latest_value(&entity.heatmap.retransmit).unwrap_or(f32::NEG_INFINITY);
            let count = non_null_count(&entity.heatmap.retransmit);
            let penalty = if count < MIN_HEATMAP_SAMPLES { 1000.0 } else { 0.0 };
            latest + COUNT_WEIGHT * (count as f32 / 15.0) - penalty
        }
        ExecutiveMetric::Rtt => {
            let latest = latest_value(&entity.heatmap.rtt).unwrap_or(f32::NEG_INFINITY);
            let count = non_null_count(&entity.heatmap.rtt);
            let penalty = if count < MIN_HEATMAP_SAMPLES { 1000.0 } else { 0.0 };
            latest + COUNT_WEIGHT * (count as f32 / 15.0) - penalty
        }
        ExecutiveMetric::Qoo => {
            let latest = latest_qoo(entity.qoq_blocks.as_ref()).unwrap_or(f32::INFINITY);
            let count = qoo_sample_count(entity.qoq_blocks.as_ref());
            let penalty = if count < MIN_HEATMAP_SAMPLES { 1000.0 } else { 0.0 };
            -latest + COUNT_WEIGHT * (count as f32 / 15.0) - penalty
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
            ExecutiveMetric::Qoo => entity.qoq_blocks.as_ref().map(qoq_to_split_blocks),
            ExecutiveMetric::Rtt => None,
        },
        rtt_blocks: match metric {
            ExecutiveMetric::Rtt => Some(heatmap_to_rtt_blocks(&entity.heatmap)),
            _ => None,
        },
    }
}

fn top_metric_rows(
    entities: &[ExecutiveEntitySnapshot],
    metric: ExecutiveMetric,
    limit: usize,
) -> Vec<ExecutiveDashboardMetricRow> {
    let mut ranked = entities
        .iter()
        .filter(|entity| matches!(entity.entity_kind, ExecutiveEntityKind::Site | ExecutiveEntityKind::Circuit))
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
    rows.sort_by(|left, right| right.total_bytes_15m.cmp(&left.total_bytes_15m));
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
            ExecutiveLeaderboardRow::WorstSiteByRtt { median_rtt_ms: left, .. },
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

fn node_path_names(
    idx: usize,
    nodes: &[lqos_config::NetworkJsonNode],
) -> Option<Vec<String>> {
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
    let reader = NETWORK_JSON.read();
    let nodes = reader.get_nodes_when_ready();
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
}

fn entity_snapshots(
    circuits: Vec<CircuitHeatmapData>,
    sites: Vec<SiteHeatmapData>,
    asns: Vec<AsnHeatmapData>,
) -> Vec<ExecutiveEntitySnapshot> {
    let locators = node_locator_map();
    let mut rows = Vec::new();
    for site in sites {
        let row_key = locators
            .get(&site.site_name)
            .and_then(|locator| {
                locator
                    .node_id
                    .as_ref()
                    .map(|node_id| format!("site:{node_id}"))
                    .or_else(|| {
                        locator
                            .node_path
                            .as_ref()
                            .map(|path| format!("site:{:?}", path))
                    })
            })
            .unwrap_or_else(|| format!("site:{}", site.site_name));
        rows.push(ExecutiveEntitySnapshot {
            row_key,
            entity_kind: ExecutiveEntityKind::Site,
            label: site.site_name.clone(),
            circuit_id: None,
            asn: None,
            tree: locators.get(&site.site_name).cloned(),
            heatmap: site.blocks,
            qoq_blocks: site.qoq_blocks,
        });
    }
    for circuit in circuits {
        let label = if circuit.circuit_name.trim().is_empty() {
            circuit.circuit_id.clone()
        } else {
            circuit.circuit_name.clone()
        };
        rows.push(ExecutiveEntitySnapshot {
            row_key: format!("circuit:{}", circuit.circuit_id),
            entity_kind: ExecutiveEntityKind::Circuit,
            label,
            circuit_id: Some(circuit.circuit_id),
            asn: None,
            tree: None,
            heatmap: circuit.blocks,
            qoq_blocks: circuit.qoq_blocks,
        });
    }
    for asn in asns {
        rows.push(ExecutiveEntitySnapshot {
            row_key: format!("asn:{}", asn.asn),
            entity_kind: ExecutiveEntityKind::Asn,
            label: asn
                .asn_name
                .clone()
                .unwrap_or_else(|| format!("ASN {}", asn.asn)),
            circuit_id: None,
            asn: Some(asn.asn),
            tree: None,
            heatmap: asn.blocks,
            qoq_blocks: None,
        });
    }
    rows
}

fn build_oversubscribed_sites(
    entities: &[ExecutiveEntitySnapshot],
) -> Vec<ExecutiveOversubscribedSiteRow> {
    let devices = SHAPED_DEVICES.load();
    let mut circuit_map: std::collections::HashMap<String, (String, f32, f32)> =
        std::collections::HashMap::new();
    for device in &devices.devices {
        let entry = circuit_map
            .entry(device.circuit_id.clone())
            .or_insert_with(|| (device.parent_node.clone(), 0.0_f32, 0.0_f32));
        entry.1 = entry.1.max(device.download_max_mbps);
        entry.2 = entry.2.max(device.upload_max_mbps);
    }

    let reader = NETWORK_JSON.read();
    let nodes = reader.get_nodes_when_ready();
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

    let entity_map = entities
        .iter()
        .filter(|entity| entity.entity_kind == ExecutiveEntityKind::Site)
        .map(|entity| (entity.label.clone(), entity))
        .collect::<FxHashMap<_, _>>();

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
        for (parent_name, circuit_down, circuit_up) in circuit_map.values() {
            if descendant_names.contains(parent_name) {
                sub_down += *circuit_down;
                sub_up += *circuit_up;
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
    let global_qoq = THROUGHPUT_TRACKER.global_qoq_heatmap.lock().blocks();
    let entities = entity_snapshots(circuit_heatmap_rows(), site_heatmap_rows(), asn_heatmap_rows());
    let oversubscribed_sites = build_oversubscribed_sites(&entities);
    let top_download = top_metric_rows(&entities, ExecutiveMetric::Download, EXECUTIVE_TOP_LIMIT);
    let top_upload = top_metric_rows(&entities, ExecutiveMetric::Upload, EXECUTIVE_TOP_LIMIT);
    let top_retransmit =
        top_metric_rows(&entities, ExecutiveMetric::Retransmit, EXECUTIVE_TOP_LIMIT);
    let top_rtt = top_metric_rows(&entities, ExecutiveMetric::Rtt, EXECUTIVE_TOP_LIMIT);
    let top_qoo = top_metric_rows(&entities, ExecutiveMetric::Qoo, EXECUTIVE_TOP_LIMIT);
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
            top_retransmit,
            top_rtt,
            top_qoo,
            oversubscribed_sites,
            top_asns,
        },
        entities,
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
