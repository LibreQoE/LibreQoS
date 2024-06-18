use crate::errors::UispIntegrationError;
use crate::uisp_types::{UispSite, UispSiteType};

/// Squashes single entry access points
/// 
/// This function will squash access points that have only one child site.
/// 
/// # Arguments
/// * `sites` - The list of sites to modify
/// 
/// # Returns
/// * An `Ok` if the operation was successful
pub fn squash_single_aps(sites: &mut [UispSite]) -> Result<(), UispIntegrationError> {
    let mut squashable = Vec::new();
    for (idx, site) in sites.iter().enumerate() {
        if site.site_type == UispSiteType::AccessPoint {
            let target_count = sites
                .iter()
                .filter(|s| s.parent_indices.contains(&idx))
                .count();
            if target_count == 1 && site.parent_indices.len() == 1 {
                //tracing::info!("Site {} has only one child and is therefore eligible for squashing.", site.name);
                squashable.push(idx);
            }
        }
    }
    for squash_idx in squashable {
        sites[squash_idx].site_type = UispSiteType::SquashDeleted;
        sites[squash_idx].name += " (SQUASHED)";
        let up = sites[squash_idx].max_up_mbps;
        let down = sites[squash_idx].max_down_mbps;
        let new_parent = *sites[squash_idx].parent_indices.iter().next().unwrap();
        sites.iter_mut().for_each(|s| {
            if s.parent_indices.contains(&squash_idx) {
                s.parent_indices.remove(&squash_idx);
                s.parent_indices.insert(new_parent);
                if s.site_type == UispSiteType::Client {
                    s.max_up_mbps = u32::min(up, s.max_up_mbps);
                    s.max_down_mbps = u32::min(down, s.max_down_mbps);
                }
            }
        });
        sites[squash_idx].parent_indices.clear();
    }

    Ok(())
}
