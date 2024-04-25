use lqos_config::Config;
use crate::uisp_types::UispSite;

pub fn correct_zero_capacity_sites(sites: &mut [UispSite], config: &Config) {
    for i in 0 .. sites.len() {
        if sites[i].max_down_mbps == 0 {
            if let Some(parent_idx) = sites[i].selected_parent {
                sites[i].max_down_mbps = sites[parent_idx].max_down_mbps;
            } else {
                sites[i].max_down_mbps = config.queues.generated_pn_download_mbps;
            }
        }

        if sites[i].max_up_mbps == 0 {
            if let Some(parent_idx) = sites[i].selected_parent {
                sites[i].max_up_mbps = sites[parent_idx].max_up_mbps;
            } else {
                sites[i].max_up_mbps = config.queues.generated_pn_upload_mbps;
            }
        }
    }
}