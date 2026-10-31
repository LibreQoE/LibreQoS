//! `lqos-74t.150` PR2: the shared compile + write core.
//!
//! These tests pin the extraction contract: one success path for every
//! producer, the stale duplicate anchor removed by the core, a typed receipt
//! carrying origin/sha/sequence, and the documented mid-sequence failure
//! boundary (earlier files new, later files last-good, no sequence advance).

use lqos_config::{
    CIRCUIT_ANCHORS_FILENAME, Config, TOPOLOGY_CANONICAL_STATE_FILENAME,
    TOPOLOGY_COMPILED_SHAPING_FILENAME, TOPOLOGY_IMPORT_FILENAME,
    TOPOLOGY_PARENT_CANDIDATES_FILENAME,
};
use lqos_topology_compile::{
    TopologyImportFile, TopologyIngestError, TopologyWriteOrigin, compile_and_write_imported,
    parse_compile_mode, parse_libreqos_network, write_sequence,
};
use std::path::{Path, PathBuf};

static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn temp_config(tag: &str) -> Config {
    let dir = std::env::temp_dir().join(format!(
        "lqos-74t150-core-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    Config {
        lqos_directory: dir.display().to_string(),
        state_directory: Some(dir.join("state").display().to_string()),
        ..Config::default()
    }
}

fn fixture() -> lqos_topology_compile::ImportedTopologyBundle {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/libreqos_network.json"
    ))
    .expect("v2 fixture readable");
    parse_libreqos_network(&raw)
        .expect("v2 fixture validates")
        .into_imported_bundle("libreqos_network")
        .expect("v2 fixture projects")
}

fn output_paths(config: &Config) -> [(&'static str, PathBuf); 4] {
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
    ]
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn writes_all_outputs_with_a_typed_receipt_and_removes_the_stale_anchor() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let config = temp_config("success");
    let anchors = config.legacy_runtime_file_path(CIRCUIT_ANCHORS_FILENAME);
    std::fs::create_dir_all(anchors.parent().expect("anchor parent")).expect("anchor parent");
    std::fs::write(&anchors, b"stale").expect("stale anchor");

    let before = write_sequence();
    let receipt = compile_and_write_imported(
        &config,
        fixture(),
        "ap_only",
        TopologyWriteOrigin::Watcher,
        Some("abc123"),
    )
    .expect("first write");
    assert_eq!(receipt.origin, TopologyWriteOrigin::Watcher);
    assert_eq!(receipt.source_sha256.as_deref(), Some("abc123"));
    assert_eq!(receipt.write_sequence, before + 1);
    assert_eq!(
        receipt.files,
        [
            "topology_import.json",
            "topology_compiled_shaping.json",
            "topology_parent_candidates.json",
            "topology_canonical_state.json",
        ]
    );
    assert!(receipt.stale_anchor_removed);
    assert!(!anchors.exists(), "the core removes the stale anchor");
    assert_eq!(write_sequence(), receipt.write_sequence);

    let import = TopologyImportFile::load(&config)
        .expect("import load")
        .expect("import written");
    assert_eq!(import.writer.as_deref(), Some("watcher"));
    assert_eq!(import.source_sha256.as_deref(), Some("abc123"));
    for (file, path) in output_paths(&config) {
        assert!(path.is_file(), "{file} should be written at {}", path.display());
    }

    let second = compile_and_write_imported(
        &config,
        fixture(),
        "ap_only",
        TopologyWriteOrigin::Watcher,
        Some("abc123"),
    )
    .expect("second write");
    assert_eq!(second.write_sequence, before + 2);
    assert!(
        !second.stale_anchor_removed,
        "a missing stale anchor is a clean no-op"
    );
    assert!(!anchors.exists());
}

#[test]
fn mid_sequence_failure_leaves_earlier_file_new_and_later_files_last_good() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let config = temp_config("mid-sequence");
    let baseline = compile_and_write_imported(
        &config,
        fixture(),
        "ap_only",
        TopologyWriteOrigin::Api,
        Some("old-sha"),
    )
    .expect("baseline write");
    let sequence_after_baseline = baseline.write_sequence;

    let paths = output_paths(&config);
    let old_import = read(&paths[0].1);
    let old_parent_candidates = read(&paths[2].1);
    let old_canonical = read(&paths[3].1);

    // The second save target becomes a directory. `topology_import.json` is
    // therefore NEW (save order step 1), while the later parent-candidates and
    // canonical files stay LAST-GOOD (steps 3/4 never run). The failed target
    // itself is the injected directory.
    std::fs::remove_file(&paths[1].1).expect("remove compiled shaping");
    std::fs::create_dir(&paths[1].1).expect("inject directory target");

    let mut changed = fixture();
    changed.source = "libreqos_network/changed".to_string();
    let error = compile_and_write_imported(
        &config,
        changed,
        "ap_only",
        TopologyWriteOrigin::Watcher,
        Some("new-sha"),
    )
    .expect_err("the second save must fail");
    match error {
        TopologyIngestError::Write { file, .. } => {
            assert_eq!(file, "topology_compiled_shaping.json")
        }
        other => panic!("expected a Write error, got {other:?}"),
    }
    assert_eq!(
        write_sequence(),
        sequence_after_baseline,
        "a failed write must not advance the write sequence"
    );

    assert_ne!(
        read(&paths[0].1),
        old_import,
        "topology_import.json is NEW: it was saved before the failure"
    );
    let new_import = TopologyImportFile::load(&config)
        .expect("new import load")
        .expect("new import written");
    assert_eq!(new_import.source, "libreqos_network/changed");
    assert_eq!(new_import.writer.as_deref(), Some("watcher"));
    assert_eq!(new_import.source_sha256.as_deref(), Some("new-sha"));
    assert_eq!(
        read(&paths[2].1),
        old_parent_candidates,
        "topology_parent_candidates.json is LAST-GOOD"
    );
    assert_eq!(
        read(&paths[3].1),
        old_canonical,
        "topology_canonical_state.json is LAST-GOOD"
    );
    assert!(
        paths[1].1.is_dir(),
        "topology_compiled_shaping.json is the injected failure target"
    );
}

#[test]
fn unknown_compile_mode_is_the_typed_mode_variant() {
    assert!(matches!(
        parse_compile_mode("bogus"),
        Err(TopologyIngestError::Mode(mode)) if mode == "bogus"
    ));
    assert_eq!(
        parse_compile_mode("FULL2").expect("full2 alias"),
        lqos_topology_compile::TopologyCompileMode::Full2
    );
}
