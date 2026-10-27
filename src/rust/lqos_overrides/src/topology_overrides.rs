use std::{
    fs::read_to_string,
    path::{Path, PathBuf},
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::file_lock::FileLock;

const TOPOLOGY_OVERRIDES_FILE: &str = "topology_overrides.json";

/// Operator selection mode for attachment resolution under a chosen parent.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TopologyAttachmentMode {
    /// Resolve the best valid attachment dynamically on every integration run.
    Auto,
    /// Try attachments in saved operator order, then fall back to the best valid one.
    PreferredOrder,
}

/// One persisted branch move inside the topology manager.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TopologyAttachmentOverride {
    /// Stable node identifier of the child branch being moved.
    pub child_node_id: String,
    /// Display name of the child branch.
    pub child_node_name: String,
    /// Stable node identifier of the selected parent branch.
    pub parent_node_id: String,
    /// Display name of the selected parent branch.
    pub parent_node_name: String,
    /// Attachment resolution mode.
    pub mode: TopologyAttachmentMode,
    /// Ranked attachment identifiers valid beneath the selected parent.
    #[serde(default)]
    pub attachment_preference_ids: Vec<String>,
    /// Human-readable labels matching `attachment_preference_ids`.
    #[serde(default)]
    pub attachment_preference_names: Vec<String>,
}

/// Manual probe policy for one attachment pair.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AttachmentProbePolicy {
    /// Stable attachment-pair identifier.
    pub attachment_pair_id: String,
    /// Whether probing is enabled for this pair.
    pub enabled: bool,
}

/// One operator-defined manual attachment option beneath a legal `(child, parent)` pair.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ManualAttachment {
    /// Stable attachment identifier.
    pub attachment_id: String,
    /// Human-readable attachment label.
    pub attachment_name: String,
    /// Capacity for this attachment in Mbps.
    pub capacity_mbps: u64,
    /// Local management IP used for probing.
    pub local_probe_ip: String,
    /// Remote management IP used for probing.
    pub remote_probe_ip: String,
    /// Whether probing is enabled for this attachment pair.
    #[serde(default)]
    pub probe_enabled: bool,
}

/// One persisted attachment-scoped rate override beneath a legal `(child, parent)` pair.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AttachmentRateOverride {
    /// Stable child node identifier.
    pub child_node_id: String,
    /// Stable parent node identifier.
    pub parent_node_id: String,
    /// Stable attachment identifier.
    pub attachment_id: String,
    /// Override download bandwidth in Mbps.
    pub download_bandwidth_mbps: u64,
    /// Override upload bandwidth in Mbps.
    pub upload_bandwidth_mbps: u64,
}

/// One operator-defined manual attachment group beneath a legal `(child, parent)` pair.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ManualAttachmentGroup {
    /// Stable child node identifier.
    pub child_node_id: String,
    /// Human-readable child node name.
    pub child_node_name: String,
    /// Stable parent node identifier.
    pub parent_node_id: String,
    /// Human-readable parent node name.
    pub parent_node_name: String,
    /// Ordered explicit attachments. List order is the saved operator preference order.
    #[serde(default)]
    pub attachments: Vec<ManualAttachment>,
}

/// Persistent operator-owned topology manager state.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TopologyOverridesFile {
    /// Schema version for compatibility checks.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Saved branch reparenting overrides.
    #[serde(default)]
    pub overrides: Vec<TopologyAttachmentOverride>,
    /// Per-pair probe enablement state.
    #[serde(default)]
    pub attachment_probe_policies: Vec<AttachmentProbePolicy>,
    /// Operator-defined attachment groups for legal canonical parent/child relationships.
    #[serde(default)]
    pub manual_attachment_groups: Vec<ManualAttachmentGroup>,
    /// Attachment-scoped rate overrides beneath legal canonical parent/child relationships.
    #[serde(default)]
    pub attachment_rate_overrides: Vec<AttachmentRateOverride>,
}

fn default_schema_version() -> u32 {
    3
}

impl Default for TopologyOverridesFile {
    fn default() -> Self {
        Self {
            schema_version: default_schema_version(),
            overrides: Vec::new(),
            attachment_probe_policies: Vec::new(),
            manual_attachment_groups: Vec::new(),
            attachment_rate_overrides: Vec::new(),
        }
    }
}

fn topology_overrides_path(config: &lqos_config::Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(TOPOLOGY_OVERRIDES_FILE)
}

fn load_from_path(path: &Path) -> Result<TopologyOverridesFile> {
    let raw = read_to_string(path)?;
    let as_json = serde_json::from_str(&raw)?;
    Ok(as_json)
}

fn save_to_path(path: &Path, overrides: &TopologyOverridesFile) -> Result<()> {
    let as_json = serde_json::to_string_pretty(overrides)?;
    let temp_path = path.with_extension("tmp");
    std::fs::write(&temp_path, as_json.as_bytes())?;
    std::fs::rename(&temp_path, path)?;
    Ok(())
}

impl TopologyOverridesFile {
    /// Returns the canonical topology-manager overrides path for `config`.
    ///
    /// This function is pure: it has no side effects.
    pub fn path_for_config(config: &lqos_config::Config) -> PathBuf {
        topology_overrides_path(config)
    }

    /// Loads topology-manager overrides from an explicit filesystem path.
    ///
    /// Side effects: reads the selected overrides file from disk.
    pub fn load_from_explicit_path(path: &Path) -> Result<Self> {
        load_from_path(path)
    }

    /// Saves topology-manager overrides to an explicit filesystem path.
    ///
    /// Side effects: writes the selected overrides file to disk.
    pub fn save_to_explicit_path(&self, path: &Path) -> Result<()> {
        save_to_path(path, self)
    }

    /// True when the section carries no operator topology overrides.
    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
            && self.attachment_probe_policies.is_empty()
            && self.manual_attachment_groups.is_empty()
            && self.attachment_rate_overrides.is_empty()
    }

    /// Loads the topology section from the combined operator overrides file.
    pub fn load() -> Result<Self> {
        let config = lqos_config::load_config()?;
        Self::load_for_config(&config)
    }

    /// Loads the topology section for an EXPLICIT config (the same world the
    /// runtime publisher reads). Phase D: READ-ONLY — the ensure-exists/migrate
    /// legs moved to [`Self::repair`].
    pub fn load_for_config(config: &lqos_config::Config) -> Result<Self> {
        Self::load_for_config_read_only(config)
    }

    /// Phase D: the explicit repair entry for the topology overrides: runs the
    /// legacy migration and ensures the combined operator file exists. Called
    /// by the write path only, never by a read/verify path.
    pub fn repair(config: &lqos_config::Config) -> Result<()> {
        let _ = crate::overrides_file::OverrideFile::load_for_config(config)?;
        Ok(())
    }

    /// Read-only variant of [`Self::load_for_config`]: loads the topology
    /// section WITHOUT running legacy migration or creating a default operator
    /// file. A read path must never write (the scheduler readiness verifier
    /// runs this per tick).
    pub fn load_for_config_read_only(config: &lqos_config::Config) -> Result<Self> {
        let path = crate::overrides_file::OverrideFile::operator_path_for_config(config);
        if !path.exists() {
            return Ok(Self::default());
        }
        let combined = crate::overrides_file::OverrideFile::load_from_explicit_path(&path)?;
        Ok(combined.topology().clone())
    }

    /// Saves this value as the `topology` section of the combined operator
    /// file via a SECTION-MERGE RMW — sibling sections are preserved verbatim
    /// (rung 1.5 §1.2, #12-MED).
    pub fn save(&self) -> Result<()> {
        let lock = FileLock::new_for_operation("save topology overrides")?;
        let config = lqos_config::load_config()?;
        let result = self.save_for_config(&config);
        drop(lock);
        result
    }

    /// Section-merge RMW against an EXPLICIT config (no lock; the caller owns it).
    pub fn save_for_config(&self, config: &lqos_config::Config) -> Result<()> {
        crate::overrides_file::migrate_legacy_topology_overrides(config)?;
        let path = crate::overrides_file::OverrideFile::operator_path_for_config(config);
        let mut combined = if path.exists() {
            crate::overrides_file::OverrideFile::load_from_explicit_path(&path)?
        } else {
            crate::overrides_file::OverrideFile::default()
        };
        combined.set_topology(self.clone());
        crate::overrides_file::OverrideFile::save_to_explicit_path(&combined, &path)
    }

    /// Returns the stored override for `child_node_id`, if present.
    ///
    /// This function is pure: it has no side effects.
    pub fn find_override(&self, child_node_id: &str) -> Option<&TopologyAttachmentOverride> {
        self.overrides
            .iter()
            .find(|entry| entry.child_node_id == child_node_id)
    }

    /// Adds or replaces the stored override for `child_node_id`. Returns true if changed.
    pub fn set_override_return_changed(
        &mut self,
        child_node_id: String,
        child_node_name: String,
        parent_node_id: String,
        parent_node_name: String,
        mode: TopologyAttachmentMode,
        attachment_preferences: Vec<(String, String)>,
    ) -> bool {
        let normalized_child_node_id = child_node_id.trim().to_string();
        let desired = {
            let mut attachment_preference_ids = Vec::new();
            let mut attachment_preference_names = Vec::new();
            let mut seen_ids = std::collections::HashSet::new();
            for (id, name) in attachment_preferences {
                let trimmed_id = id.trim();
                if trimmed_id.is_empty() || !seen_ids.insert(trimmed_id.to_string()) {
                    continue;
                }
                attachment_preference_ids.push(trimmed_id.to_string());
                attachment_preference_names.push(name.trim().to_string());
            }

            TopologyAttachmentOverride {
                child_node_id: normalized_child_node_id.clone(),
                child_node_name: child_node_name.trim().to_string(),
                parent_node_id: parent_node_id.trim().to_string(),
                parent_node_name: parent_node_name.trim().to_string(),
                mode,
                attachment_preference_ids,
                attachment_preference_names,
            }
        };

        if self.find_override(&normalized_child_node_id) == Some(&desired) {
            return false;
        }

        self.overrides
            .retain(|entry| entry.child_node_id != normalized_child_node_id);
        self.overrides.push(desired);
        self.overrides
            .sort_unstable_by(|left, right| left.child_node_id.cmp(&right.child_node_id));
        true
    }

    /// Removes any override for `child_node_id`. Returns the number removed.
    pub fn remove_override_by_child_node_id_count(&mut self, child_node_id: &str) -> usize {
        let before = self.overrides.len();
        self.overrides
            .retain(|entry| entry.child_node_id != child_node_id);
        before.saturating_sub(self.overrides.len())
    }

    /// Returns the saved probe policy for `attachment_pair_id`, if present.
    pub fn find_probe_policy(&self, attachment_pair_id: &str) -> Option<&AttachmentProbePolicy> {
        self.attachment_probe_policies
            .iter()
            .find(|entry| entry.attachment_pair_id == attachment_pair_id)
    }

    /// Adds or replaces probe enablement for `attachment_pair_id`. Returns true if changed.
    pub fn set_probe_policy_return_changed(
        &mut self,
        attachment_pair_id: String,
        enabled: bool,
    ) -> bool {
        let normalized_pair_id = attachment_pair_id.trim().to_string();
        if normalized_pair_id.is_empty() {
            return false;
        }

        let desired = AttachmentProbePolicy {
            attachment_pair_id: normalized_pair_id.clone(),
            enabled,
        };
        if self.find_probe_policy(&normalized_pair_id) == Some(&desired) {
            return false;
        }

        self.attachment_probe_policies
            .retain(|entry| entry.attachment_pair_id != normalized_pair_id);
        self.attachment_probe_policies.push(desired);
        self.attachment_probe_policies
            .sort_unstable_by(|left, right| left.attachment_pair_id.cmp(&right.attachment_pair_id));
        true
    }

    /// Removes any saved probe policy for `attachment_pair_id`. Returns the number removed.
    pub fn remove_probe_policy_count(&mut self, attachment_pair_id: &str) -> usize {
        let before = self.attachment_probe_policies.len();
        self.attachment_probe_policies
            .retain(|entry| entry.attachment_pair_id != attachment_pair_id);
        before.saturating_sub(self.attachment_probe_policies.len())
    }

    /// Returns the saved attachment rate override for `(child_node_id, parent_node_id,
    /// attachment_id)`, if present.
    pub fn find_attachment_rate_override(
        &self,
        child_node_id: &str,
        parent_node_id: &str,
        attachment_id: &str,
    ) -> Option<&AttachmentRateOverride> {
        self.attachment_rate_overrides.iter().find(|entry| {
            entry.child_node_id == child_node_id
                && entry.parent_node_id == parent_node_id
                && entry.attachment_id == attachment_id
        })
    }

    /// Adds or replaces an attachment-scoped rate override. Returns true if changed.
    pub fn set_attachment_rate_override_return_changed(
        &mut self,
        child_node_id: String,
        parent_node_id: String,
        attachment_id: String,
        download_bandwidth_mbps: u64,
        upload_bandwidth_mbps: u64,
    ) -> bool {
        let normalized_child_node_id = child_node_id.trim().to_string();
        let normalized_parent_node_id = parent_node_id.trim().to_string();
        let normalized_attachment_id = attachment_id.trim().to_string();
        if normalized_child_node_id.is_empty()
            || normalized_parent_node_id.is_empty()
            || normalized_attachment_id.is_empty()
            || download_bandwidth_mbps == 0
            || upload_bandwidth_mbps == 0
        {
            return false;
        }

        let desired = AttachmentRateOverride {
            child_node_id: normalized_child_node_id.clone(),
            parent_node_id: normalized_parent_node_id.clone(),
            attachment_id: normalized_attachment_id.clone(),
            download_bandwidth_mbps,
            upload_bandwidth_mbps,
        };

        if self.find_attachment_rate_override(
            &normalized_child_node_id,
            &normalized_parent_node_id,
            &normalized_attachment_id,
        ) == Some(&desired)
        {
            return false;
        }

        self.attachment_rate_overrides.retain(|entry| {
            !(entry.child_node_id == normalized_child_node_id
                && entry.parent_node_id == normalized_parent_node_id
                && entry.attachment_id == normalized_attachment_id)
        });
        self.attachment_rate_overrides.push(desired);
        self.attachment_rate_overrides
            .sort_unstable_by(|left, right| {
                left.child_node_id
                    .cmp(&right.child_node_id)
                    .then(left.parent_node_id.cmp(&right.parent_node_id))
                    .then(left.attachment_id.cmp(&right.attachment_id))
            });
        true
    }

    /// Removes any saved attachment rate override for `(child_node_id, parent_node_id,
    /// attachment_id)`. Returns the number removed.
    pub fn remove_attachment_rate_override_count(
        &mut self,
        child_node_id: &str,
        parent_node_id: &str,
        attachment_id: &str,
    ) -> usize {
        let before = self.attachment_rate_overrides.len();
        self.attachment_rate_overrides.retain(|entry| {
            !(entry.child_node_id == child_node_id
                && entry.parent_node_id == parent_node_id
                && entry.attachment_id == attachment_id)
        });
        before.saturating_sub(self.attachment_rate_overrides.len())
    }

    /// Returns the saved manual attachment group for `(child_node_id, parent_node_id)`, if present.
    pub fn find_manual_attachment_group(
        &self,
        child_node_id: &str,
        parent_node_id: &str,
    ) -> Option<&ManualAttachmentGroup> {
        self.manual_attachment_groups.iter().find(|entry| {
            entry.child_node_id == child_node_id && entry.parent_node_id == parent_node_id
        })
    }

    /// Adds or replaces the saved manual attachment group. Returns true if changed.
    pub fn set_manual_attachment_group_return_changed(
        &mut self,
        child_node_id: String,
        child_node_name: String,
        parent_node_id: String,
        parent_node_name: String,
        attachments: Vec<ManualAttachment>,
    ) -> bool {
        let normalized_child_node_id = child_node_id.trim().to_string();
        let normalized_parent_node_id = parent_node_id.trim().to_string();
        if normalized_child_node_id.is_empty() || normalized_parent_node_id.is_empty() {
            return false;
        }

        let desired = ManualAttachmentGroup {
            child_node_id: normalized_child_node_id.clone(),
            child_node_name: child_node_name.trim().to_string(),
            parent_node_id: normalized_parent_node_id.clone(),
            parent_node_name: parent_node_name.trim().to_string(),
            attachments,
        };

        if self.find_manual_attachment_group(&normalized_child_node_id, &normalized_parent_node_id)
            == Some(&desired)
        {
            return false;
        }

        self.manual_attachment_groups.retain(|entry| {
            !(entry.child_node_id == normalized_child_node_id
                && entry.parent_node_id == normalized_parent_node_id)
        });
        self.manual_attachment_groups.push(desired);
        self.manual_attachment_groups
            .sort_unstable_by(|left, right| {
                left.child_node_id
                    .cmp(&right.child_node_id)
                    .then(left.parent_node_id.cmp(&right.parent_node_id))
            });
        true
    }

    /// Removes any saved manual attachment group for `(child_node_id, parent_node_id)`.
    pub fn remove_manual_attachment_group_count(
        &mut self,
        child_node_id: &str,
        parent_node_id: &str,
    ) -> usize {
        let before = self.manual_attachment_groups.len();
        self.manual_attachment_groups.retain(|entry| {
            !(entry.child_node_id == child_node_id && entry.parent_node_id == parent_node_id)
        });
        before.saturating_sub(self.manual_attachment_groups.len())
    }
}

#[cfg(test)]
mod tests {
    use super::TopologyOverridesFile;

    #[test]
    fn default_section_is_empty() {
        let section = TopologyOverridesFile::default();
        assert!(section.is_empty());
    }
}
