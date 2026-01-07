use crate::BakeryCommands;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};

pub(crate) enum SiteDiffResult {
    RebuildRequired,
    SpeedChanges { changes: Vec<BakeryCommands> },
    NoChange,
}

pub(crate) fn diff_sites(
    batch: &[Arc<BakeryCommands>],
    old_sites: &HashMap<i64, Arc<BakeryCommands>>,
) -> SiteDiffResult {
    let new_sites: HashMap<i64, &Arc<BakeryCommands>> = batch
        .iter()
        .filter_map(|cmd| {
            if let BakeryCommands::AddSite { site_hash, .. } = cmd.as_ref() {
                Some((*site_hash, cmd))
            } else {
                None
            }
        })
        .collect();

    if old_sites.len() != new_sites.len() {
        // There is a difference in the number of sites.
        // Therefore, we must rebuild the entire site configuration.
        let old_keys: Vec<i64> = old_sites.keys().cloned().collect();
        let new_keys: Vec<i64> = new_sites.keys().copied().collect();
        warn!(
            "Site count mismatch: old {} vs new {}",
            old_sites.len(),
            new_sites.len()
        );
        debug!("Old site hashes: {:?}", old_keys);
        debug!("New site hashes: {:?}", new_keys);
        return SiteDiffResult::RebuildRequired;
    }

    // Compare each site in the old and new maps for structural differences.
    let mut speed_changes = Vec::new();
    for (site_hash, old_cmd) in old_sites {
        if let Some(new_cmd) = new_sites.get(site_hash) {
            // If the commands are structurally different, we need to rebuild.
            if is_structurally_different(old_cmd.as_ref(), new_cmd.as_ref()) {
                debug!(
                    "Structural difference detected for site hash: {}",
                    site_hash
                );
                // Log a concise before/after for diagnostics at warn! level so operators
                // can see why the site is considered structurally different.
                let (_ocpu, opar, oup, omin) = match old_cmd.as_ref() {
                    crate::BakeryCommands::AddSite {
                        parent_class_id,
                        up_parent_class_id,
                        class_minor,
                        ..
                    } => (
                        0i32,
                        parent_class_id.as_tc_string(),
                        up_parent_class_id.as_tc_string(),
                        *class_minor,
                    ),
                    _ => (0, String::new(), String::new(), 0),
                };
                let (_ncpu, npar, nup, nmin) = match new_cmd.as_ref() {
                    crate::BakeryCommands::AddSite {
                        parent_class_id,
                        up_parent_class_id,
                        class_minor,
                        ..
                    } => (
                        0i32,
                        parent_class_id.as_tc_string(),
                        up_parent_class_id.as_tc_string(),
                        *class_minor,
                    ),
                    _ => (0, String::new(), String::new(), 0),
                };
                warn!(
                    "Site hash {} change detail: parent={}→{}, up_parent={}→{}, minor=0x{:x}→0x{:x}",
                    site_hash, opar, npar, oup, nup, omin, nmin
                );
                return SiteDiffResult::RebuildRequired;
            }
            // If the speeds have changed, we need to store the change.
            if let Some(speed_change) = site_speeds_changed(old_cmd.as_ref(), new_cmd.as_ref()) {
                debug!("Speed change detected for site hash: {}", site_hash);
                speed_changes.push(speed_change);
            }
        } else {
            // If a site is missing in the new batch, we need to rebuild.
            debug!("Site hash {} is missing in the new batch", site_hash);
            return SiteDiffResult::RebuildRequired;
        }
    }

    // If we have speed changes, return them.
    if !speed_changes.is_empty() {
        return SiteDiffResult::SpeedChanges {
            changes: speed_changes,
        };
    }

    SiteDiffResult::NoChange
}

fn is_structurally_different(a: &BakeryCommands, b: &BakeryCommands) -> bool {
    let BakeryCommands::AddSite {
        site_hash,
        parent_class_id,
        up_parent_class_id,
        class_minor,
        ..
    } = a
    else {
        debug!(
            "is_structurally_different called with non-site command: {:?}",
            a
        );
        return false; // Not a site command
    };

    let BakeryCommands::AddSite {
        site_hash: b_site_hash,
        parent_class_id: b_parent_class_id,
        up_parent_class_id: b_up_parent_class_id,
        class_minor: b_class_minor,
        ..
    } = b
    else {
        debug!(
            "is_structurally_different called with non-site command: {:?}",
            b
        );
        return false; // Not a site command
    };

    if site_hash != b_site_hash {
        // This should never happen.
        debug!(
            "is_structurally_different called for different site hashes: {} != {}",
            site_hash, b_site_hash
        );
        return false;
    }

    // If the classes are different, it's different.
    parent_class_id != b_parent_class_id
        || up_parent_class_id != b_up_parent_class_id
        || class_minor != b_class_minor
}

fn site_speeds_changed(a: &BakeryCommands, b: &BakeryCommands) -> Option<BakeryCommands> {
    let BakeryCommands::AddSite {
        site_hash,
        parent_class_id,
        up_parent_class_id,
        class_minor,
        download_bandwidth_min,
        upload_bandwidth_min,
        download_bandwidth_max,
        upload_bandwidth_max,
    } = a
    else {
        debug!("site_speeds_changed called with non-site command: {:?}", a);
        return None; // Not a site command
    };

    let BakeryCommands::AddSite {
        site_hash: b_site_hash,
        download_bandwidth_min: b_download_bandwidth_min,
        upload_bandwidth_min: b_upload_bandwidth_min,
        download_bandwidth_max: b_download_bandwidth_max,
        upload_bandwidth_max: b_upload_bandwidth_max,
        ..
    } = b
    else {
        debug!("site_speeds_changed called with non-site command: {:?}", b);
        return None; // Not a site command
    };

    if site_hash != b_site_hash {
        // This should never happen.
        debug!(
            "site_speeds_changed called for different site hashes: {} != {}",
            site_hash, b_site_hash
        );
        return None;
    }

    // If the speeds are different, return a new command with the updated speeds.
    if download_bandwidth_min != b_download_bandwidth_min
        || upload_bandwidth_min != b_upload_bandwidth_min
        || download_bandwidth_max != b_download_bandwidth_max
        || upload_bandwidth_max != b_upload_bandwidth_max
    {
        Some(BakeryCommands::AddSite {
            site_hash: *site_hash,
            parent_class_id: *parent_class_id,
            up_parent_class_id: *up_parent_class_id,
            class_minor: *class_minor,
            download_bandwidth_min: *b_download_bandwidth_min,
            upload_bandwidth_min: *b_upload_bandwidth_min,
            download_bandwidth_max: *b_download_bandwidth_max,
            upload_bandwidth_max: *b_upload_bandwidth_max,
        })
    } else {
        None
    }
}
