use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::Arc;
use lqos_config::Config;
use crate::errors::UispIntegrationError;
use crate::strategies::full::routes_override::{write_routing_overrides_template, RouteOverride};
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
    config: Arc<Config>,
    sites: &mut Vec<UispSite>,
    root_site: &str,
    overrides: &Vec<RouteOverride>,
) -> Result<(), UispIntegrationError> {

    // Initialize the visualization
    let mut dot_graph = "digraph G {\n  graph [ ranksep=2.0 overlap=false ]\n".to_string();

    // Make sure we know where the root is
    let Some(root_idx) = sites.iter().position(|s| s.name == root_site) else {
        tracing::error!("Unable to build a path-weights graph because I can't find the root node");
        return Err(UispIntegrationError::NoRootSite);
    };

    // Now we iterate through every node that ISN'T the root
    for i in 0..sites.len() {
        // Skip the root. It's not going anywhere.
        if (i == root_idx) {
            continue;
        }

        // We need to find the shortest path to the root
        let parents = sites[i].parent_indices.clone();
        for destination_idx in parents {
            // Is there a route override?
            if let Some(route_override) = overrides.iter().find(|o| o.from_site == sites[i].name && o.to_site == sites[destination_idx].name) {
                sites[i].route_weights.push((destination_idx, route_override.cost));
                continue;
            }

            // If there's a direct route, it makes sense to use it
            if destination_idx == root_idx {
                sites[i].route_weights.push((destination_idx, 10));
                continue;
            }
            // There's no direct route, so we want to evaluate the shortest path
            let mut visited = std::collections::HashSet::new();
            visited.insert(i); // Don't go back to where we came from
            let weight = find_shortest_path(
                destination_idx,
                root_idx,
                visited,
                sites,
                overrides,
                10,
            );
            if let Some(shortest) = weight {
                sites[i].route_weights.push((destination_idx, shortest));
            }
        }
    }

    // Apply the lowest weight route
    let site_index = sites
        .iter()
        .enumerate()
        .map(|(i, site)| (i, site.name.clone()))
        .collect::<std::collections::HashMap<usize, String>>();
    for site in sites.iter_mut() {
        if site.site_type != UispSiteType::Root && !site.route_weights.is_empty() {
            // Sort to find the lowest exit
            site.route_weights.sort_by(|a, b| a.1.cmp(&b.1));
            site.selected_parent = Some(site.route_weights[0].0);
        }

        // Plot it
        for (i,(idx, weight)) in site.route_weights.iter().enumerate() {
            let from = site_index.get(&idx).unwrap().clone();
            let to = site.name.clone();
            if i == 0 {
                dot_graph.push_str(&format!("\"{}\" -> \"{}\" [label=\"{}\" color=\"red\"] \n", from, to, weight));
            } else {
                dot_graph.push_str(&format!("\"{}\" -> \"{}\" [label=\"{}\"] \n", from, to, weight));
            }
        }

    }

    dot_graph.push_str("}\n");
    {
        let graph_file = std::fs::File::create("graph.dot");
        if let Ok(mut file) = graph_file {
            let _ = file.write_all(dot_graph.as_bytes());
        }
    }


    Ok(())
}

fn find_shortest_path(
    from_idx: usize,
    root_idx: usize,
    mut visited: HashSet<usize>,
    sites: &mut Vec<UispSite>,
    overrides: &Vec<RouteOverride>,
    weight: u32,
) -> Option<u32> {
    // Make sure we don't loop
    if visited.contains(&from_idx) {
        return None;
    }
    visited.insert(from_idx);

    let destinations = sites[from_idx].parent_indices.clone();
    for destination_idx in destinations {
        // Is there a route override?
        if let Some(route_override) = overrides.iter().find(|o| o.from_site == sites[from_idx].name && o.to_site == sites[destination_idx].name) {
            return Some(route_override.cost);
        }
        // If there's a direct route, go that way
        if destination_idx == root_idx {
            return Some(weight + 10);
        }
        // Don't go back to where we came from
        if visited.contains(&destination_idx) {
            continue;
        }
        // Calculate the route
        let new_weight = find_shortest_path(destination_idx, root_idx, visited.clone(), sites, overrides, weight + 10);
        if let Some(new_weight) = new_weight {
            return Some(new_weight);
        }
    }

    None
}
