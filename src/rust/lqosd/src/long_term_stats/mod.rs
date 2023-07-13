//! Most of this functionality is now in the` lts_stats` crate.
use crate::shaped_devices_tracker::NETWORK_JSON;
use lqos_bus::BusResponse;
use lts_client::{
    collector::NetworkTreeEntry, submission_queue::get_current_stats
};

pub(crate) fn get_network_tree() -> Vec<(usize, NetworkTreeEntry)> {
    if let Ok(reader) = NETWORK_JSON.read() {
        let result = reader
            .nodes
            .iter()
            .enumerate()
            .map(|(idx, n)| (idx, n.into()))
            .collect::<Vec<(usize, NetworkTreeEntry)>>();
        //println!("{result:#?}");
        return result;
    }
    Vec::new()
}

pub fn get_stats_totals() -> BusResponse {
    let current = get_current_stats();
    if let Some(c) = current {
        if let Some(totals) = &c.totals {
            return BusResponse::LongTermTotals(totals.clone());
        }
    }
    BusResponse::Fail("No Data".to_string())
}

pub fn get_stats_host() -> BusResponse {
    let current = get_current_stats();
    if let Some(c) = current {
        if let Some(hosts) = c.hosts {
            return BusResponse::LongTermHosts(hosts);
        }
    }
    BusResponse::Fail("No Data".to_string())
}

pub fn get_stats_tree() -> BusResponse {
    let current = get_current_stats();
    if let Some(c) = current {
        if let Some(tree) = c.tree {
            return BusResponse::LongTermTree(tree);
        }
    }
    BusResponse::Fail("No Data".to_string())
}
