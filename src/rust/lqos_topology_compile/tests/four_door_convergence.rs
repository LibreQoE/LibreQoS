//! Phase C S3c: the four-door convergence oracle
//! (`docs/libreqos_network_format_card.md` §8(3.1)/(3.3)).
//!
//! Four integration doors project the same subscriber world onto an imported
//! bundle and then run the shared compiler:
//!
//! 1. the standard `libreqos_network.json` format (the new reader + mapper),
//! 2. the legacy on-disk pair (`network.json` + `ShapedDevices.csv`, via
//!    `ImportedTopologyBundle::from_legacy_artifacts`),
//! 3. a representative UISP `full2` bundle,
//! 4. a representative Python graph-payload bundle.
//!
//! (3.1) pins the representation: convergence is asserted ONLY within a pinned
//! `native_canonical` shape. A door feeding the compatibility-only shape
//! (no native canonical) is an ASSERTED divergence with an expected output —
//! here, the AP's rate falls back because the compatibility tree carries no
//! rate while the standard format does.
//!
//! Door-level PARSE paths live in their home crates (uisp `full2`, the
//! `lqos_python` bindings); this oracle asks whether the resulting bundles
//! converge, so the UISP/Python representatives are constructed in-test.

use lqos_config::{
    Config, ConfigShapedDevices, ShapedDevice, TopologyCanonicalStateFile,
    TopologyCircuitMatchIndex, TopologyParentCandidatesFile,
};
use lqos_topology_compile::{
    CompiledTopologyBundle, ImportedTopologyBundle, TopologyCompileMode, compile_topology,
    parse_libreqos_network,
};
use serde_json::Value;
use std::path::PathBuf;

const FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn fixtures_dir() -> PathBuf {
    PathBuf::from(FIXTURES_DIR)
}

fn format_payload() -> String {
    std::fs::read_to_string(fixtures_dir().join("libreqos_network.json"))
        .expect("the standard-format fixture should be readable")
}

fn legacy_network_json() -> Value {
    let raw = std::fs::read_to_string(fixtures_dir().join("legacy/network.json"))
        .expect("the legacy network.json fixture should be readable");
    serde_json::from_str(&raw).expect("the legacy network.json fixture should parse")
}

fn canonical_fixture() -> TopologyCanonicalStateFile {
    let raw = std::fs::read_to_string(
        fixtures_dir().join("legacy/state/topology/topology_canonical_state.json"),
    )
    .expect("the canonical-state fixture should be readable");
    serde_json::from_str(&raw).expect("the canonical-state fixture should parse")
}

fn legacy_config() -> Config {
    let dir = fixtures_dir().join("legacy");
    Config {
        lqos_directory: dir.display().to_string(),
        state_directory: Some(dir.join("state").display().to_string()),
        ..Config::default()
    }
}

fn format_bundle() -> ImportedTopologyBundle {
    parse_libreqos_network(&format_payload())
        .expect("the standard-format fixture should validate")
        .into_imported_bundle("libreqos_network")
}

fn legacy_bundle() -> ImportedTopologyBundle {
    ImportedTopologyBundle::from_legacy_artifacts(&legacy_config(), "legacy/artifacts")
        .expect("the legacy on-disk pair should import")
}

fn shaped_devices(rows: Vec<ShapedDevice>) -> ConfigShapedDevices {
    let mut devices = ConfigShapedDevices::default();
    devices.replace_with_new_data(rows);
    devices
}

/// Builds a representative bundle for a door that carries the pinned native
/// canonical shape plus its own compatibility tree and parent candidates.
fn representative_bundle(
    source: &str,
    native_canonical: Option<TopologyCanonicalStateFile>,
    compatibility_network_json: Value,
    base: &ImportedTopologyBundle,
) -> ImportedTopologyBundle {
    ImportedTopologyBundle {
        source: source.to_string(),
        generated_unix: None,
        ingress_identity: None,
        native_canonical,
        parent_candidates: Some(TopologyParentCandidatesFile::default()),
        compatibility_network_json,
        shaped_devices: shaped_devices(base.shaped_devices.devices.clone()),
        circuit_anchors: base.circuit_anchors.clone(),
        ethernet_advisories: Vec::new(),
        circuit_matches: base.circuit_matches.clone(),
    }
}

#[derive(Debug, PartialEq)]
struct DeviceRow {
    circuit_id: String,
    circuit_name: String,
    device_id: String,
    device_name: String,
    parent_node: String,
    parent_node_id: Option<String>,
    mac: String,
    ipv4: Vec<(std::net::Ipv4Addr, u32)>,
    ipv6: Vec<(std::net::Ipv6Addr, u32)>,
    download_min_mbps: f32,
    upload_min_mbps: f32,
    download_max_mbps: f32,
    upload_max_mbps: f32,
}

fn device_rows(compiled: &CompiledTopologyBundle) -> Vec<DeviceRow> {
    let mut rows = compiled
        .shaped_devices
        .devices
        .iter()
        .map(|device| DeviceRow {
            circuit_id: device.circuit_id.clone(),
            circuit_name: device.circuit_name.clone(),
            device_id: device.device_id.clone(),
            device_name: device.device_name.clone(),
            parent_node: device.parent_node.clone(),
            parent_node_id: device.parent_node_id.clone(),
            mac: device.mac.clone(),
            ipv4: device.ipv4.clone(),
            ipv6: device.ipv6.clone(),
            download_min_mbps: device.download_min_mbps,
            upload_min_mbps: device.upload_min_mbps,
            download_max_mbps: device.download_max_mbps,
            upload_max_mbps: device.upload_max_mbps,
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        (&left.circuit_id, &left.device_id).cmp(&(&right.circuit_id, &right.device_id))
    });
    rows
}

fn anchor_rows(compiled: &CompiledTopologyBundle) -> Vec<(String, String, Option<String>)> {
    let mut rows = compiled
        .circuit_anchors
        .anchors
        .iter()
        .map(|anchor| {
            (
                anchor.circuit_id.clone(),
                anchor.anchor_node_id.clone(),
                anchor.anchor_node_name.clone(),
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

fn compile_ap_only(bundle: ImportedTopologyBundle) -> CompiledTopologyBundle {
    compile_topology(bundle, TopologyCompileMode::ApOnly).expect("ap_only compilation should pass")
}

/// The two circuits in the fixture both attach under `ap1`.
fn expected_device_rows() -> Vec<DeviceRow> {
    vec![
        DeviceRow {
            circuit_id: "c1".to_string(),
            circuit_name: "Circuit One".to_string(),
            device_id: "d1".to_string(),
            device_name: "d1".to_string(),
            parent_node: "AP One".to_string(),
            parent_node_id: Some("ap1".to_string()),
            mac: "aa:bb:cc:dd:ee:ff".to_string(),
            ipv4: vec![("192.0.2.10".parse().unwrap(), 32)],
            ipv6: Vec::new(),
            download_min_mbps: 10.0,
            upload_min_mbps: 5.0,
            download_max_mbps: 100.0,
            upload_max_mbps: 50.0,
        },
        DeviceRow {
            circuit_id: "c2".to_string(),
            circuit_name: "Circuit Two".to_string(),
            device_id: "d2".to_string(),
            device_name: "d2".to_string(),
            parent_node: "AP One".to_string(),
            parent_node_id: Some("ap1".to_string()),
            mac: "aa:bb:cc:dd:ee:01".to_string(),
            ipv4: vec![("192.0.2.11".parse().unwrap(), 32)],
            ipv6: Vec::new(),
            download_min_mbps: 20.0,
            upload_min_mbps: 10.0,
            download_max_mbps: 200.0,
            upload_max_mbps: 100.0,
        },
    ]
}

#[test]
fn format_fixture_reads_and_projects_into_native_canonical() {
    let file = parse_libreqos_network(&format_payload()).expect("fixture should validate");
    assert_eq!(file.schema_version, 1);
    assert_eq!(file.topology.len(), 2);
    assert_eq!(file.circuits.len(), 2);

    let bundle = file.into_imported_bundle("libreqos_network");
    let canonical = bundle
        .native_canonical
        .expect("the format targets native_canonical");
    assert_eq!(canonical.nodes.len(), 2);
    let ap = canonical
        .nodes
        .iter()
        .find(|node| node.node_id == "ap1")
        .expect("ap1 should be projected");
    assert_eq!(ap.node_kind, "ap");
    assert_eq!(ap.current_parent_node_id.as_deref(), Some("s1"));
    assert_eq!(ap.current_parent_node_name.as_deref(), Some("Site One"));
    assert_eq!(ap.rate_input.intrinsic_download_mbps, Some(100));

    assert_eq!(device_rows(&compile_ap_only(format_bundle())).len(), 2);
    assert!(
        bundle.circuit_matches.is_empty(),
        "ip mode carries no per-circuit identity"
    );
    assert_eq!(
        bundle.compatibility_network_json,
        Value::Object(serde_json::Map::new()),
        "the compatibility tree is the legacy door's business"
    );
}

#[test]
fn four_doors_converge_within_the_pinned_native_canonical_shape() {
    let canonical = canonical_fixture();
    let compat = legacy_network_json();

    let format = format_bundle();
    assert_eq!(
        format
            .native_canonical
            .as_ref()
            .expect("format native canonical")
            .nodes,
        canonical.nodes,
        "the standard format and the pinned canonical fixture must agree"
    );

    let legacy = legacy_bundle();
    assert_eq!(
        legacy
            .native_canonical
            .as_ref()
            .expect("legacy native canonical")
            .nodes,
        canonical.nodes,
        "the on-disk canonical state must carry the pinned shape"
    );

    let uisp = representative_bundle(
        "uisp/full2",
        Some(canonical.clone()),
        compat.clone(),
        &format,
    );
    let python = representative_bundle(
        "python/integration_common",
        Some(canonical.clone()),
        compat.clone(),
        &format,
    );

    // (3.1): the doors' input shapes are NOT identical — the format targets
    // native_canonical and leaves the compatibility tree empty, while the
    // legacy/UISP/Python doors carry the compatibility tree. That shape
    // difference is asserted, not silently converged. Capture it before the
    // bundles are consumed by the compiler.
    let format_compat_is_empty =
        format.compatibility_network_json == Value::Object(serde_json::Map::new());
    let legacy_compat_is_non_empty = legacy
        .compatibility_network_json
        .as_object()
        .is_some_and(|tree| !tree.is_empty());

    let compiled = [
        ("format", compile_ap_only(format)),
        ("legacy", compile_ap_only(legacy)),
        ("uisp", compile_ap_only(uisp)),
        ("python", compile_ap_only(python)),
    ];

    let expected_devices = expected_device_rows();
    let expected_anchors = vec![
        (
            "c1".to_string(),
            "ap1".to_string(),
            Some("AP One".to_string()),
        ),
        (
            "c2".to_string(),
            "ap1".to_string(),
            Some("AP One".to_string()),
        ),
    ];
    for (door, bundle) in &compiled {
        assert_eq!(
            &device_rows(bundle),
            &expected_devices,
            "{door} door should project the same shaped devices"
        );
        assert_eq!(
            &anchor_rows(bundle),
            &expected_anchors,
            "{door} door should project the same anchors"
        );
        assert_eq!(
            &bundle.circuit_matches,
            &TopologyCircuitMatchIndex::new(),
            "{door} door carries no ip-mode identity"
        );
        assert_eq!(
            bundle.compatibility_network_json["AP One"]["downloadBandwidthMbps"],
            serde_json::json!(100),
            "{door} door should inherit the AP rate from the pinned canonical shape"
        );
    }

    assert!(
        format_compat_is_empty,
        "the standard format leaves the compatibility tree to the legacy door"
    );
    assert!(
        legacy_compat_is_non_empty,
        "the legacy door carries the compatibility tree as its input shape"
    );
}

#[test]
fn compatibility_only_shape_diverges_by_the_pinned_rate_fallback() {
    let format = format_bundle();
    let compat_only = representative_bundle(
        "legacy/compatibility-only",
        None,
        legacy_network_json(),
        &format,
    );

    let compiled = compile_ap_only(compat_only);
    assert_eq!(
        device_rows(&compiled),
        expected_device_rows(),
        "the compatibility tree still places the circuits under the same AP"
    );
    assert_eq!(
        compiled.compatibility_network_json["AP One"]["downloadBandwidthMbps"],
        serde_json::json!(1),
        "ASSERTED DIVERGENCE (3.1): the compatibility-only shape has no rate, \
         so the AP falls back to 1 Mbps while the native shape carries 100"
    );
}
