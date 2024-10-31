use crate::errors::UispIntegrationError;
use crate::strategies::full::routes_override::RouteOverride;
use crate::uisp_types::{UispSite, UispSiteType};

/// Walks the tree to determine the best route for each site
/// 
/// This function will walk the tree to determine the best route for each site.
/// 
/// # Arguments
/// * `sites` - The list of sites
/// * `root_site` - The name of the root site
/// * `overrides` - The list of route overrides
pub fn walk_tree_for_routing(
    sites: &mut Vec<UispSite>,
    root_site: &str,
    overrides: &Vec<RouteOverride>,
) -> Result<(), UispIntegrationError> {
    if let Some(root_idx) = sites.iter().position(|s| s.name == root_site) {
        let mut visited = std::collections::HashSet::new();
        let current_node = root_idx;
        walk_node(current_node, 10, sites, &mut visited, overrides);
    } else {
        tracing::error!("Unable to build a path-weights graph because I can't find the root node");
        return Err(UispIntegrationError::NoRootSite);
    }

    // Apply the lowest weight route
    for site in sites.iter_mut() {
        if site.site_type != UispSiteType::Root && !site.route_weights.is_empty() {
            // Sort to find the lowest exit
            site.route_weights.sort_by(|a, b| a.1.cmp(&b.1));
            site.selected_parent = Some(site.route_weights[0].0);
        }
    }

    Ok(())
}

fn walk_node(
    idx: usize,
    weight: u32,
    sites: &mut Vec<UispSite>,
    visited: &mut std::collections::HashSet<usize>,
    overrides: &Vec<RouteOverride>,
) {
    if visited.contains(&idx) {
        return;
    }
    visited.insert(idx);
    for i in 0..sites.len() {
        if sites[i].parent_indices.contains(&idx) {
            let from = sites[i].name.clone();
            let to = sites[idx].name.clone();
            if let Some(route_override) = overrides
                .iter()
                .find(|o| (o.from_site == from && o.to_site == to) || (o.from_site == to && o.to_site == from))
            {
                sites[i].route_weights.push((idx, route_override.cost));
                tracing::info!("Applied route override {} - {}", route_override.from_site, route_override.to_site);
            } else {
                sites[i].route_weights.push((idx, weight));
            }
            walk_node(i, weight + 10, sites, visited, overrides);
        }
    }
}
