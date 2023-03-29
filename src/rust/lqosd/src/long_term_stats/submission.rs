use std::sync::RwLock;
use lqos_bus::{BusResponse, long_term_stats::StatsHost};
use once_cell::sync::Lazy;
use super::collator::StatsSubmission;

pub(crate) static CURRENT_STATS: Lazy<RwLock<Option<StatsSubmission>>> = Lazy::new(|| RwLock::new(None));

pub(crate) fn new_submission(data: StatsSubmission) {
    *CURRENT_STATS.write().unwrap() = Some(data);
}

pub fn get_stats_totals() -> BusResponse {
    let current = CURRENT_STATS.read().unwrap().clone();
    if let Some(c) = current {
        BusResponse::LongTermTotals(c.into())
    } else {
        BusResponse::Fail("No Data".to_string())
    }
}

pub fn get_stats_host() -> BusResponse {
    let current = CURRENT_STATS.read().unwrap();
    if let Some(c) = &*current {
        BusResponse::LongTermHosts(
            c.hosts.iter().cloned().map(std::convert::Into::<StatsHost>::into).collect()
        )
    } else {
        BusResponse::Fail("No Data".to_string())
    }
}

pub fn get_stats_tree() -> BusResponse {
    let current = CURRENT_STATS.read().unwrap();
    if let Some(c) = &*current {
        BusResponse::LongTermTree(
            c.tree.iter().map(|n| n.into()).collect()
        )
    } else {
        BusResponse::Fail("No Data".to_string())
    }
}