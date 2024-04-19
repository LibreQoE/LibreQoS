use crate::uisp_types::{UispSite, UispSiteType};
use std::collections::HashSet;
use tracing::info;
use uisp::{DataLink, Device, Site};

pub fn promote_access_points(
    sites: &mut Vec<UispSite>,
    devices_raw: &[Device],
    data_links_raw: &[DataLink],
    sites_raw: &[Site],
) {
    let mut all_links = Vec::new();
    sites.iter().for_each(|s| {
        let links = s.find_aps(&devices_raw, &data_links_raw, &sites_raw);
        if !links.is_empty() {
            all_links.extend(links);
        }
    });
    info!("Detected {} intra-site links", all_links.len());

    // Insert AP entries
    for link in all_links {
        // Create the new AP site
        let parent_site_id = sites.iter().position(|s| s.id == link.site_id).unwrap();
        /*if sites[parent_site_id].site_type == UispSiteType::Client {
            warn!(
                "{} is a client, but has an AP pointing at other locations",
                sites[parent_site_id].name
            );
        }*/
        let mut new_site = UispSite {
            id: link.device_id,
            name: link.device_name,
            site_type: UispSiteType::AccessPoint,
            uisp_parent_id: None,
            parent_indices: HashSet::new(),
            max_up_mbps: 0, // TODO: I need to read this from the device capacity
            max_down_mbps: 0,
            ..Default::default()
        };
        new_site.parent_indices.insert(parent_site_id);

        // Add it
        let new_id = sites.len();
        sites.push(new_site);
        sites.iter_mut().for_each(|s| {
            if link.child_sites.contains(&s.id) {
                s.parent_indices.insert(new_id);
            }
        });
    }
}
