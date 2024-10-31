use crate::uisp_types::{UispSite, UispSiteType};
use lqos_config::Config;
use tracing::warn;
use uisp::Device;

/// Counts how many devices are present at a siteId. It's a simple
/// iteration of the devices.
pub fn count_devices_in_site(site_id: &str, devices: &[Device]) -> usize {
    devices
        .iter()
        .filter(|d| {
            if let Some(site) = &d.identification.site {
                if let Some(parent) = &site.parent {
                    if parent.id == site_id {
                        return true;
                    }
                }
            }
            false
        })
        .count()
}

/// Utility function to dump the site tree to the console.
/// Useful for debugging.
pub fn print_sites(sites: &[UispSite], root_idx: usize) {
    println!("{}", sites[root_idx].name);
    iterate_child_sites(sites, root_idx, 2);
}

fn iterate_child_sites(sites: &[UispSite], parent: usize, indent: usize) {
    sites
        .iter()
        .enumerate()
        .filter(|(_, s)| s.selected_parent == Some(parent))
        .for_each(|(i, s)| {
            // Indent print
            for _ in 0..indent {
                print!("-");
            }
            s.print_tree_summary();
            println!();
            if indent < 20 {
                iterate_child_sites(sites, i, indent + 2);
            }
        });
}

/// Warns if there are any sites with no parents, and promotes them to be parented off of the root
/// site.
/// 
/// # Arguments
/// * `sites` - The list of sites
/// * `devices_raw` - The raw device data
/// * `root_idx` - The index of the root site
/// * `config` - The configuration
pub fn warn_of_no_parents_and_promote(
    sites: &mut Vec<UispSite>,
    devices_raw: &[Device],
    root_idx: usize,
    config: &Config,
) {
    let mut orphans = Vec::new();

    sites
        .iter()
        .filter(|s| s.selected_parent.is_none())
        .for_each(|s| {
            if count_devices_in_site(&s.id, devices_raw) > 0 {
                warn!("Site: {} has no parents", s.name);
                orphans.push(s.id.clone());
            }
        });

    // If we have orphans, promote them to be parented off of a special branch
    if !orphans.is_empty() {
        let orgphanage_id = sites.len();
        let orphanage = UispSite {
            id: "orphans".to_string(),
            name: "Orphaned Nodes".to_string(),
            site_type: UispSiteType::Site,
            uisp_parent_id: None,
            parent_indices: Default::default(),
            max_down_mbps: config.queues.downlink_bandwidth_mbps,
            max_up_mbps: config.queues.uplink_bandwidth_mbps,
            suspended: false,
            device_indices: vec![],
            route_weights: vec![],
            selected_parent: Some(root_idx),
        };
        sites.push(orphanage);

        for orphan_id in orphans {
            if let Some((_, site)) = sites
                .iter_mut()
                .enumerate()
                .find(|(idx, s)| *idx != root_idx && s.id == orphan_id)
            {
                site.selected_parent = Some(orgphanage_id);
            }
        }
    }
}
