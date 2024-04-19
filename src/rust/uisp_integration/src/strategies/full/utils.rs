use crate::uisp_types::UispSite;
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

pub fn warn_of_no_parents(sites: &[UispSite], devices_raw: &[Device]) {
    sites
        .iter()
        .filter(|s| s.parent_indices.is_empty())
        .for_each(|s| {
            if count_devices_in_site(&s.id, &devices_raw) > 0 {
                warn!("Site: {} has no parents", s.name);
            }
        });
}
