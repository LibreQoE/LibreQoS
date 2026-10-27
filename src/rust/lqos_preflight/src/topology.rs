//! The card's §1.2 "will my network compile?" dry-run: read a v2 compat pair
//! (`network.json` + `ShapedDevices.csv`) in place and run it through the v3
//! topology compiler, daemon-free.
//!
//! The compiler loads its inputs from `Config::lqos_directory`, so the
//! preflight points a default `Config` at the pair's directory — no v3
//! install and no daemon are required.

use std::path::Path;

use lqos_config::{Config, topology_import_path};
use lqos_topology_compile::{
    ImportedTopologyBundle, TopologyCompiledShapingFile, TopologyCompileMode, TopologyImportFile,
    compile_topology,
};

use crate::circuits::count_shaped_devices;

/// The preflight's config: point BOTH the directory and the state directory at
/// the pair under test. The default config can carry a machine-global state
/// directory, which would make the discovered integration artifacts escape the
/// directory the operator asked to check.
fn preflight_config(dir: &Path) -> Config {
    Config {
        lqos_directory: dir.to_string_lossy().into_owned(),
        state_directory: Some(dir.join("state").to_string_lossy().into_owned()),
        ..Default::default()
    }
}

/// The verdict of the dry-run compile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompileReport {
    /// Fatal errors; a non-empty list is a BLOCK.
    pub errors: Vec<String>,
    /// Non-fatal advisories. The compiler exposes no warnings channel today,
    /// so this stays empty — kept so the surface is stable and honest.
    pub warnings: Vec<String>,
    /// Which source was discovered and validated (`integration-import` or
    /// `legacy-pair`). The integration door (UISP/compiled producers) never
    /// writes the legacy CSV pair, so the door must be discovered, not assumed.
    pub source: Option<String>,
}

impl CompileReport {
    /// True when the pair compiled with no errors.
    pub fn is_pass(&self) -> bool {
        self.errors.is_empty()
    }
}

/// The fresh-install / no-source advisory (never a BLOCK by itself).
pub const NO_TOPOLOGY_SOURCE: &str =
    "no topology source yet — configure an integration to write topology_import.json, \
     or provide the manual network.json + ShapedDevices.csv pair";

/// True when the directory holds ANY topology input: the integration import
/// (`state/topology/topology_import.json`) or either legacy pair file. A
/// fresh install has none — that is a WARN, never a BLOCK (`lqos-74t.37.10`).
pub fn has_topology_inputs(dir: &Path) -> bool {
    let config = preflight_config(dir);
    topology_import_path(&config).exists()
        || dir.join("network.json").exists()
        || dir.join("ShapedDevices.csv").exists()
}

/// Validate the topology in `dir` using the legacy default mode.
pub fn validate_pair(dir: &Path) -> CompileReport {
    validate_pair_with_mode(dir, TopologyCompileMode::Full)
}

/// Validate the topology in `dir` using an explicit compile mode, DISCOVERING
/// the source (§1): the integration/materialized import (the compiled door the
/// UISP class writes) first, then the legacy `network.json`+`ShapedDevices.csv`
/// pair (the manual class). The verdict names which door was validated.
pub fn validate_pair_with_mode(dir: &Path, mode: TopologyCompileMode) -> CompileReport {
    let config = preflight_config(dir);
    let import = topology_import_path(&config).exists();
    let network = dir.join("network.json").exists();
    let shaped = dir.join("ShapedDevices.csv").exists();

    // A fresh v3 install has NO topology source until an integration runs.
    // That is a WARNING, not a BLOCK (`lqos-74t.37.10`). A half legacy pair is
    // also a warning; only a PRESENT source that fails to compile blocks.
    if !import {
        if !network && !shaped {
            return CompileReport {
                warnings: vec![NO_TOPOLOGY_SOURCE.to_string()],
                ..Default::default()
            };
        }
        if !network || !shaped {
            return CompileReport {
                warnings: vec![
                    "incomplete manual topology: provide BOTH network.json and \
                     ShapedDevices.csv (or an integration topology_import.json); \
                     this is a warning until the source is complete"
                        .to_string(),
                ],
                ..Default::default()
            };
        }
    }

    // The integration door: the producers (UISP's Rust producer, the python
    // integrations) compile straight to `topology_import.json` + the
    // compiled-shaping artifact and never write the legacy CSV pair.
    if topology_import_path(&config).exists() {
        return match TopologyImportFile::load(&config) {
            Ok(Some(import)) => {
                let imported = import.into_imported_bundle();
                finish(imported, mode, "integration-import")
            }
            Ok(None) => {
                let mut report = CompileReport {
                    source: Some("integration-import".to_string()),
                    ..Default::default()
                };
                report
                    .errors
                    .push("the topology import disappeared between checks".to_string());
                report
            }
            Err(err) => {
                let mut report = CompileReport {
                    source: Some("integration-import".to_string()),
                    ..Default::default()
                };
                report
                    .errors
                    .push(format!("could not read the topology import: {err}"));
                report
            }
        };
    }

    // The manual/legacy door.
    match ImportedTopologyBundle::from_legacy_artifacts(&config, "preflight") {
        Ok(imported) => finish(imported, mode, "legacy-pair"),
        Err(err) => {
            let mut report = CompileReport {
                source: Some("legacy-pair".to_string()),
                ..Default::default()
            };
            report
                .errors
                .push(format!("could not read the v2 pair: {err}"));
            report
        }
    }
}

/// Count the circuits from the DISCOVERED source (§1): the compiled-shaping
/// artifact for the integration door (which never writes the legacy CSV pair),
/// else the legacy `ShapedDevices.csv` row count. The licensing warning's
/// reach is then the whole fleet, not only the manual class.
pub fn count_discovered_circuits(dir: &Path) -> Option<usize> {
    let config = preflight_config(dir);
    if topology_import_path(&config).exists() {
        return TopologyCompiledShapingFile::load(&config)
            .ok()
            .flatten()
            .map(|compiled| compiled.shaping_artifacts().shaped_devices.devices.len());
    }
    count_shaped_devices(&dir.join("ShapedDevices.csv")).ok()
}

fn finish(
    imported: ImportedTopologyBundle,
    mode: TopologyCompileMode,
    source: &str,
) -> CompileReport {
    let mut report = CompileReport {
        source: Some(source.to_string()),
        ..Default::default()
    };
    if let Err(err) = compile_topology(imported, mode) {
        report.errors.push(err.to_string());
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A unique scratch directory for one test (removed on drop).
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new() -> Self {
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "lqos_preflight_{}_{}",
                std::process::id(),
                id
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, name: &str, contents: &str) {
            std::fs::write(self.0.join(name), contents).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const CSV: &str = "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Sparking Min,Sparking Max,IPv4,IPv6,Download Min,Upload Min,Download Max,Upload Max\n\
        1,Circuit 1,1,Device 1,,, ,192.168.101.2,,25,5,10000,10000\n";

    #[test]
    fn an_empty_pair_compiles() {
        let dir = Scratch::new();
        dir.write("network.json", "{}");
        dir.write("ShapedDevices.csv", CSV);
        let report = validate_pair(&dir.0);
        assert!(report.is_pass(), "unexpected errors: {:?}", report.errors);
        assert!(report.warnings.is_empty());
        assert_eq!(report.source.as_deref(), Some("legacy-pair"));
    }

    #[test]
    fn discovers_the_integration_import_door() {
        let dir = Scratch::new();
        dir.write("network.json", "{}");
        dir.write("ShapedDevices.csv", CSV);
        let config = preflight_config(&dir.0);
        // A producer writes the import + compiled-shaping artifacts; the
        // legacy pair is NOT consulted.
        let imported =
            ImportedTopologyBundle::from_legacy_artifacts(&config, "test").unwrap();
        let compiled = compile_topology(
            ImportedTopologyBundle::from_legacy_artifacts(&config, "test").unwrap(),
            TopologyCompileMode::Full,
        )
        .unwrap();
        TopologyImportFile::from_imported_bundle(&imported, "full")
            .save(&config)
            .unwrap();
        TopologyCompiledShapingFile::from_compiled(&compiled, "full")
            .save(&config)
            .unwrap();
        // The integration door never writes the legacy pair; remove it so the
        // test proves an import-ONLY fresh v3 host validates.
        let _ = std::fs::remove_file(dir.0.join("network.json"));
        let _ = std::fs::remove_file(dir.0.join("ShapedDevices.csv"));
        let report = validate_pair(&dir.0);
        assert_eq!(report.source.as_deref(), Some("integration-import"));
        assert!(report.is_pass(), "unexpected errors: {:?}", report.errors);
        assert_eq!(count_discovered_circuits(&dir.0), Some(1));
    }

    #[test]
    fn a_missing_csv_warns_not_blocks() {
        let dir = Scratch::new();
        dir.write("network.json", "{}");
        let report = validate_pair(&dir.0);
        assert!(report.is_pass(), "a half pair is a WARN: {:?}", report.errors);
        assert!(
            report.warnings[0].contains("incomplete manual topology"),
            "unexpected warning: {:?}",
            report.warnings
        );
    }

    #[test]
    fn an_empty_directory_warns_not_blocks() {
        let dir = Scratch::new();
        let report = validate_pair(&dir.0);
        assert!(report.is_pass(), "a fresh install is a WARN: {:?}", report.errors);
        assert!(
            report.warnings[0].contains("no topology source yet"),
            "unexpected warning: {:?}",
            report.warnings
        );
    }
}
