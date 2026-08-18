use crate::BakeryCommands;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};

const SITE_DELTA_PREVIEW_THRESHOLD: usize = 8;

/// Structured metadata about a site-level structural diff that forced a full rebuild.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StructuralSiteDiffDetails {
    /// Stable Bakery site hash for the structurally changed site.
    pub(crate) site_hash: i64,
}

pub(crate) enum SiteDiffResult {
    RebuildRequired {
        summary: String,
        details: Option<StructuralSiteDiffDetails>,
    },
    SpeedChanges {
        changes: Vec<BakeryCommands>,
    },
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
        let (added_site_hashes, removed_site_hashes) = site_set_delta(old_sites, &new_sites);
        warn!(
            "Site count mismatch: old {} vs new {}",
            old_sites.len(),
            new_sites.len()
        );
        debug!("Old site hashes: {:?}", old_keys);
        debug!("New site hashes: {:?}", new_keys);
        debug!("Added site hashes: {:?}", added_site_hashes);
        debug!("Removed site hashes: {:?}", removed_site_hashes);
        return SiteDiffResult::RebuildRequired {
            summary: format_site_delta_summary(
                "site_count_mismatch",
                None,
                old_sites.len(),
                new_sites.len(),
                &added_site_hashes,
                &removed_site_hashes,
            ),
            details: None,
        };
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
                return SiteDiffResult::RebuildRequired {
                    summary: format!(
                        "Bakery full reload triggered by site diff: site_hash={} parent={}→{} up_parent={}→{} minor=0x{:x}→0x{:x}",
                        site_hash, opar, npar, oup, nup, omin, nmin
                    ),
                    details: Some(StructuralSiteDiffDetails {
                        site_hash: *site_hash,
                    }),
                };
            }
            // If the speeds have changed, we need to store the change.
            if let Some(speed_change) = site_speeds_changed(old_cmd.as_ref(), new_cmd.as_ref()) {
                debug!("Speed change detected for site hash: {}", site_hash);
                speed_changes.push(speed_change);
            }
        } else {
            // If a site is missing in the new batch, we need to rebuild.
            debug!("Site hash {} is missing in the new batch", site_hash);
            let (added_site_hashes, removed_site_hashes) = site_set_delta(old_sites, &new_sites);
            debug!("Added site hashes: {:?}", added_site_hashes);
            debug!("Removed site hashes: {:?}", removed_site_hashes);
            return SiteDiffResult::RebuildRequired {
                summary: format_site_delta_summary(
                    "site_set_changed",
                    Some(*site_hash),
                    old_sites.len(),
                    new_sites.len(),
                    &added_site_hashes,
                    &removed_site_hashes,
                ),
                details: None,
            };
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

fn site_set_delta(
    old_sites: &HashMap<i64, Arc<BakeryCommands>>,
    new_sites: &HashMap<i64, &Arc<BakeryCommands>>,
) -> (Vec<i64>, Vec<i64>) {
    let mut added_site_hashes: Vec<i64> = new_sites
        .keys()
        .filter(|hash| !old_sites.contains_key(hash))
        .copied()
        .collect();
    added_site_hashes.sort_unstable();

    let mut removed_site_hashes: Vec<i64> = old_sites
        .keys()
        .filter(|hash| !new_sites.contains_key(hash))
        .copied()
        .collect();
    removed_site_hashes.sort_unstable();

    (added_site_hashes, removed_site_hashes)
}

fn format_site_hash_preview(label: &str, hashes: &[i64], total_changed: usize) -> Option<String> {
    if hashes.is_empty()
        || hashes.len() > SITE_DELTA_PREVIEW_THRESHOLD
        || total_changed > SITE_DELTA_PREVIEW_THRESHOLD
    {
        return None;
    }
    Some(format!(" {label}={hashes:?}"))
}

fn format_site_delta_summary(
    reason: &str,
    missing_old_site_hash: Option<i64>,
    old_count: usize,
    new_count: usize,
    added_site_hashes: &[i64],
    removed_site_hashes: &[i64],
) -> String {
    let mut summary = format!(
        "Bakery full reload triggered by site diff: {reason} old_count={old_count} new_count={new_count} added_count={} removed_count={}",
        added_site_hashes.len(),
        removed_site_hashes.len()
    );
    let total_changed = added_site_hashes.len() + removed_site_hashes.len();
    if let Some(site_hash) = missing_old_site_hash {
        summary.push_str(&format!(" missing_old_site_hash={site_hash}"));
    }
    if let Some(preview) =
        format_site_hash_preview("added_preview", added_site_hashes, total_changed)
    {
        summary.push_str(&preview);
    }
    if let Some(preview) =
        format_site_hash_preview("removed_preview", removed_site_hashes, total_changed)
    {
        summary.push_str(&preview);
    }
    summary
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

#[cfg(test)]
mod tests {
    use super::{SiteDiffResult, StructuralSiteDiffDetails, diff_sites};
    use crate::BakeryCommands;
    use lqos_bus::TcHandle;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[allow(clippy::too_many_arguments)]
    fn add_site(
        site_hash: i64,
        parent_class_id: u32,
        up_parent_class_id: u32,
        class_minor: u16,
        dl_min: f32,
        ul_min: f32,
        dl_max: f32,
        ul_max: f32,
    ) -> Arc<BakeryCommands> {
        Arc::new(BakeryCommands::AddSite {
            site_hash,
            parent_class_id: TcHandle::from_u32(parent_class_id),
            up_parent_class_id: TcHandle::from_u32(up_parent_class_id),
            class_minor,
            download_bandwidth_min: dl_min,
            upload_bandwidth_min: ul_min,
            download_bandwidth_max: dl_max,
            upload_bandwidth_max: ul_max,
        })
    }

    #[test]
    fn site_count_mismatch_reports_added_and_removed_hashes() {
        let old_site = add_site(10, 0x10010, 0x20010, 0x11, 1.0, 1.0, 10.0, 10.0);
        let new_site = add_site(20, 0x10020, 0x20020, 0x21, 1.0, 1.0, 10.0, 10.0);
        let old_sites = HashMap::from([(10, old_site)]);
        let batch = vec![
            new_site,
            add_site(30, 0x10030, 0x20030, 0x31, 1.0, 1.0, 10.0, 10.0),
        ];

        let SiteDiffResult::RebuildRequired { summary, details } = diff_sites(&batch, &old_sites)
        else {
            panic!("expected rebuild-required site diff");
        };

        assert!(details.is_none());
        assert!(summary.contains("site_count_mismatch"));
        assert!(summary.contains("added_count=2"));
        assert!(summary.contains("removed_count=1"));
        assert!(summary.contains("added_preview=[20, 30]"));
        assert!(summary.contains("removed_preview=[10]"));
    }

    #[test]
    fn large_site_count_mismatch_stays_counts_only() {
        let old_sites = HashMap::from([(
            10,
            add_site(10, 0x10010, 0x20010, 0x11, 1.0, 1.0, 10.0, 10.0),
        )]);
        let mut batch = Vec::new();
        for index in 20..30 {
            batch.push(add_site(
                index,
                0x10000 | index as u32,
                0x20000 | index as u32,
                index as u16,
                1.0,
                1.0,
                10.0,
                10.0,
            ));
        }

        let SiteDiffResult::RebuildRequired { summary, details } = diff_sites(&batch, &old_sites)
        else {
            panic!("expected rebuild-required site diff");
        };

        assert!(details.is_none());
        assert!(summary.contains("site_count_mismatch"));
        assert!(summary.contains("added_count=10"));
        assert!(summary.contains("removed_count=1"));
        assert!(!summary.contains("added_preview="));
        assert!(!summary.contains("removed_preview="));
    }

    #[test]
    fn structural_site_change_reports_parent_transition() {
        let old_site = add_site(10, 0x10010, 0x20010, 0x11, 1.0, 1.0, 10.0, 10.0);
        let new_site = add_site(10, 0x10020, 0x20020, 0x21, 1.0, 1.0, 10.0, 10.0);
        let old_sites = HashMap::from([(10, old_site)]);
        let batch = vec![new_site];

        let SiteDiffResult::RebuildRequired { summary, details } = diff_sites(&batch, &old_sites)
        else {
            panic!("expected rebuild-required site diff");
        };

        assert_eq!(details, Some(StructuralSiteDiffDetails { site_hash: 10 }));
        assert!(summary.contains("site_hash=10"));
        assert!(summary.contains("parent=0x1:0x10→0x1:0x20"));
        assert!(summary.contains("up_parent=0x2:0x10→0x2:0x20"));
        assert!(summary.contains("minor=0x11→0x21"));
    }
}
