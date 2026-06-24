const TOPOLOGY_EFFECTIVE_PUBLISH_LOCK_FILENAME: &str = "topology_effective_publish.lock";
const TOPOLOGY_EFFECTIVE_PUBLISH_LOCK_GUARD_FILENAME: &str =
    "topology_effective_publish.lock.guard";
const TOPOLOGY_EFFECTIVE_PUBLISH_LOCK_CONTENTION_CODE: &str =
    "LQOS_TOPOLOGY_EFFECTIVE_PUBLISH_LOCKED";
const TOPOLOGY_WARNING_EXAMPLE_LIMIT: usize = 5;
type EffectiveQueueAliasMap = HashMap<String, (String, String)>;

#[derive(Default)]
struct TopologyFallbackWarningSummary {
    unresolved_exported_anchor_count: usize,
    unresolved_exported_anchor_examples: Vec<String>,
    missing_anchor_count: usize,
    missing_anchor_examples: Vec<String>,
    missing_parent_count: usize,
    missing_parent_examples: Vec<String>,
    non_exported_parent_count: usize,
    non_exported_parent_examples: Vec<String>,
}

impl TopologyFallbackWarningSummary {
    fn record_unresolved_exported_anchor(
        &mut self,
        circuit_id: &str,
        anchor_id: &str,
        anchor_name: &str,
    ) {
        self.unresolved_exported_anchor_count += 1;
        push_capped_warning_example(
            &mut self.unresolved_exported_anchor_examples,
            format!("circuit '{circuit_id}' -> anchor '{anchor_id}' ('{anchor_name}')"),
        );
    }

    fn record_missing_anchor(
        &mut self,
        circuit_id: &str,
        anchor_id: &str,
        anchor_name: Option<&str>,
    ) {
        self.missing_anchor_count += 1;
        let example = if let Some(anchor_name) = anchor_name.filter(|value| !value.is_empty()) {
            format!("circuit '{circuit_id}' -> anchor '{anchor_id}' ('{anchor_name}')")
        } else {
            format!("circuit '{circuit_id}' -> anchor '{anchor_id}'")
        };
        push_capped_warning_example(&mut self.missing_anchor_examples, example);
    }

    fn record_missing_parent(&mut self, circuit_id: &str, parent_name: &str, parent_id: &str) {
        self.missing_parent_count += 1;
        push_capped_warning_example(
            &mut self.missing_parent_examples,
            format!("circuit '{circuit_id}' -> parent '{parent_name}' ({parent_id})"),
        );
    }

    fn record_non_exported_parent(&mut self, circuit_id: &str, parent_name: &str, parent_id: &str) {
        self.non_exported_parent_count += 1;
        push_capped_warning_example(
            &mut self.non_exported_parent_examples,
            format!("circuit '{circuit_id}' -> parent '{parent_name}' ({parent_id})"),
        );
    }

    fn append_to(self, warnings: &mut Vec<String>) {
        push_fallback_summary_warning(
            warnings,
            self.unresolved_exported_anchor_count,
            "anchor(s) that did not resolve to exported effective queue nodes",
            &self.unresolved_exported_anchor_examples,
        );
        push_fallback_summary_warning(
            warnings,
            self.missing_anchor_count,
            "anchor(s) that were not found in the effective topology",
            &self.missing_anchor_examples,
        );
        push_fallback_summary_warning(
            warnings,
            self.missing_parent_count,
            "parent reference(s) that were not found in the exported effective topology",
            &self.missing_parent_examples,
        );
        push_fallback_summary_warning(
            warnings,
            self.non_exported_parent_count,
            "resolved parent(s) that were not exported effective queue nodes",
            &self.non_exported_parent_examples,
        );
    }
}

fn push_capped_warning_example(examples: &mut Vec<String>, example: String) {
    if examples.len() < TOPOLOGY_WARNING_EXAMPLE_LIMIT {
        examples.push(example);
    }
}

fn push_fallback_summary_warning(
    warnings: &mut Vec<String>,
    count: usize,
    reason: &str,
    examples: &[String],
) {
    if count == 0 {
        return;
    }
    let mut warning = format!(
        "{count} circuit(s) referenced {reason}. Falling back to generated parent-node shaping."
    );
    if !examples.is_empty() {
        warning.push_str(" Examples: ");
        warning.push_str(&examples.join("; "));
        warning.push('.');
        let omitted_count = count.saturating_sub(examples.len());
        if omitted_count > 0 {
            warning.push_str(&format!(" {omitted_count} more omitted."));
        }
    }
    warnings.push(warning);
}

fn push_unresolved_runtime_fallback_summary(warnings: &mut Vec<String>, count: usize) {
    if count > 0 {
        warnings.push(format!(
            "{count} circuit(s) are unresolved in runtime topology and will be shaped under generated parent nodes."
        ));
    }
}

/// One unique probe pair emitted from topology state plus operator intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentProbeSpec {
    /// Stable pair identifier.
    pub pair_id: String,
    /// Stable attachment identifier.
    pub attachment_id: String,
    /// Display name of the attachment.
    pub attachment_name: String,
    /// Stable node identifier of the child being shaped.
    pub node_id: String,
    /// Display name of the child being shaped.
    pub node_name: String,
    /// Stable parent node identifier for this attachment group.
    pub parent_node_id: String,
    /// Display name of the parent node for this attachment group.
    pub parent_node_name: String,
    /// Local endpoint IP.
    pub local_ip: String,
    /// Remote endpoint IP.
    pub remote_ip: String,
    /// Whether probes are enabled for this pair.
    pub enabled: bool,
}

fn now_unix() -> Option<u64> {
    lqos_utils::unix_time::unix_now().ok()
}

fn atomic_write_json_value(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(value)?;
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(raw.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp_path, path)?;
    Ok(())
}

fn read_json_value(path: &Path) -> Option<Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn effective_state_payload_equals(
    left: &TopologyEffectiveStateFile,
    right: &TopologyEffectiveStateFile,
) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left.generated_unix = None;
    right.generated_unix = None;
    left == right
}

fn optional_non_empty(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn optional_non_empty_owned(raw: Option<String>) -> Option<String> {
    raw.and_then(|value| optional_non_empty(&value))
}
