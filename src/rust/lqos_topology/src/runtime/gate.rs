use lqos_overrides::TopologyOverridesFile;
use std::hash::{Hash, Hasher};

use crate::AttachmentProbeSpec;

#[derive(Clone, Debug, Default)]
pub(super) struct RuntimeBuildGate {
    pub(super) last_source_generation: Option<String>,
    pub(super) last_overrides_generation: Option<u64>,
    pub(super) last_health_effective_signature: Option<u64>,
    pub(super) cached_probe_specs: Vec<AttachmentProbeSpec>,
    pub(super) publish_completed: bool,
    pub(super) next_error_retry_after_unix: Option<u64>,
}

pub(super) fn topology_overrides_generation(config: &lqos_config::Config) -> u64 {
    let path = TopologyOverridesFile::path_for_config(config);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    if let Ok(contents) = std::fs::read(&path) {
        match serde_json::from_slice::<TopologyOverridesFile>(&contents)
            .and_then(|overrides| serde_json::to_vec(&overrides))
        {
            Ok(canonical) => canonical.hash(&mut hasher),
            Err(_) => contents.hash(&mut hasher),
        }
    }
    hasher.finish()
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RoundHints {
    pub(super) probes_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::topology_overrides_generation;
    use lqos_config::Config;
    use lqos_overrides::TopologyOverridesFile;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be valid")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()))
    }

    #[test]
    fn topology_overrides_generation_tracks_manual_override_file_changes() {
        let lqos_directory = unique_temp_dir("lqos-topology-runtime-overrides");
        fs::create_dir_all(&lqos_directory).expect("temp lqos directory should exist");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };
        let path = TopologyOverridesFile::path_for_config(&config);

        let before = topology_overrides_generation(&config);
        fs::write(&path, "{\"schemaVersion\":1}\n").expect("override file should write");
        let after = topology_overrides_generation(&config);

        assert_ne!(before, after);
    }
}
