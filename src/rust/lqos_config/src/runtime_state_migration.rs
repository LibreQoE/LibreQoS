use crate::{
    CIRCUIT_ANCHORS_FILENAME, CIRCUIT_ETHERNET_METADATA_FILENAME, Config,
    TOPOLOGY_ATTACHMENT_HEALTH_STATE_FILENAME, TOPOLOGY_CANONICAL_STATE_FILENAME,
    TOPOLOGY_COMPILED_SHAPING_FILENAME, TOPOLOGY_EDITOR_STATE_FILENAME,
    TOPOLOGY_EFFECTIVE_NETWORK_FILENAME, TOPOLOGY_EFFECTIVE_STATE_FILENAME,
    TOPOLOGY_IMPORT_FILENAME, TOPOLOGY_PARENT_CANDIDATES_FILENAME,
    TOPOLOGY_RUNTIME_STATUS_FILENAME, TOPOLOGY_SHAPING_INPUTS_FILENAME,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tracing::info;

#[derive(Debug, Error)]
pub(crate) enum RuntimeStateMigrationError {
    #[error("Unable to create directory {path}: {source}")]
    CreateDirectory {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Unable to read runtime directory {path}: {source}")]
    ReadDirectory {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Unable to move legacy runtime artifact from {from} to {to}: {source}")]
    Move {
        from: String,
        to: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Unable to remove copied legacy runtime file {path}: {source}")]
    RemoveFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Unable to remove copied legacy runtime directory {path}: {source}")]
    RemoveDirectory {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

enum LegacyArtifactKind {
    File,
    Directory,
}

struct LegacyMigration<'a> {
    legacy_name: &'a str,
    destination: PathBuf,
    kind: LegacyArtifactKind,
}

pub(crate) fn migrate_legacy_runtime_state(
    config: &Config,
) -> Result<(), RuntimeStateMigrationError> {
    ensure_state_directories(config)?;

    let mut migrated = Vec::new();
    let mut removed_duplicate_legacy = Vec::new();
    let mut quarantined = Vec::new();

    for item in legacy_migrations(config) {
        migrate_legacy_item(
            &config.legacy_runtime_file_path(item.legacy_name),
            &item.destination,
            item.kind,
            &mut migrated,
            &mut removed_duplicate_legacy,
        )?;
    }

    migrate_legacy_token_caches(config, &mut migrated, &mut removed_duplicate_legacy)?;
    quarantine_legacy_backups(config, &mut quarantined)?;

    if !migrated.is_empty() {
        info!(
            "Migrated legacy LibreQoS runtime artifacts into state directory: {}",
            migrated.join(", ")
        );
    }
    if !removed_duplicate_legacy.is_empty() {
        info!(
            "Removed duplicate legacy runtime artifacts from lqos_directory after confirming canonical state copies exist: {}",
            removed_duplicate_legacy.join(", ")
        );
    }
    if !quarantined.is_empty() {
        info!(
            "Quarantined obsolete LibreQoS legacy artifacts: {}",
            quarantined.join(", ")
        );
    }

    Ok(())
}

fn ensure_state_directories(config: &Config) -> Result<(), RuntimeStateMigrationError> {
    let directories = [
        config.topology_state_file_path(".keep"),
        config.shaping_state_file_path(".keep"),
        config.stats_state_file_path(".keep"),
        config.cache_state_file_path(".keep"),
        config.debug_state_file_path(".keep"),
        config.quarantine_state_directory_path().join(".keep"),
        config.legacy_quarantine_directory_path().join(".keep"),
    ];
    for marker in directories {
        let path = marker
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or(marker.clone());
        fs::create_dir_all(&path).map_err(|source| {
            RuntimeStateMigrationError::CreateDirectory {
                path: path.display().to_string(),
                source,
            }
        })?;
    }
    Ok(())
}

fn legacy_migrations(config: &Config) -> Vec<LegacyMigration<'static>> {
    vec![
        LegacyMigration {
            legacy_name: CIRCUIT_ANCHORS_FILENAME,
            destination: config.topology_state_file_path(CIRCUIT_ANCHORS_FILENAME),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: CIRCUIT_ETHERNET_METADATA_FILENAME,
            destination: config.topology_state_file_path(CIRCUIT_ETHERNET_METADATA_FILENAME),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: TOPOLOGY_CANONICAL_STATE_FILENAME,
            destination: config.topology_state_file_path(TOPOLOGY_CANONICAL_STATE_FILENAME),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: TOPOLOGY_EDITOR_STATE_FILENAME,
            destination: config.topology_state_file_path(TOPOLOGY_EDITOR_STATE_FILENAME),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: TOPOLOGY_PARENT_CANDIDATES_FILENAME,
            destination: config.topology_state_file_path(TOPOLOGY_PARENT_CANDIDATES_FILENAME),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: TOPOLOGY_ATTACHMENT_HEALTH_STATE_FILENAME,
            destination: config.topology_state_file_path(TOPOLOGY_ATTACHMENT_HEALTH_STATE_FILENAME),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: TOPOLOGY_EFFECTIVE_STATE_FILENAME,
            destination: config.topology_state_file_path(TOPOLOGY_EFFECTIVE_STATE_FILENAME),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: TOPOLOGY_EFFECTIVE_NETWORK_FILENAME,
            destination: config.topology_state_file_path(TOPOLOGY_EFFECTIVE_NETWORK_FILENAME),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: TOPOLOGY_IMPORT_FILENAME,
            destination: config.topology_state_file_path(TOPOLOGY_IMPORT_FILENAME),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: TOPOLOGY_RUNTIME_STATUS_FILENAME,
            destination: config.topology_state_file_path(TOPOLOGY_RUNTIME_STATUS_FILENAME),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: TOPOLOGY_SHAPING_INPUTS_FILENAME,
            destination: config.shaping_state_file_path(TOPOLOGY_SHAPING_INPUTS_FILENAME),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: TOPOLOGY_COMPILED_SHAPING_FILENAME,
            destination: config.shaping_state_file_path(TOPOLOGY_COMPILED_SHAPING_FILENAME),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: "planner_state.json",
            destination: config.shaping_state_file_path("planner_state.json"),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: "queuingStructure.json",
            destination: config.shaping_state_file_path("queuingStructure.json"),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: "bakery_qdisc_handles.json",
            destination: config.shaping_state_file_path("bakery_qdisc_handles.json"),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: "lastGoodConfig.json",
            destination: config.shaping_state_file_path("lastGoodConfig.json"),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: "lastGoodConfig.csv",
            destination: config.shaping_state_file_path("lastGoodConfig.csv"),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: "ShapedDevices.lastLoaded.csv",
            destination: config.shaping_state_file_path("ShapedDevices.lastLoaded.csv"),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: "statsByCircuit.json",
            destination: config.stats_state_file_path("statsByCircuit.json"),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: "statsByParentNode.json",
            destination: config.stats_state_file_path("statsByParentNode.json"),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: "lastRun.txt",
            destination: config.stats_state_file_path("lastRun.txt"),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: "cpu_topology_cache.json",
            destination: config.cache_state_file_path("cpu_topology_cache.json"),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: "linux_tc.txt",
            destination: config.debug_state_file_path("linux_tc.txt"),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: "linux_tc_rust.txt",
            destination: config.debug_state_file_path("linux_tc_rust.txt"),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: "network.insight.debug.json",
            destination: config.debug_state_file_path("network.insight.debug.json"),
            kind: LegacyArtifactKind::File,
        },
        LegacyMigration {
            legacy_name: ".topology_stale",
            destination: config
                .quarantine_state_directory_path()
                .join(".topology_stale"),
            kind: LegacyArtifactKind::Directory,
        },
    ]
}

fn migrate_legacy_item(
    source: &Path,
    destination: &Path,
    kind: LegacyArtifactKind,
    migrated: &mut Vec<String>,
    removed_duplicate_legacy: &mut Vec<String>,
) -> Result<(), RuntimeStateMigrationError> {
    if !source.exists() {
        return Ok(());
    }

    match kind {
        LegacyArtifactKind::File if !source.is_file() => return Ok(()),
        LegacyArtifactKind::Directory if !source.is_dir() => return Ok(()),
        _ => {}
    }

    if destination.exists() {
        remove_path(source)?;
        removed_duplicate_legacy.push(source.display().to_string());
        return Ok(());
    }

    move_path(source, destination)?;
    migrated.push(format!("{} -> {}", source.display(), destination.display()));
    Ok(())
}

fn migrate_legacy_token_caches(
    config: &Config,
    migrated: &mut Vec<String>,
    removed_duplicate_legacy: &mut Vec<String>,
) -> Result<(), RuntimeStateMigrationError> {
    let legacy_root = Path::new(&config.lqos_directory);
    let entries =
        fs::read_dir(legacy_root).map_err(|source| RuntimeStateMigrationError::ReadDirectory {
            path: legacy_root.display().to_string(),
            source,
        })?;

    for entry in entries {
        let entry = entry.map_err(|source| RuntimeStateMigrationError::ReadDirectory {
            path: legacy_root.display().to_string(),
            source,
        })?;
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        if !name.starts_with(".visp_token_cache_") || !name.ends_with(".json") {
            continue;
        }
        let source = entry.path();
        let destination = config.cache_state_file_path(&name);
        migrate_legacy_item(
            &source,
            &destination,
            LegacyArtifactKind::File,
            migrated,
            removed_duplicate_legacy,
        )?;
    }

    Ok(())
}

fn quarantine_legacy_backups(
    config: &Config,
    quarantined: &mut Vec<String>,
) -> Result<(), RuntimeStateMigrationError> {
    let legacy_root = Path::new(&config.lqos_directory);
    let entries =
        fs::read_dir(legacy_root).map_err(|source| RuntimeStateMigrationError::ReadDirectory {
            path: legacy_root.display().to_string(),
            source,
        })?;

    for entry in entries {
        let entry = entry.map_err(|source| RuntimeStateMigrationError::ReadDirectory {
            path: legacy_root.display().to_string(),
            source,
        })?;
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        if !is_obsolete_legacy_backup(&name) {
            continue;
        }
        let source = entry.path();
        let destination = unique_quarantine_destination(config, &name);
        move_path(&source, &destination)?;
        quarantined.push(format!("{} -> {}", source.display(), destination.display()));
    }

    Ok(())
}

fn is_obsolete_legacy_backup(name: &str) -> bool {
    (name.starts_with("network.json.treeguard-polluted.") && name.ends_with(".bak"))
        || name.starts_with("network.json.pre_")
}

fn unique_quarantine_destination(config: &Config, name: &str) -> PathBuf {
    let base = config.legacy_quarantine_directory_path();
    let candidate = base.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let mut attempt = 0usize;
    loop {
        let candidate = base.join(format!("{name}.{stamp}.{attempt}"));
        if !candidate.exists() {
            return candidate;
        }
        attempt += 1;
    }
}

fn move_path(source: &Path, destination: &Path) -> Result<(), RuntimeStateMigrationError> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|source_err| {
            RuntimeStateMigrationError::CreateDirectory {
                path: parent.display().to_string(),
                source: source_err,
            }
        })?;
    }

    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(rename_err) => {
            if source.is_dir() {
                copy_dir_all(source, destination)?;
                fs::remove_dir_all(source).map_err(|source_err| {
                    RuntimeStateMigrationError::RemoveDirectory {
                        path: source.display().to_string(),
                        source: source_err,
                    }
                })?;
            } else {
                fs::copy(source, destination).map_err(|source_err| {
                    RuntimeStateMigrationError::Move {
                        from: source.display().to_string(),
                        to: destination.display().to_string(),
                        source: source_err,
                    }
                })?;
                fs::remove_file(source).map_err(|source_err| {
                    RuntimeStateMigrationError::RemoveFile {
                        path: source.display().to_string(),
                        source: source_err,
                    }
                })?;
            }
            if destination.exists() {
                Ok(())
            } else {
                Err(RuntimeStateMigrationError::Move {
                    from: source.display().to_string(),
                    to: destination.display().to_string(),
                    source: rename_err,
                })
            }
        }
    }
}

fn remove_path(path: &Path) -> Result<(), RuntimeStateMigrationError> {
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|source| RuntimeStateMigrationError::RemoveDirectory {
            path: path.display().to_string(),
            source,
        })?;
    } else {
        fs::remove_file(path).map_err(|source| RuntimeStateMigrationError::RemoveFile {
            path: path.display().to_string(),
            source,
        })?;
    }
    Ok(())
}

fn copy_dir_all(source: &Path, destination: &Path) -> Result<(), RuntimeStateMigrationError> {
    fs::create_dir_all(destination).map_err(|source_err| {
        RuntimeStateMigrationError::CreateDirectory {
            path: destination.display().to_string(),
            source: source_err,
        }
    })?;
    let entries =
        fs::read_dir(source).map_err(|source_err| RuntimeStateMigrationError::ReadDirectory {
            path: source.display().to_string(),
            source: source_err,
        })?;
    for entry in entries {
        let entry = entry.map_err(|source_err| RuntimeStateMigrationError::ReadDirectory {
            path: source.display().to_string(),
            source: source_err,
        })?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_all(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path).map_err(|source_err| {
                RuntimeStateMigrationError::Move {
                    from: source_path.display().to_string(),
                    to: destination_path.display().to_string(),
                    source: source_err,
                }
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::migrate_legacy_runtime_state;
    use crate::Config;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("libreqos-state-migration-{label}-{stamp}"))
    }

    fn test_config(root: &PathBuf) -> Config {
        Config {
            lqos_directory: root.join("src").display().to_string(),
            state_directory: Some(root.join("state").display().to_string()),
            ..Config::default()
        }
    }

    #[test]
    fn migrates_legacy_runtime_files_into_state_directory() {
        let root = temp_path("migrate-files");
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("lastGoodConfig.csv"), "legacy-csv\n").unwrap();
        fs::write(src.join("lastRun.txt"), "legacy-run\n").unwrap();
        fs::write(src.join("cpu_topology_cache.json"), "{}\n").unwrap();

        let config = test_config(&root);
        migrate_legacy_runtime_state(&config).unwrap();

        assert_eq!(
            fs::read_to_string(root.join("state/shaping/lastGoodConfig.csv")).unwrap(),
            "legacy-csv\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("state/stats/lastRun.txt")).unwrap(),
            "legacy-run\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("state/cache/cpu_topology_cache.json")).unwrap(),
            "{}\n"
        );
        assert!(!src.join("lastGoodConfig.csv").exists());
        assert!(!src.join("lastRun.txt").exists());
        assert!(!src.join("cpu_topology_cache.json").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn removes_duplicate_legacy_file_when_new_state_file_exists() {
        let root = temp_path("duplicate");
        let src = root.join("src");
        let state = root.join("state/shaping");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&state).unwrap();
        fs::write(src.join("lastGoodConfig.csv"), "legacy\n").unwrap();
        fs::write(state.join("lastGoodConfig.csv"), "state\n").unwrap();

        let config = test_config(&root);
        migrate_legacy_runtime_state(&config).unwrap();

        assert!(!src.join("lastGoodConfig.csv").exists());
        assert_eq!(
            fs::read_to_string(state.join("lastGoodConfig.csv")).unwrap(),
            "state\n"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn quarantines_obsolete_backups_and_moves_stale_directory() {
        let root = temp_path("quarantine");
        let src = root.join("src");
        fs::create_dir_all(src.join(".topology_stale")).unwrap();
        fs::write(
            src.join("network.json.treeguard-polluted.20260322T014345Z.bak"),
            "polluted\n",
        )
        .unwrap();
        fs::write(src.join("network.json.pre_50k_generator.bak"), "backup\n").unwrap();
        fs::write(src.join(".topology_stale/stale.json"), "{}\n").unwrap();

        let config = test_config(&root);
        migrate_legacy_runtime_state(&config).unwrap();

        assert!(
            root.join(
                "state/quarantine/legacy/network.json.treeguard-polluted.20260322T014345Z.bak"
            )
            .exists()
        );
        assert!(
            root.join("state/quarantine/legacy/network.json.pre_50k_generator.bak")
                .exists()
        );
        assert!(
            root.join("state/quarantine/.topology_stale/stale.json")
                .exists()
        );
        assert!(
            !src.join("network.json.treeguard-polluted.20260322T014345Z.bak")
                .exists()
        );
        assert!(!src.join("network.json.pre_50k_generator.bak").exists());
        assert!(!src.join(".topology_stale").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_visp_token_cache_files() {
        let root = temp_path("visp-cache");
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join(".visp_token_cache_deadbeef.json"),
            "{\"token\":1}\n",
        )
        .unwrap();

        let config = test_config(&root);
        migrate_legacy_runtime_state(&config).unwrap();

        assert_eq!(
            fs::read_to_string(root.join("state/cache/.visp_token_cache_deadbeef.json")).unwrap(),
            "{\"token\":1}\n"
        );
        assert!(!src.join(".visp_token_cache_deadbeef.json").exists());

        let _ = fs::remove_dir_all(root);
    }
}
