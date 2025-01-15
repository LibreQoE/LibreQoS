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
    if let Some(root_idx) = sites.iter().position(|s| s.name == root_site) {
        let mut visited = std::collections::HashSet::new();
        let current_node = root_idx;
        let mut natural_weights: Vec<RouteOverride> = Vec::new();
        let mut dot_graph = "digraph G {\n  graph [ ranksep=2.0 overlap=false ]\n".to_string();
        walk_node(current_node, 10, sites, &mut visited, overrides, &mut dot_graph, &mut natural_weights);
        dot_graph.push_str("}\n");
        {
            let graph_file = std::fs::File::create("graph.dot");
            if let Ok(mut file) = graph_file {
                let _ = file.write_all(dot_graph.as_bytes());
            }
        }
        if let Err(e) = write_routing_overrides_template(config, &natural_weights) {
            tracing::error!("Unable to write routing overrides template: {:?}", e);
        } else {
            tracing::info!("Wrote routing overrides template");
        }
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
    dot_graph: &mut String,
    natural_weights: &mut Vec<RouteOverride>,
) {
    if visited.contains(&idx) {
        return;
    }
    visited.insert(idx);
    for i in 0..sites.len() {
        if sites[i].parent_indices.contains(&idx) {
            let from = sites[i].name.clone();
            let to = sites[idx].name.clone();
            if sites[idx].site_type != UispSiteType::Client && sites[i].site_type != UispSiteType::Client
            {
                dot_graph.push_str(&format!("\"{}\" -> \"{}\" [label=\"{}\"] \n", from, to, weight));
                natural_weights.push(RouteOverride {
                    from_site: from.clone(),
                    to_site: to.clone(),
                    cost: weight,
                });
            }
            if let Some(route_override) = overrides
                .iter()
                .find(|o| (o.from_site == from && o.to_site == to) || (o.from_site == to && o.to_site == from))
            {
                sites[i].route_weights.push((idx, route_override.cost));
                tracing::info!("Applied route override {} - {}", route_override.from_site, route_override.to_site);
            } else {
                sites[i].route_weights.push((idx, weight));
            }
            walk_node(i, weight + 10, sites, visited, overrides, dot_graph, natural_weights);
        }
    }
}
