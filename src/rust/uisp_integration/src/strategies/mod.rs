mod common;
mod full2;
mod legacy_bandwidth_overrides;
mod legacy_mikrotik;
mod legacy_parse;
mod legacy_routes_override;
mod legacy_uisp_fetch;

use crate::blackboard;
use crate::errors::UispIntegrationError;
use crate::ethernet_advisory::write_ethernet_advisories;
use crate::ip_ranges::IpRanges;
use lqos_bus::BlackboardSystem;
use lqos_config::Config;
use lqos_topology_compile::{TopologyCompileMode, TopologyWriteOrigin, compile_and_write_imported};
use std::sync::Arc;
use tracing::{error, info, warn};

fn compile_mode_for_strategy(strategy: &str) -> Result<TopologyCompileMode, UispIntegrationError> {
    match strategy {
        "flat" => Ok(TopologyCompileMode::Flat),
        "ap_only" => Ok(TopologyCompileMode::ApOnly),
        "ap_site" => Ok(TopologyCompileMode::ApSite),
        "full" => Ok(TopologyCompileMode::Full),
        "full2" => Ok(TopologyCompileMode::Full2),
        _ => Err(UispIntegrationError::UnknownIntegrationStrategy),
    }
}

/// Builds the network using the selected strategy.
pub async fn build_with_strategy(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    let strategy = config.resolved_topology_compile_mode_for_uisp().to_string();
    blackboard(BlackboardSystem::System, "UISP", strategy.as_str()).await;
    let mode = compile_mode_for_strategy(&strategy)?;
    info!("Strategy selected: {strategy}");
    let require_configured_root_site =
        matches!(mode, TopologyCompileMode::Full | TopologyCompileMode::Full2);
    let imported =
        full2::build_imported_full2_bundle(config.clone(), ip_ranges, require_configured_root_site)
            .await?;
    // The shared compile core persists the four topology artifacts but does
    // not own UISP's negotiated-Ethernet advisory file. Capture the advisories
    // before the bundle is consumed and write them after, preserving the
    // legacy order (artifacts, stale-anchor cleanup, advisories).
    let ethernet_advisories = imported.ethernet_advisories.clone();
    compile_and_write_imported(
        config.as_ref(),
        imported,
        &strategy,
        TopologyWriteOrigin::Uisp,
        None,
    )
    .map_err(|e| {
        error!("Unable to compile UISP topology for mode '{strategy}': {e:?}");
        UispIntegrationError::WriteNetJson
    })?;
    // The shared core removes the legacy root `circuit_anchors.json`. UISP
    // also removes the canonical `state/topology/circuit_anchors.json` it
    // historically owned (its anchors now live inside the compiled artifacts),
    // so keep that pre-5C cleanup, including its best-effort failure mode.
    let anchors_path = lqos_config::circuit_anchors_path(config.as_ref());
    if let Err(err) = std::fs::remove_file(&anchors_path)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        warn!(
            "Unable to remove stale duplicate circuit_anchors.json at {}: {err:?}",
            anchors_path.display()
        );
    }
    write_ethernet_advisories(config.as_ref(), &ethernet_advisories)?;
    Ok(())
}

/// Step 5C convergence oracle: the UISP producer must persist the same
/// artifacts through the shared [`compile_and_write_imported`] core as the
/// pre-5C private `write_compiled_outputs` path did. The only permitted
/// difference is the deliberate `writer: "uisp"` provenance stamp in
/// `topology_import.json` (asserted explicitly below).
#[cfg(test)]
mod convergence_tests {
    use super::*;
    use lqos_config::{
        CIRCUIT_ETHERNET_METADATA_FILENAME, Config, TOPOLOGY_CANONICAL_STATE_FILENAME,
        TOPOLOGY_COMPILED_SHAPING_FILENAME, TOPOLOGY_IMPORT_FILENAME,
        TOPOLOGY_PARENT_CANDIDATES_FILENAME,
    };
    use lqos_topology_compile::{
        ImportedTopologyBundle, TopologyCompiledShapingFile, TopologyImportFile, compile_topology,
        parse_libreqos_network,
    };
    use std::path::{Path, PathBuf};

    fn fixture() -> ImportedTopologyBundle {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../lqos_topology_compile/tests/fixtures/libreqos_network.json"
        ))
        .expect("fixture readable");
        parse_libreqos_network(&raw)
            .expect("fixture parses")
            .into_imported_bundle("libreqos_network")
            .expect("fixture projects")
    }

    fn temp_config(root: &Path, tag: &str) -> Config {
        let dir = root.join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        Config {
            lqos_directory: dir.display().to_string(),
            state_directory: Some(dir.join("state").display().to_string()),
            ..Config::default()
        }
    }

    fn seed_stale_anchors(config: &Config) {
        let canonical = lqos_config::circuit_anchors_path(config);
        std::fs::create_dir_all(canonical.parent().expect("anchor parent")).expect("anchor dir");
        std::fs::write(&canonical, b"stale canonical").expect("stale canonical anchor");
        let legacy = config.legacy_runtime_file_path(lqos_config::CIRCUIT_ANCHORS_FILENAME);
        std::fs::create_dir_all(legacy.parent().expect("legacy anchor parent"))
            .expect("legacy anchor dir");
        std::fs::write(&legacy, b"stale legacy").expect("stale legacy anchor");
    }

    /// Test-local reproduction of the pre-5C `write_compiled_outputs` block:
    /// build the import artifact, compile, save the four outputs in order,
    /// remove the stale anchor (best-effort), then write the Ethernet
    /// advisories. Kept as the oracle for the shared-core convergence.
    fn write_legacy_path(
        config: &Config,
        imported: ImportedTopologyBundle,
        strategy: &str,
    ) -> Result<(), UispIntegrationError> {
        let mode = compile_mode_for_strategy(strategy)?;
        let topology_import = TopologyImportFile::from_imported_bundle(&imported, strategy);
        let compiled =
            compile_topology(imported, mode).map_err(|_| UispIntegrationError::WriteNetJson)?;
        let compiled_shaping = TopologyCompiledShapingFile::from_compiled(&compiled, strategy);
        topology_import
            .save(config)
            .map_err(|_| UispIntegrationError::WriteNetJson)?;
        compiled_shaping
            .save(config)
            .map_err(|_| UispIntegrationError::WriteNetJson)?;
        compiled
            .parent_candidates
            .save(config)
            .map_err(|_| UispIntegrationError::WriteNetJson)?;
        compiled
            .canonical
            .save(config)
            .map_err(|_| UispIntegrationError::WriteNetJson)?;
        let anchors = lqos_config::circuit_anchors_path(config);
        if let Err(err) = std::fs::remove_file(&anchors)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            // The legacy path only warned here; it never failed the write.
        }
        write_ethernet_advisories(config, &compiled.ethernet_advisories)
    }

    /// The post-5C path: shared core for the four artifacts and the legacy
    /// root anchor cleanup, then UISP's own canonical-anchor cleanup and the
    /// UISP-owned advisory file.
    fn write_shared_path(
        config: &Config,
        imported: ImportedTopologyBundle,
        strategy: &str,
    ) -> Result<(), UispIntegrationError> {
        let ethernet_advisories = imported.ethernet_advisories.clone();
        compile_and_write_imported(config, imported, strategy, TopologyWriteOrigin::Uisp, None)
            .map_err(|_| UispIntegrationError::WriteNetJson)?;
        let canonical = lqos_config::circuit_anchors_path(config);
        if let Err(err) = std::fs::remove_file(&canonical)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            // Matches production: best-effort, never fails the write.
        }
        write_ethernet_advisories(config, &ethernet_advisories)
    }

    fn artifact_paths(config: &Config) -> [(&'static str, PathBuf); 5] {
        [
            (
                "topology_import.json",
                config.topology_state_file_path(TOPOLOGY_IMPORT_FILENAME),
            ),
            (
                "topology_compiled_shaping.json",
                config.shaping_state_file_path(TOPOLOGY_COMPILED_SHAPING_FILENAME),
            ),
            (
                "topology_parent_candidates.json",
                config.topology_state_file_path(TOPOLOGY_PARENT_CANDIDATES_FILENAME),
            ),
            (
                "topology_canonical_state.json",
                config.topology_state_file_path(TOPOLOGY_CANONICAL_STATE_FILENAME),
            ),
            (
                "circuit_ethernet_metadata.json",
                config.topology_state_file_path(CIRCUIT_ETHERNET_METADATA_FILENAME),
            ),
        ]
    }

    #[test]
    fn uisp_shared_core_matches_the_legacy_artifacts() {
        let root = std::env::temp_dir().join(format!(
            "lqos-step5c-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        // Run the whole fixture comparison twice: the artifacts must be
        // stable, not merely equal once.
        for iteration in 0..2 {
            for mode_name in ["flat", "ap_only", "ap_site", "full", "full2"] {
                let legacy_config = temp_config(&root, &format!("legacy-{iteration}-{mode_name}"));
                seed_stale_anchors(&legacy_config);
                write_legacy_path(&legacy_config, fixture(), mode_name)
                    .unwrap_or_else(|error| panic!("{mode_name} legacy: {error:?}"));
                assert!(
                    !lqos_config::circuit_anchors_path(&legacy_config).exists(),
                    "legacy path removes the canonical stale anchor"
                );
                assert!(
                    legacy_config
                        .legacy_runtime_file_path(lqos_config::CIRCUIT_ANCHORS_FILENAME)
                        .exists(),
                    "legacy path did not touch the legacy root duplicate"
                );

                let shared_config = temp_config(&root, &format!("shared-{iteration}-{mode_name}"));
                seed_stale_anchors(&shared_config);
                write_shared_path(&shared_config, fixture(), mode_name)
                    .unwrap_or_else(|error| panic!("{mode_name} shared: {error:?}"));
                assert!(
                    !lqos_config::circuit_anchors_path(&shared_config).exists(),
                    "shared path keeps UISP's canonical stale-anchor cleanup"
                );
                assert!(
                    !shared_config
                        .legacy_runtime_file_path(lqos_config::CIRCUIT_ANCHORS_FILENAME)
                        .exists(),
                    "the shared core removes the legacy root duplicate"
                );

                let legacy_paths = artifact_paths(&legacy_config);
                let shared_paths = artifact_paths(&shared_config);
                for ((name, legacy_path), (_, shared_path)) in
                    legacy_paths.iter().zip(shared_paths.iter())
                {
                    let legacy_bytes = std::fs::read(legacy_path)
                        .unwrap_or_else(|error| panic!("{mode_name} {name}: {error}"));
                    let shared_bytes = std::fs::read(shared_path)
                        .unwrap_or_else(|error| panic!("{mode_name} {name}: {error}"));
                    if *name == "topology_import.json" {
                        let mut legacy_json: serde_json::Value =
                            serde_json::from_slice(&legacy_bytes).expect("legacy import json");
                        let mut shared_json: serde_json::Value =
                            serde_json::from_slice(&shared_bytes).expect("shared import json");
                        assert!(
                            legacy_json.get("writer").is_none(),
                            "legacy UISP import had no writer stamp"
                        );
                        assert_eq!(
                            shared_json.get("writer").and_then(|value| value.as_str()),
                            Some("uisp"),
                            "the shared core must stamp the UISP provenance"
                        );
                        legacy_json
                            .as_object_mut()
                            .expect("import object")
                            .remove("writer");
                        shared_json
                            .as_object_mut()
                            .expect("import object")
                            .remove("writer");
                        assert_eq!(
                            legacy_json, shared_json,
                            "{mode_name}: topology_import.json differs beyond the writer stamp"
                        );
                    } else {
                        assert_eq!(
                            legacy_bytes, shared_bytes,
                            "{mode_name}: {name} differs between the legacy and shared paths"
                        );
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
