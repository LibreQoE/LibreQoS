use crate::errors::UispIntegrationError;
use crate::uisp_types::{UispSite, UispSiteType};
use std::collections::HashSet;
use tracing::info;

pub fn promote_clients_with_children(
    sites: &mut Vec<UispSite>,
) -> Result<(), UispIntegrationError> {
    info!("Scanning for client sites with child sites");

    let mut client_sites_with_children = Vec::new();

    sites
        .iter()
        .enumerate()
        .filter(|(_, s)| s.site_type == UispSiteType::Client)
        .for_each(|(idx, s)| {
            let child_count = sites
                .iter()
                .filter(|c| c.parent_indices.contains(&idx))
                .count();
            if child_count > 1 {
                client_sites_with_children.push(idx);
            }
        });

    for child_site in client_sites_with_children {
        //info!("Promoting {} to ClientWithChildren", sites[child_site].name);
        sites[child_site].site_type = UispSiteType::ClientWithChildren;
        let old_name = sites[child_site].name.clone();
        sites[child_site].name = format!("(Generated Site) {}", sites[child_site].name);
        let old_id = sites[child_site].id.clone();
        sites[child_site].id = format!("GEN-{}", sites[child_site].id);
        sites[child_site].suspended = false;
        let new_id = sites.len();
        let mut parent_indices = HashSet::new();
        parent_indices.insert(child_site);
        let mut new_site = UispSite {
            id: old_id,
            name: old_name,
            site_type: UispSiteType::Client,
            uisp_parent_id: None,
            parent_indices,
            max_down_mbps: sites[child_site].max_down_mbps,
            max_up_mbps: sites[child_site].max_up_mbps,
            suspended: sites[child_site].suspended,
            ..Default::default()
        };
        new_site
            .device_indices
            .extend_from_slice(&sites[child_site].device_indices);
        sites[child_site].device_indices.clear();
        sites.push(new_site);
    }

    Ok(())
}
