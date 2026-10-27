use crate::{
    CIRCUIT_ANCHORS_FILENAME, CIRCUIT_ETHERNET_METADATA_FILENAME, Config,
    TOPOLOGY_ATTACHMENT_HEALTH_STATE_FILENAME, TOPOLOGY_CANONICAL_STATE_FILENAME,
    TOPOLOGY_COMPILED_SHAPING_FILENAME,
    TOPOLOGY_EFFECTIVE_NETWORK_FILENAME, TOPOLOGY_EFFECTIVE_STATE_FILENAME,
    TOPOLOGY_IMPORT_FILENAME, TOPOLOGY_PARENT_CANDIDATES_FILENAME,
    TOPOLOGY_RUNTIME_STATUS_FILENAME, TOPOLOGY_SHAPING_INPUTS_FILENAME,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tracing::{info, warn};

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

/// Phase D: the explicit repair entry. Runs every writey leg that `load_config`
/// no longer runs: legacy runtime-state migration, the stale-topology
/// quarantine, and the legacy credentials migration. Called by the write path
/// (lqosd startup, the scheduler's idempotent repair-before-materialize) —
/// NEVER by a read/verify path.
///
/// Side effects: moves/quarantines runtime files as described above.
pub fn repair_runtime_state(config: &Config) -> Result<(), String> {
    migrate_legacy_runtime_state(config).map_err(|e| e.to_string())?;
    crate::migrate_legacy_mikrotik_ipv6_credentials(config).map_err(|e| e.to_string())?;
    crate::topology_canonical_state::repair_stale_topology_state(config)
        .map_err(|e| e.to_string())?;
    Ok(())
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
    quarantine_stale_manual_source_files(config, &mut quarantined)?;
    // AVAIL 0(d) (CARD §3): generated state that fails CUSTODY (torn /
    // unparseable) is auto-quarantined so the runtime re-mints from a clean
    // slate instead of deferring forever. OPERATOR INPUTS are never touched.
    quarantine_uncustodied_generated_state(config, &mut quarantined)?;

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

fn quarantine_stale_manual_source_files(
    config: &Config,
    quarantined: &mut Vec<String>,
) -> Result<(), RuntimeStateMigrationError> {
    if !integration_topology_import_is_ready(config) {
        return Ok(());
    }

    for name in ["network.json", "ShapedDevices.csv"] {
        let source = config.legacy_runtime_file_path(name);
        if !source.is_file() {
            continue;
        }
        let destination = unique_quarantine_destination(config, name);
        move_path(&source, &destination)?;
        quarantined.push(format!(
            "{} -> {} (integration ingress active)",
            source.display(),
            destination.display()
        ));
    }

    Ok(())
}

/// AVAIL 0(d) (CARD `v3_availability_hardening_card` §3): auto-quarantine
/// GENERATED artifacts that fail CUSTODY — a torn/unparseable JSON body. This
/// is the stale-generated-state class an upgrade can leave behind (the
/// handshake then defers forever until an operator hand-moves the file). The
/// custody set is the GENERATED topology/shaping artifacts ONLY — operator
/// inputs (overrides, manual network.json/ShapedDevices) live under
/// `lqos_directory` and are never in this set.
fn quarantine_uncustodied_generated_state(
    config: &Config,
    quarantined: &mut Vec<String>,
) -> Result<(), RuntimeStateMigrationError> {
    // (path, expected schema_version: `Some(v)` = the TYPED bar — a JSON
    // OBJECT carrying `schema_version == v`; `None` = a version-less generated
    // document, where an OBJECT is the bar). For the VERSIONED docs this
    // rejects `{}` / `[]` / a bare number / a wrong-version body — not merely
    // a torn parse; the two VERSION-LESS docs (their producers emit no
    // `schema_version`) admit any OBJECT, including `{}`.
    let generated: [(PathBuf, Option<u32>); 7] = [
        (config.topology_state_read_path(TOPOLOGY_CANONICAL_STATE_FILENAME), Some(1)),
        (config.shaping_state_read_path(TOPOLOGY_COMPILED_SHAPING_FILENAME), None),
        (config.topology_state_read_path(TOPOLOGY_ATTACHMENT_HEALTH_STATE_FILENAME), Some(1)),
        (config.topology_state_read_path(TOPOLOGY_EFFECTIVE_NETWORK_FILENAME), None),
        (config.topology_state_read_path(TOPOLOGY_EFFECTIVE_STATE_FILENAME), Some(1)),
        (config.topology_state_read_path(TOPOLOGY_RUNTIME_STATUS_FILENAME), Some(1)),
        (config.shaping_state_read_path(TOPOLOGY_SHAPING_INPUTS_FILENAME), Some(1)),
    ];

    for (path, expected_schema) in generated {
        if !path.is_file() {
            continue;
        }
        // Custody = the body parses AND satisfies the typed bar. An EMPTY file
        // is a torn write and is quarantined too.
        let custodied = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|value| {
                let object = value.as_object()?;
                match expected_schema {
                    Some(expected) => Some(
                        object.get("schema_version").and_then(|v| v.as_u64())
                            == Some(u64::from(expected)),
                    ),
                    None => Some(true),
                }
            })
            .unwrap_or(false);
        if custodied {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let destination = unique_quarantine_destination(config, name);
        // AVAILABILITY (CARD §0/§7): quarantining is a HEALING action — an
        // inability to heal (permissions / disk / cross-device) must NEVER
        // abort the control plane. WARN + continue; the runtime re-mints /
        // defers exactly as it would have without 0(d).
        if let Err(error) = move_path(&path, &destination) {
            warn!(
                "0(d): could not quarantine uncustodied generated state {} ({error}); \
                 continuing — the runtime will re-mint or defer as before",
                path.display()
            );
            continue;
        }
        quarantined.push(format!(
            "{} -> {} (failed custody; will re-mint)",
            path.display(),
            destination.display()
        ));
    }

    Ok(())
}

/// Phase C S4: the import is integration-sourced (and may quarantine the
/// superseded manual pair) unless the manual producer itself wrote it. The
/// integration-ingress mode flag no longer exists; the materialized import's
/// `source` is the discriminator (`legacy/*` = the manual producer).
fn topology_import_is_integration_sourced(config: &Config) -> bool {
    let path = config.topology_state_file_path(TOPOLOGY_IMPORT_FILENAME);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    value
        .get("source")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|source| !source.trim().starts_with("legacy/"))
}

fn integration_topology_import_is_ready(config: &Config) -> bool {
    topology_import_is_integration_sourced(config)
        && crate::topology_import_has_shaped_devices(config)
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
    use crate::{
        Config, TOPOLOGY_ATTACHMENT_HEALTH_STATE_FILENAME, TOPOLOGY_EFFECTIVE_STATE_FILENAME,
        TOPOLOGY_IMPORT_FILENAME, TOPOLOGY_RUNTIME_STATUS_FILENAME,
        TOPOLOGY_SHAPING_INPUTS_FILENAME,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after UNIX_EPOCH")
            .as_nanos();
        std::env::temp_dir().join(format!("libreqos-state-migration-{label}-{stamp}"))
    }

    fn test_config(root: &Path) -> Config {
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
        fs::create_dir_all(&src).expect("test should create src dir");
        fs::write(src.join("lastGoodConfig.csv"), "legacy-csv\n")
            .expect("test should write legacy shaping csv");
        fs::write(src.join("lastRun.txt"), "legacy-run\n")
            .expect("test should write legacy stats file");
        fs::write(src.join("cpu_topology_cache.json"), "{}\n")
            .expect("test should write legacy cache file");

        let config = test_config(&root);
        migrate_legacy_runtime_state(&config).expect("legacy runtime files should migrate");

        assert_eq!(
            fs::read_to_string(root.join("state/shaping/lastGoodConfig.csv"))
                .expect("migrated shaping file should be readable"),
            "legacy-csv\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("state/stats/lastRun.txt"))
                .expect("migrated stats file should be readable"),
            "legacy-run\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("state/cache/cpu_topology_cache.json"))
                .expect("migrated cache file should be readable"),
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
        fs::create_dir_all(&src).expect("test should create src dir");
        fs::create_dir_all(&state).expect("test should create state dir");
        fs::write(src.join("lastGoodConfig.csv"), "legacy\n")
            .expect("test should write legacy file");
        fs::write(state.join("lastGoodConfig.csv"), "state\n")
            .expect("test should write destination file");

        let config = test_config(&root);
        migrate_legacy_runtime_state(&config)
            .expect("duplicate legacy file should be removed safely");

        assert!(!src.join("lastGoodConfig.csv").exists());
        assert_eq!(
            fs::read_to_string(state.join("lastGoodConfig.csv"))
                .expect("state file should still be readable"),
            "state\n"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn quarantines_obsolete_backups_and_moves_stale_directory() {
        let root = temp_path("quarantine");
        let src = root.join("src");
        fs::create_dir_all(src.join(".topology_stale"))
            .expect("test should create stale topology dir");
        fs::write(
            src.join("network.json.treeguard-polluted.20260322T014345Z.bak"),
            "polluted\n",
        )
        .expect("test should write polluted backup");
        fs::write(src.join("network.json.pre_50k_generator.bak"), "backup\n")
            .expect("test should write backup file");
        fs::write(src.join(".topology_stale/stale.json"), "{}\n")
            .expect("test should write stale topology file");

        let config = test_config(&root);
        migrate_legacy_runtime_state(&config).expect("obsolete backups should be quarantined");

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

    /// AVAIL 0(d) (CARD §3): a GENERATED artifact that fails custody (torn /
    /// unparseable) is auto-quarantined; a custodied one stays; an OPERATOR
    /// input under `lqos_directory` is NEVER touched.
    #[test]
    fn quarantines_uncustodied_generated_state() {
        let root = temp_path("custody");
        let config = test_config(&root);

        let shaping = config.shaping_state_read_path(TOPOLOGY_SHAPING_INPUTS_FILENAME);
        fs::create_dir_all(shaping.parent().expect("shaping parent"))
            .expect("test should create shaping state dir");
        fs::write(&shaping, "{ torn").expect("test should write torn shaping inputs");

        let runtime = config.topology_state_read_path(TOPOLOGY_RUNTIME_STATUS_FILENAME);
        fs::create_dir_all(runtime.parent().expect("runtime parent"))
            .expect("test should create topology state dir");
        fs::write(&runtime, "{\"schema_version\":1}").expect("test should write a custodied file");

 // TYPED-bar failures (the deriver ITEM 2): a parseable but structurally-wrong
        // body for a versioned generated doc is quarantined.
        let not_object = config.topology_state_read_path(TOPOLOGY_EFFECTIVE_STATE_FILENAME);
        fs::write(&not_object, "[]").expect("test should write a non-object body");
        let missing_version =
            config.topology_state_read_path(TOPOLOGY_ATTACHMENT_HEALTH_STATE_FILENAME);
        fs::write(&missing_version, "{}").expect("test should write a version-less object");

        let src = root.join("src");
        fs::create_dir_all(&src).expect("test should create src dir");
        fs::write(src.join("lqos_overrides.json"), "{ operator }")
            .expect("test should write an operator input");

        migrate_legacy_runtime_state(&config).expect("custody quarantine should run");

        assert!(
            !shaping.exists(),
            "the torn generated artifact is quarantined"
        );
        assert!(
            runtime.exists(),
            "a custodied generated artifact stays in place"
        );
        assert!(
            !not_object.exists(),
            "a non-object body fails the typed bar and is quarantined"
        );
        assert!(
            !missing_version.exists(),
            "a version-less object fails the typed bar and is quarantined"
        );
        assert!(
            src.join("lqos_overrides.json").exists(),
            "operator inputs are NEVER touched"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_visp_token_cache_files() {
        let root = temp_path("visp-cache");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("test should create src dir");
        fs::write(
            src.join(".visp_token_cache_deadbeef.json"),
            "{\"token\":1}\n",
        )
        .expect("test should write legacy visp cache");

        let config = test_config(&root);
        migrate_legacy_runtime_state(&config).expect("visp cache should migrate");

        assert_eq!(
            fs::read_to_string(root.join("state/cache/.visp_token_cache_deadbeef.json"))
                .expect("migrated visp cache should be readable"),
            "{\"token\":1}\n"
        );
        assert!(!src.join(".visp_token_cache_deadbeef.json").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn quarantines_stale_manual_source_files_when_topology_import_is_ready() {
        let root = temp_path("integration-source-quarantine");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("test should create src dir");
        fs::write(src.join("network.json"), "{\"Root\":{}}\n")
            .expect("test should write legacy network source file");
        fs::write(src.join("ShapedDevices.csv"), "Circuit ID,Device ID\n")
            .expect("test should write legacy shaped devices source file");

        let mut config = test_config(&root);
        config.uisp_integration.enable_uisp = true;
        let topology_state = config.resolved_state_directory().join("topology");
        fs::create_dir_all(&topology_state).expect("test should create topology state dir");
        fs::write(
            topology_state.join(TOPOLOGY_IMPORT_FILENAME),
            serde_json::json!({
                "schema_version": 1,
                "source": "test/import",
                "compile_mode": "full",
                "imported": {
                    "source": "test/import",
                    "compatibility_network_json": {},
                    "shaped_devices": [
                        {
                            "circuit_id": "circuit-1",
                            "device_id": "device-1"
                        }
                    ],
                    "circuit_anchors": { "anchors": [] },
                    "ethernet_advisories": []
                }
            })
            .to_string(),
        )
        .expect("test should write topology import artifact");

        migrate_legacy_runtime_state(&config).expect("stale manual source files should quarantine");

        assert!(!src.join("network.json").exists());
        assert!(!src.join("ShapedDevices.csv").exists());
        assert!(root.join("state/quarantine/legacy/network.json").exists());
        assert!(
            root.join("state/quarantine/legacy/ShapedDevices.csv")
                .exists()
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn keeps_manual_source_files_in_place_when_topology_import_is_missing() {
        let root = temp_path("manual-source-preserved");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("test should create src dir");
        fs::write(src.join("network.json"), "{\"Root\":{}}\n")
            .expect("test should write manual network source file");
        fs::write(src.join("ShapedDevices.csv"), "Circuit ID,Device ID\n")
            .expect("test should write manual shaped devices source file");

        let mut config = test_config(&root);
        config.uisp_integration.enable_uisp = true;
        migrate_legacy_runtime_state(&config)
            .expect("manual source files should remain until integration publishes data");

        assert!(src.join("network.json").exists());
        assert!(src.join("ShapedDevices.csv").exists());
        assert!(!root.join("state/quarantine/legacy/network.json").exists());
        assert!(
            !root
                .join("state/quarantine/legacy/ShapedDevices.csv")
                .exists()
        );

        let _ = fs::remove_dir_all(root);
    }
}
