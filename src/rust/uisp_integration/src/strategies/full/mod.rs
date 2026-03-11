#![allow(dead_code)]
mod ap_promotion;
pub(crate) mod bandwidth_overrides;
mod client_site_promotion;
pub mod mikrotik;
mod network_json;
pub(crate) mod parse;
mod root_site;
pub(crate) mod routes_override;
pub mod shaped_devices_writer;
mod squash_single_entry_aps;
mod tree_walk;
pub(crate) mod uisp_fetch;
mod utils;
mod zero_capacity_sites;

use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::strategies::full::ap_promotion::promote_access_points;
use crate::strategies::full::bandwidth_overrides::{
    apply_bandwidth_overrides, get_site_bandwidth_overrides,
};
use crate::strategies::full::client_site_promotion::promote_clients_with_children;
use crate::strategies::full::network_json::write_network_file;
use crate::strategies::full::parse::parse_uisp_datasets;
use crate::strategies::full::root_site::{find_root_site, set_root_site};
use crate::strategies::full::routes_override::get_route_overrides;
use crate::strategies::full::shaped_devices_writer::write_shaped_devices;
use crate::strategies::full::squash_single_entry_aps::{squash_single_aps, squash_squashed_sites};
use crate::strategies::full::tree_walk::walk_tree_for_routing;
use crate::strategies::full::uisp_fetch::load_uisp_data;
use crate::strategies::full::utils::{print_sites, warn_of_no_parents_and_promote};
use crate::strategies::full::zero_capacity_sites::correct_zero_capacity_sites;
use crate::uisp_types::{UispSite, UispSiteType};
use crate::{blackboard, blackboard_blob};
use lqos_bus::BlackboardSystem;
use lqos_config::Config;
use std::sync::Arc;
use tracing::warn;

/// Attempt to construct a full hierarchy topology for the UISP network.
/// This function will load the UISP data, parse it into a more usable format,
/// and then attempt to build a full network topology.
///
/// # Arguments
/// * `config` - The configuration
/// * `ip_ranges` - The IP ranges to use for the network
///
/// # Returns
/// * An `Ok` if the operation was successful
/// * An `Err` if the operation failed
pub async fn build_full_network(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    // Load any bandwidth overrides
    let bandwidth_overrides = get_site_bandwidth_overrides(&config)?;
    blackboard(
        BlackboardSystem::System,
        "UISP-Bandwidth",
        &serde_json::to_string(&bandwidth_overrides).unwrap_or_default(),
    )
    .await;

    // Load any routing overrides
    let routing_overrides = get_route_overrides(&config)?;
    blackboard(
        BlackboardSystem::System,
        "UISP-Routes",
        &serde_json::to_string(&routing_overrides).unwrap_or_default(),
    )
    .await;

    // Obtain the UISP data and transform it into easier to work with types
    let (sites_raw, devices_raw, data_links_raw, devices_as_json) =
        load_uisp_data(config.clone()).await?;

    if let Err(e) = blackboard_blob("uisp_sites", &sites_raw).await {
        warn!("Unable to write sites to blackboard: {e:?}");
    }
    if let Err(e) = blackboard_blob("uisp_devices", &devices_as_json).await {
        warn!("Unable to write devices to blackboard: {e:?}");
    }
    if let Err(e) = blackboard_blob("uisp_data_links", &data_links_raw).await {
        warn!("Unable to write data links to blackboard: {e:?}");
    }

    // If Mikrotik is enabled, we need to fetch the Mikrotik data
    let ipv4_to_v6 = mikrotik::mikrotik_data(&config)
        .await
        .unwrap_or_else(|_| Vec::new());
    //println!("{:?}", ipv4_to_v6);

    // Parse the UISP data into a more usable format
    let (mut sites, data_links, devices) = parse_uisp_datasets(
        &sites_raw,
        &data_links_raw,
        &devices_raw,
        &config,
        &ip_ranges,
        ipv4_to_v6,
    );

    // Check root sites
    let root_site = find_root_site(&config, &mut sites, &data_links)?;
    blackboard(BlackboardSystem::System, "UISP-Root", &root_site).await;

    // Set the site root
    set_root_site(&mut sites, &root_site)?;

    // Create a new "_Infrastructure" node for the parent, since we can't link to the top
    // level very easily
    if let Some(root_idx) = sites.iter().position(|s| s.name == root_site) {
        sites.push(UispSite {
            id: format!("{}_Infrastructure", sites[root_idx].name.clone()),
            name: format!("{}_Infrastructure", sites[root_idx].name.clone()),
            site_type: UispSiteType::Site,
            uisp_parent_id: None,
            parent_indices: Default::default(),
            max_down_mbps: sites[root_idx].max_down_mbps,
            max_up_mbps: sites[root_idx].max_down_mbps,
            base_down_mbps: 0.0,
            base_up_mbps: 0.0,
            burst_down_mbps: 0.0,
            burst_up_mbps: 0.0,
            suspended: false,
            device_indices: vec![],
            route_weights: vec![],
            selected_parent: Some(root_idx),
        });
    }

    // Search for devices that provide links elsewhere
    promote_access_points(
        &mut sites,
        &devices_raw,
        &data_links_raw,
        &sites_raw,
        &devices,
        &config,
    )
    .await;

    // Sites that are clients but have children should be promoted
    promote_clients_with_children(&mut sites)?;

    // Do Link Squashing
    squash_single_aps(&mut sites)?;

    // Apply bandwidth overrides
    apply_bandwidth_overrides(&mut sites, &bandwidth_overrides);

    // Correct any sites with zero capacity
    correct_zero_capacity_sites(&mut sites, &config);

    // Squash any sites that are in the squash list
    squash_squashed_sites(&mut sites, config.clone(), &root_site)?;

    // Build Path Weights
    walk_tree_for_routing(config.clone(), &mut sites, &root_site, &routing_overrides)?;

    // Print Sites
    if let Some(root_idx) = sites.iter().position(|s| s.name == root_site) {
        // Issue No Parent Warnings
        warn_of_no_parents_and_promote(&mut sites, &devices_raw, root_idx, &config);

        print_sites(&sites, root_idx);

        // Output a network.json
        write_network_file(&config, &sites, root_idx)?;

        // Write ShapedDevices.csv
        write_shaped_devices(&config, &sites, root_idx, &devices)?;
    }

    Ok(())
}
