mod ap_promotion;
mod client_site_promotion;
mod parse;
mod root_site;
mod squash_single_entry_aps;
mod tree_walk;
mod uisp_fetch;
mod utils;

use crate::errors::UispIntegrationError;
use crate::strategies::full::ap_promotion::promote_access_points;
use crate::strategies::full::client_site_promotion::promote_clients_with_children;
use crate::strategies::full::parse::parse_uisp_datasets;
use crate::strategies::full::root_site::{find_root_site, set_root_site};
use crate::strategies::full::squash_single_entry_aps::squash_single_aps;
use crate::strategies::full::tree_walk::walk_tree_for_routing;
use crate::strategies::full::uisp_fetch::load_uisp_data;
use crate::strategies::full::utils::{print_sites, warn_of_no_parents};
use crate::uisp_types::UispSite;
use lqos_config::Config;

/// Attempt to construct a full hierarchy topology for the UISP network.
pub async fn build_full_network(config: Config) -> Result<(), UispIntegrationError> {
    // Obtain the UISP data and transform it into easier to work with types
    let (sites_raw, devices_raw, data_links_raw) = load_uisp_data(config.clone()).await?;
    let (mut sites, data_links, devices) =
        parse_uisp_datasets(&sites_raw, &data_links_raw, &devices_raw, &config);

    // Check root sites
    let root_site = find_root_site(&config, &mut sites, &data_links)?;

    // Set the site root
    set_root_site(&mut sites, &root_site)?;

    // Search for devices that provide links elsewhere
    promote_access_points(
        &mut sites,
        &devices_raw,
        &data_links_raw,
        &sites_raw,
        &devices,
        &config,
    );

    // Sites that are clients but have children should be promoted
    promote_clients_with_children(&mut sites)?;

    // Do Link Squashing
    squash_single_aps(&mut sites)?;

    // Build Path Weights
    walk_tree_for_routing(&mut sites, &root_site)?;

    // Issue No Parent Warnings
    warn_of_no_parents(&sites, &devices_raw);

    // Print Sites
    if let Some(root_idx) = sites.iter().position(|s| s.name == root_site) {
        print_sites(&sites, root_idx);
    }

    Ok(())
}

fn walk_node(
    idx: usize,
    weight: u32,
    sites: &mut Vec<UispSite>,
    visited: &mut std::collections::HashSet<usize>,
) {
    if visited.contains(&idx) {
        return;
    }
    visited.insert(idx);
    for i in 0..sites.len() {
        if sites[i].parent_indices.contains(&idx) {
            sites[i].route_weights.push((idx, weight));
            walk_node(i, weight + 10, sites, visited);
        }
    }
}
