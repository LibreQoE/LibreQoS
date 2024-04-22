mod ap_promotion;
mod bandwidth_overrides;
mod client_site_promotion;
mod network_json;
mod parse;
mod root_site;
mod routes_override;
mod shaped_devices_writer;
mod squash_single_entry_aps;
mod tree_walk;
mod uisp_fetch;
mod utils;

use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::strategies::full::ap_promotion::promote_access_points;
use crate::strategies::full::bandwidth_overrides::get_site_bandwidth_overrides;
use crate::strategies::full::client_site_promotion::promote_clients_with_children;
use crate::strategies::full::network_json::write_network_file;
use crate::strategies::full::parse::parse_uisp_datasets;
use crate::strategies::full::root_site::{find_root_site, set_root_site};
use crate::strategies::full::routes_override::get_route_overrides;
use crate::strategies::full::shaped_devices_writer::write_shaped_devices;
use crate::strategies::full::squash_single_entry_aps::squash_single_aps;
use crate::strategies::full::tree_walk::walk_tree_for_routing;
use crate::strategies::full::uisp_fetch::load_uisp_data;
use crate::strategies::full::utils::{print_sites, warn_of_no_parents};
use crate::uisp_types::UispSite;
pub use bandwidth_overrides::BandwidthOverrides;
use lqos_config::Config;

/// Attempt to construct a full hierarchy topology for the UISP network.
pub async fn build_full_network(
    config: Config,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    // Load any bandwidth overrides
    let bandwidth_overrides = get_site_bandwidth_overrides(&config)?;

    // Load any routing overrrides
    let routing_overrides = get_route_overrides(&config)?;

    // Obtain the UISP data and transform it into easier to work with types
    let (sites_raw, devices_raw, data_links_raw) = load_uisp_data(config.clone()).await?;
    let (mut sites, data_links, devices) = parse_uisp_datasets(
        &sites_raw,
        &data_links_raw,
        &devices_raw,
        &config,
        &bandwidth_overrides,
        &ip_ranges,
    );

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
    walk_tree_for_routing(&mut sites, &root_site, &routing_overrides)?;

    // Issue No Parent Warnings
    warn_of_no_parents(&sites, &devices_raw);

    // Print Sites
    if let Some(root_idx) = sites.iter().position(|s| s.name == root_site) {
        print_sites(&sites, root_idx);

        // Output a network.json
        write_network_file(&config, &sites, root_idx)?;

        // Write ShapedDevices.csv
        write_shaped_devices(&config, &sites, root_idx, &devices)?;
    }

    Ok(())
}
