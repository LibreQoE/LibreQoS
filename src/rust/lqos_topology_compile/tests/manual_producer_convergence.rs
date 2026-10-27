//! Phase C S4 close: the convergence-of-worlds oracle.
//!
//! The manual-DEFAULT config (no adapter enabled) materializes its pair through
//! the SAME producer the integration world uses, and the persisted artifacts
//! carry the same shape: a `NativeIntegration` canonical whose ingress identity
//! equals the import's, so the runtime custody chain validates both worlds
//! identically. `docs/unified_simplification_plan.md` "THE MODE FLAG DIES".

use lqos_config::{Config, TopologyCanonicalIngressKind, TopologyCanonicalStateFile};
use lqos_topology_compile::{
    ImportedTopologyBundle, TopologyCompiledShapingFile, TopologyImportFile,
    materialize_manual_topology,
};
use std::path::PathBuf;

fn unique_dir(label: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after the epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("libreqos-s4-{label}-{stamp}"))
}

fn manual_config(dir: &PathBuf) -> Config {
    Config {
        lqos_directory: dir.display().to_string(),
        state_directory: None,
        ..Config::default()
    }
}

fn write_manual_pair(dir: &PathBuf) {
    std::fs::write(
        dir.join("network.json"),
        r#"{
            "Site One": {
                "children": {
                    "AP One": {
                        "children": {},
                        "id": "ap1",
                        "name": "AP One",
                        "type": "AP",
                        "downloadBandwidthMbps": 100,
                        "uploadBandwidthMbps": 100
                    }
                },
                "id": "s1",
                "name": "Site One",
                "type": "Site",
                "downloadBandwidthMbps": 1000,
                "uploadBandwidthMbps": 1000
            }
        }"#,
    )
    .expect("network.json should write");
    std::fs::write(
        dir.join("ShapedDevices.csv"),
        "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n\"c1\",\"Circuit One\",\"d1\",\"d1\",\"AP One\",\"ap1\",\"\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",10,5,100,50,\"\"\n",
    )
    .expect("ShapedDevices.csv should write");
}

#[test]
fn manual_default_world_materializes_through_the_one_producer() {
    let dir = unique_dir("manual-producer");
    std::fs::create_dir_all(&dir).expect("temp dir should exist");
    write_manual_pair(&dir);
    let config = manual_config(&dir);

    let materialized = materialize_manual_topology(&config).expect("manual producer should run");
    assert!(materialized, "a manual pair must materialize the import");

    let import = TopologyImportFile::load(&config)
        .expect("import should load")
        .expect("import should exist");
    let compiled = TopologyCompiledShapingFile::load(&config)
        .expect("compiled shaping should load")
        .expect("compiled shaping should exist");
    let canonical = TopologyCanonicalStateFile::load(&config).expect("canonical should load");

    // The custody chain: a NativeIntegration canonical whose identity is the
    // import's, exactly as the integration producers persist it.
    assert_eq!(
        canonical.ingress_kind,
        TopologyCanonicalIngressKind::NativeIntegration
    );
    assert_eq!(canonical.ingress_identity, import.ingress_identity);
    assert_eq!(import.ingress_identity, compiled.ingress_identity);
    assert!(!canonical.nodes.is_empty());

    // The pair's user data survives the producer unchanged.
    let artifacts = compiled.shaping_artifacts();
    assert_eq!(artifacts.shaped_devices.devices.len(), 1);
    let device = &artifacts.shaped_devices.devices[0];
    assert_eq!(device.circuit_id, "c1");
    assert_eq!(device.parent_node, "AP One");
    assert_eq!(device.download_max_mbps, 100.0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_producer_and_the_direct_legacy_door_agree() {
    // The materialized world equals the direct `from_legacy_artifacts` +
    // `from_imported_manual` projection, so the persisted import is exactly
    // the door's output (no second producer, no projection drift).
    let dir = unique_dir("producer-agrees");
    std::fs::create_dir_all(&dir).expect("temp dir should exist");
    write_manual_pair(&dir);
    let config = manual_config(&dir);

    assert!(materialize_manual_topology(&config).expect("manual producer should run"));
    let compiled = TopologyCompiledShapingFile::load(&config)
        .expect("compiled shaping should load")
        .expect("compiled shaping should exist");
    let materialized = compiled.shaping_artifacts();

    let direct_bundle = ImportedTopologyBundle::from_legacy_artifacts(&config, "legacy/manual")
        .expect("bundle should load");
    let direct = TopologyCompiledShapingFile::from_imported_manual(&direct_bundle, "full", None)
        .shaping_artifacts();

    assert_eq!(
        materialized.shaped_devices.devices, direct.shaped_devices.devices,
        "the producer persists the legacy door's shaped devices unchanged"
    );
    assert_eq!(
        materialized.circuit_anchors, direct.circuit_anchors,
        "the producer persists the legacy door's anchors unchanged"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
