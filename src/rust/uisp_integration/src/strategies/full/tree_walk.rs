use crate::errors::UispIntegrationError;
use crate::strategies::full::walk_node;
use crate::uisp_types::{UispSite, UispSiteType};

pub fn walk_tree_for_routing(
    sites: &mut Vec<UispSite>,
    root_site: &str,
) -> Result<(), UispIntegrationError> {
    if let Some(root_idx) = sites.iter().position(|s| s.name == root_site) {
        let mut visited = std::collections::HashSet::new();
        let mut current_node = root_idx;
        walk_node(current_node, 10, sites, &mut visited);
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
