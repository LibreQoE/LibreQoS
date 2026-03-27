use crate::node_manager::local_api::executive_cache::fresh_executive_cache_snapshot;
use lqos_bus::ExecutiveSummaryHeader;
use lqos_utils::{HeatmapBlocks, qoq_heatmap::QoqHeatmapBlocks};
use serde::{Deserialize, Serialize};

const DEFAULT_EXECUTIVE_HEATMAP_PAGE_SIZE: usize = 50;
const DEFAULT_EXECUTIVE_LEADERBOARD_PAGE_SIZE: usize = 50;
const MAX_EXECUTIVE_PAGE_SIZE: usize = 250;

/// Entity kinds shown in executive dashboard and detail views.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ExecutiveEntityKind {
    Site,
    Circuit,
    Asn,
}

/// Executive metrics supported by summary and detail heatmap views.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ExecutiveMetric {
    Download,
    Upload,
    Retransmit,
    Rtt,
    Qoo,
}

/// Server-side sort modes for executive heatmap detail pages.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutiveHeatmapSort {
    LatestValue,
    Label,
    SampleCount,
}

/// Named executive leaderboard views.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ExecutiveLeaderboardKind {
    WorstSitesByRtt,
    OversubscribedSites,
    SitesDueUpgrade,
    CircuitsDueUpgrade,
    TopAsnsByTraffic,
}

/// Stable locator for linking executive rows back to the tree view.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct ExecutiveTreeLocator {
    pub node_id: Option<String>,
    pub node_path: Option<Vec<String>>,
    pub parent_index: Option<usize>,
}

/// One scalar historical series, usually used for a single metric.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct ExecutiveScalarBlocks {
    pub values: Vec<Option<f32>>,
}

/// One split directional historical series.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct ExecutiveSplitBlocks {
    pub download: Vec<Option<f32>>,
    pub upload: Vec<Option<f32>>,
}

/// RTT history including combined and directional percentile data.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct ExecutiveRttBlocks {
    pub rtt: Vec<Option<f32>>,
    pub dl_p50: Vec<Option<f32>>,
    pub dl_p90: Vec<Option<f32>>,
    pub ul_p50: Vec<Option<f32>>,
    pub ul_p90: Vec<Option<f32>>,
}

/// One server-ranked metric row for the executive dashboard.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExecutiveDashboardMetricRow {
    pub row_key: String,
    pub entity_kind: ExecutiveEntityKind,
    pub metric: ExecutiveMetric,
    pub label: String,
    pub circuit_id: Option<String>,
    pub asn: Option<u32>,
    pub tree: Option<ExecutiveTreeLocator>,
    pub scalar_blocks: Option<ExecutiveScalarBlocks>,
    pub split_blocks: Option<ExecutiveSplitBlocks>,
    pub rtt_blocks: Option<ExecutiveRttBlocks>,
}

/// Oversubscription leaderboard row for the executive dashboard.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExecutiveOversubscribedSiteRow {
    pub row_key: String,
    pub site_name: String,
    pub tree: Option<ExecutiveTreeLocator>,
    pub ratio_down: Option<f32>,
    pub ratio_up: Option<f32>,
    pub ratio_max: Option<f32>,
    pub cap_down: f32,
    pub cap_up: f32,
    pub sub_down: f32,
    pub sub_up: f32,
    pub avg_down_util: Option<f32>,
    pub avg_up_util: Option<f32>,
    pub median_rtt_ms: Option<f32>,
}

/// Top-ASN row for executive dashboard summary.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExecutiveTopAsnRow {
    pub row_key: String,
    pub asn: u32,
    pub asn_name: Option<String>,
    pub total_bytes_15m: u64,
    pub median_rtt_ms: Option<f32>,
    pub median_retransmit_pct: Option<f32>,
}

/// Compact executive summary payload for the dashboard.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExecutiveDashboardSummary {
    pub generated_at_unix_ms: u64,
    pub header: ExecutiveSummaryHeader,
    pub global: HeatmapBlocks,
    pub global_qoq: QoqHeatmapBlocks,
    pub top_download: Vec<ExecutiveDashboardMetricRow>,
    pub top_upload: Vec<ExecutiveDashboardMetricRow>,
    pub top_retransmit: Vec<ExecutiveDashboardMetricRow>,
    pub top_rtt: Vec<ExecutiveDashboardMetricRow>,
    pub top_qoo: Vec<ExecutiveDashboardMetricRow>,
    pub oversubscribed_sites: Vec<ExecutiveOversubscribedSiteRow>,
    pub top_asns: Vec<ExecutiveTopAsnRow>,
}

/// Query for a paged executive heatmap detail page.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExecutiveHeatmapPageQuery {
    pub metric: ExecutiveMetric,
    pub entity_kinds: Vec<ExecutiveEntityKind>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
    pub search: Option<String>,
    pub sort: Option<ExecutiveHeatmapSort>,
    pub descending: Option<bool>,
    pub client_request_id: Option<String>,
}

/// One paged executive heatmap detail row.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExecutiveHeatmapPageRow {
    pub row_key: String,
    pub entity_kind: ExecutiveEntityKind,
    pub label: String,
    pub circuit_id: Option<String>,
    pub asn: Option<u32>,
    pub tree: Option<ExecutiveTreeLocator>,
    pub scalar_blocks: Option<ExecutiveScalarBlocks>,
    pub split_blocks: Option<ExecutiveSplitBlocks>,
    pub rtt_blocks: Option<ExecutiveRttBlocks>,
}

/// One paged executive heatmap detail response.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExecutiveHeatmapPage {
    pub generated_at_unix_ms: u64,
    pub query: ExecutiveHeatmapPageQuery,
    pub total_rows: usize,
    pub rows: Vec<ExecutiveHeatmapPageRow>,
}

/// Query for a paged executive leaderboard detail view.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExecutiveLeaderboardPageQuery {
    pub kind: ExecutiveLeaderboardKind,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
    pub search: Option<String>,
    pub client_request_id: Option<String>,
}

/// One paged executive leaderboard response.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExecutiveLeaderboardPage {
    pub generated_at_unix_ms: u64,
    pub query: ExecutiveLeaderboardPageQuery,
    pub total_rows: usize,
    pub rows: Vec<ExecutiveLeaderboardRow>,
}

/// Rows for the standalone executive leaderboard pages.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum ExecutiveLeaderboardRow {
    WorstSiteByRtt {
        row_key: String,
        site_name: String,
        tree: Option<ExecutiveTreeLocator>,
        median_rtt_ms: f32,
        avg_down_util: Option<f32>,
        avg_up_util: Option<f32>,
    },
    OversubscribedSite {
        row_key: String,
        site_name: String,
        tree: Option<ExecutiveTreeLocator>,
        ratio_down: Option<f32>,
        ratio_up: Option<f32>,
        ratio_max: Option<f32>,
        cap_down: f32,
        cap_up: f32,
        sub_down: f32,
        sub_up: f32,
        avg_down_util: Option<f32>,
        avg_up_util: Option<f32>,
        median_rtt_ms: Option<f32>,
    },
    SiteDueUpgrade {
        row_key: String,
        site_name: String,
        tree: Option<ExecutiveTreeLocator>,
        avg_down_util: f32,
        avg_up_util: f32,
    },
    CircuitDueUpgrade {
        row_key: String,
        circuit_id: String,
        circuit_name: String,
        avg_down_util: f32,
        avg_up_util: f32,
    },
    TopAsnByTraffic {
        row_key: String,
        asn: u32,
        asn_name: Option<String>,
        total_bytes_15m: u64,
        median_rtt_ms: Option<f32>,
        median_retransmit_pct: Option<f32>,
    },
}

fn normalize_page_size(requested: Option<usize>, default_size: usize) -> usize {
    requested
        .unwrap_or(default_size)
        .clamp(1, MAX_EXECUTIVE_PAGE_SIZE)
}

fn normalized_search(search: &Option<String>) -> Option<String> {
    let value = search.as_deref().unwrap_or("").trim().to_lowercase();
    if value.is_empty() { None } else { Some(value) }
}

fn normalized_entity_kinds(mut kinds: Vec<ExecutiveEntityKind>) -> Vec<ExecutiveEntityKind> {
    if kinds.is_empty() {
        return vec![
            ExecutiveEntityKind::Site,
            ExecutiveEntityKind::Circuit,
            ExecutiveEntityKind::Asn,
        ];
    }
    kinds.sort_by_key(|kind| match kind {
        ExecutiveEntityKind::Site => 0_u8,
        ExecutiveEntityKind::Circuit => 1_u8,
        ExecutiveEntityKind::Asn => 2_u8,
    });
    kinds.dedup();
    kinds
}

fn latest_value(row: &ExecutiveHeatmapPageRow, metric: &ExecutiveMetric) -> Option<f32> {
    match metric {
        ExecutiveMetric::Download | ExecutiveMetric::Upload | ExecutiveMetric::Retransmit => row
            .scalar_blocks
            .as_ref()
            .and_then(|blocks| blocks.values.iter().rev().flatten().copied().next()),
        ExecutiveMetric::Qoo => row.split_blocks.as_ref().and_then(|blocks| {
            let values = [
                blocks.download.iter().rev().flatten().copied().next(),
                blocks.upload.iter().rev().flatten().copied().next(),
            ];
            let present: Vec<f32> = values.into_iter().flatten().collect();
            if present.is_empty() {
                None
            } else {
                Some(present.iter().sum::<f32>() / present.len() as f32)
            }
        }),
        ExecutiveMetric::Rtt => row
            .rtt_blocks
            .as_ref()
            .and_then(|blocks| blocks.rtt.iter().rev().flatten().copied().next()),
    }
}

fn sample_count(row: &ExecutiveHeatmapPageRow, metric: &ExecutiveMetric) -> usize {
    match metric {
        ExecutiveMetric::Download | ExecutiveMetric::Upload | ExecutiveMetric::Retransmit => row
            .scalar_blocks
            .as_ref()
            .map(|blocks| blocks.values.iter().filter(|value| value.is_some()).count())
            .unwrap_or(0),
        ExecutiveMetric::Qoo => row
            .split_blocks
            .as_ref()
            .map(|blocks| {
                blocks
                    .download
                    .iter()
                    .chain(blocks.upload.iter())
                    .filter(|value| value.is_some())
                    .count()
            })
            .unwrap_or(0),
        ExecutiveMetric::Rtt => row
            .rtt_blocks
            .as_ref()
            .map(|blocks| blocks.rtt.iter().filter(|value| value.is_some()).count())
            .unwrap_or(0),
    }
}

fn leaderboard_matches_search(row: &ExecutiveLeaderboardRow, search: &Option<String>) -> bool {
    let Some(search) = search else {
        return true;
    };
    match row {
        ExecutiveLeaderboardRow::WorstSiteByRtt { site_name, .. }
        | ExecutiveLeaderboardRow::OversubscribedSite { site_name, .. }
        | ExecutiveLeaderboardRow::SiteDueUpgrade { site_name, .. } => {
            site_name.to_lowercase().contains(search)
        }
        ExecutiveLeaderboardRow::CircuitDueUpgrade {
            circuit_id,
            circuit_name,
            ..
        } => {
            circuit_id.to_lowercase().contains(search)
                || circuit_name.to_lowercase().contains(search)
        }
        ExecutiveLeaderboardRow::TopAsnByTraffic { asn, asn_name, .. } => {
            asn.to_string().contains(search)
                || asn_name
                    .as_ref()
                    .map(|name| name.to_lowercase().contains(search))
                    .unwrap_or(false)
        }
    }
}

/// Returns the compact executive dashboard summary for published websocket updates.
pub fn executive_dashboard_summary() -> ExecutiveDashboardSummary {
    fresh_executive_cache_snapshot().dashboard.clone()
}

/// Returns one filtered, sorted executive heatmap detail page.
pub fn executive_heatmap_page(query: ExecutiveHeatmapPageQuery) -> ExecutiveHeatmapPage {
    let snapshot = fresh_executive_cache_snapshot();
    let page = query.page.unwrap_or(0);
    let page_size = normalize_page_size(query.page_size, DEFAULT_EXECUTIVE_HEATMAP_PAGE_SIZE);
    let search = normalized_search(&query.search);
    let sort = query
        .sort
        .clone()
        .unwrap_or(ExecutiveHeatmapSort::LatestValue);
    let descending = query.descending.unwrap_or(true);
    let entity_kinds = normalized_entity_kinds(query.entity_kinds.clone());

    let mut rows = snapshot
        .heatmap_rows_for_metric(&query.metric)
        .into_iter()
        .filter(|row| entity_kinds.contains(&row.entity_kind))
        .filter(|row| {
            let Some(search) = &search else {
                return true;
            };
            row.label.to_lowercase().contains(search)
                || row
                    .circuit_id
                    .as_ref()
                    .map(|id| id.to_lowercase().contains(search))
                    .unwrap_or(false)
                || row
                    .asn
                    .map(|asn| asn.to_string().contains(search))
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    rows.sort_by(|left, right| match sort {
        ExecutiveHeatmapSort::Label => left.label.cmp(&right.label),
        ExecutiveHeatmapSort::SampleCount => sample_count(left, &query.metric)
            .cmp(&sample_count(right, &query.metric))
            .then_with(|| left.label.cmp(&right.label)),
        ExecutiveHeatmapSort::LatestValue => {
            let left_latest = latest_value(left, &query.metric);
            let right_latest = latest_value(right, &query.metric);
            right_latest
                .partial_cmp(&left_latest)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.label.cmp(&left.label))
        }
    });

    if !descending {
        rows.reverse();
    }

    let total_rows = rows.len();
    let start = page.saturating_mul(page_size);
    let rows = if start >= total_rows {
        Vec::new()
    } else {
        rows.into_iter().skip(start).take(page_size).collect()
    };

    ExecutiveHeatmapPage {
        generated_at_unix_ms: snapshot.generated_at_unix_ms,
        query: ExecutiveHeatmapPageQuery {
            metric: query.metric,
            entity_kinds,
            page: Some(page),
            page_size: Some(page_size),
            search,
            sort: Some(sort),
            descending: Some(descending),
            client_request_id: query.client_request_id,
        },
        total_rows,
        rows,
    }
}

/// Returns one filtered executive leaderboard page.
pub fn executive_leaderboard_page(
    query: ExecutiveLeaderboardPageQuery,
) -> ExecutiveLeaderboardPage {
    let snapshot = fresh_executive_cache_snapshot();
    let page = query.page.unwrap_or(0);
    let page_size = normalize_page_size(query.page_size, DEFAULT_EXECUTIVE_LEADERBOARD_PAGE_SIZE);
    let search = normalized_search(&query.search);
    let mut rows = snapshot
        .leaderboard_rows
        .get(&query.kind)
        .cloned()
        .unwrap_or_default();
    rows.retain(|row| leaderboard_matches_search(row, &search));
    let total_rows = rows.len();
    let start = page.saturating_mul(page_size);
    let rows = if start >= total_rows {
        Vec::new()
    } else {
        rows.into_iter().skip(start).take(page_size).collect()
    };

    ExecutiveLeaderboardPage {
        generated_at_unix_ms: snapshot.generated_at_unix_ms,
        query: ExecutiveLeaderboardPageQuery {
            kind: query.kind,
            page: Some(page),
            page_size: Some(page_size),
            search,
            client_request_id: query.client_request_id,
        },
        total_rows,
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExecutiveEntityKind, ExecutiveHeatmapPageQuery, ExecutiveHeatmapSort,
        ExecutiveLeaderboardKind, ExecutiveLeaderboardPageQuery, ExecutiveMetric,
        executive_heatmap_page, executive_leaderboard_page,
    };

    #[test]
    fn executive_heatmap_page_preserves_client_request_id() {
        let page = executive_heatmap_page(ExecutiveHeatmapPageQuery {
            metric: ExecutiveMetric::Rtt,
            entity_kinds: vec![ExecutiveEntityKind::Site],
            page: Some(0),
            page_size: Some(10),
            search: Some("WestRedd".to_string()),
            sort: Some(ExecutiveHeatmapSort::LatestValue),
            descending: Some(true),
            client_request_id: Some("heatmap-req-1".to_string()),
        });

        assert_eq!(
            page.query.client_request_id.as_deref(),
            Some("heatmap-req-1")
        );
    }

    #[test]
    fn executive_leaderboard_page_preserves_client_request_id() {
        let page = executive_leaderboard_page(ExecutiveLeaderboardPageQuery {
            kind: ExecutiveLeaderboardKind::WorstSitesByRtt,
            page: Some(0),
            page_size: Some(10),
            search: Some("WestRedd".to_string()),
            client_request_id: Some("leaderboard-req-1".to_string()),
        });

        assert_eq!(
            page.query.client_request_id.as_deref(),
            Some("leaderboard-req-1")
        );
    }
}
