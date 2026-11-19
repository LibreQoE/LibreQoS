//! U2/U3 — no-sudo dynamic-circuit LIFECYCLE harness (design card §5.3).
//!
//! One test walks the real chain with no root, no bridge process and no
//! network:
//!
//! 1. baseline publish of a hermetic temp config with NO dynamic circuits;
//! 2. create through the production front door (`dynamic_circuits::
//!    create_dynamic_circuit`, the bus handler RADIUS reaches);
//! 3. the CHURN gate: 20 liveness-only renewals through the same RADIUS path
//!    move neither the source generation, the store bytes, the store
//!    inode/mtime, nor (at steady state) the broker delta;
//! 4. the real topology publish (`publish_effective_topology_artifacts`) and
//!    the committed artifact `topology_dataplane_transaction_path`;
//! 5. `translate::semantic_diff` of that artifact against the applied world —
//!    EXACTLY one leaf `Upsert` and one `Map`;
//! 6. `step` against a test seam segment: the frames are pushed, the injected
//!    ack folds, and the applied belief converges on the new circuit;
//! 7. (u3) the SHAPED-DEVICES RELOAD that used to prune it: the actor re-reads
//!    the post-fold runtime shaping inputs, and the circuit must survive with
//!    its store untouched, still shaped, and no `Remove` emitted;
//! 8. expiry through the production removal handler → `Remove` + `Del` → ack
//!    → the circuit is gone from the applied world;
//! 9. (u3) REAL PROVISIONING supersedes it: an operator device covering the
//!    subscriber's address makes the reload prune the circuit, and that prune
//!    emits `Remove` + `Del` which the bridge acks.
//!
//! The churn gate runs BEFORE the fold-carrying publish deliberately: that keeps
//! it measuring membership-identical writes alone, with no directory event or
//! fold-shaped reload able to interleave.
//!
//! The evidence (counts, run stamp, op-delta summary, origins) lands in
//! [`EVIDENCE_PATH`] outside the repository.
//!
//! Global state: the daemon-mode actor is started once per test binary
//! (`ACTOR`), and `step`'s idle arm owns the real `SEGMENT_NAME`, so the test
//! serializes on `test_fixtures::seam_segment_lock` AND on the repo's
//! `runtime_config_test_lock` for the config globals it mutates.
#![cfg(test)]

use super::seam::{AckFrame, SEGMENT_NAME, SeamSegment, create_full_test_segment, unlink_test_segment};
use super::translate;
use super::{BrokerState, step};
use lqos_config::{Config, DynamicCircuitsFile, ShapedDevice};
use lqos_dataplane_types::wire::AckOutcome;
use lqos_dataplane_types::{
    DataplaneInnerOp, DataplaneLeafOp, DataplaneMappingOp, DataplaneTransaction,
};
use std::net::Ipv4Addr;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime};

/// The circuit the RADIUS Start creates, then expires.
const CIRCUIT_ID: &str = "dyn-sub-1";
/// Its device row id.
const CIRCUIT_DEVICE_ID: &str = "dyn-dev-1";
/// The subscriber address the circuit maps (`/32`).
const CIRCUIT_IP: Ipv4Addr = Ipv4Addr::new(100, 64, 0, 5);
/// The only static node in the hermetic `network.json`.
const SITE_ID: &str = "s1";
/// The provisioned circuit the world already carries (so the site is not
/// elided, and the create delta is the circuit alone — never structural churn).
const STATIC_CIRCUIT_ID: &str = "static-1";
/// The provisioned circuit's address.
const STATIC_IP: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 10);
/// The provisioned circuit the supersede leg adds: a REAL device covering the
/// dynamic subscriber's address with a BROADER prefix, so the dynamic `/32`
/// shadows it by longest-prefix match instead of colliding on one prefix.
const SUPERSEDING_CIRCUIT_ID: &str = "static-2";
/// The superseding provisioned prefix, which contains `CIRCUIT_IP`.
const SUPERSEDING_SUBNET: Ipv4Addr = Ipv4Addr::new(100, 64, 0, 0);
const SUPERSEDING_PREFIX_LEN: u8 = 24;
/// The node NAME the RADIUS fallback carries (the fold resolves it by name).
const SITE_NAME: &str = "Site One";
/// Liveness-only renewals replayed by the churn gate.
const CHURN_RENEWALS: usize = 20;
/// Machine-readable evidence (outside the repository).
const EVIDENCE_PATH: &str = "/tmp/libreqos-u2-dynamic-lifecycle-evidence.json";
/// The hermetic runtime directory. A fixed path, wiped per run: the actor the
/// test starts outlives it and keeps reading this config, so it must stay
/// valid (and must not accumulate a directory per run).
const RUNTIME_DIR: &str = "/tmp/libreqos-u2-lifecycle-runtime";
/// Bound on the actor's directory-watch -> reload -> supersede-prune round
/// trip after a fold-carrying publish.
const PRUNE_DEADLINE: Duration = Duration::from_secs(30);
/// The broker's real ACK window is 30 s; a harness ack is injected, so a wait
/// this short only bounds a broken harness.
const HARNESS_ACK_TIMEOUT_MS: u64 = 5_000;

/// A site whose operator minimum (900 Mbps) dominates the sum of its circuits'
/// minima, so a circuit create/expire is the ONLY op it can produce. Without
/// the pin the parent's commit is `Σ descendant circuit commit` and a create
/// legitimately re-rates the site — real behaviour, but it would blur the
/// circuit delta this harness asserts.
fn network_json() -> &'static str {
    r#"{
  "Site One": {
    "children": {},
    "id": "s1",
    "name": "Site One",
    "type": "Site",
    "downloadBandwidthMbps": 1000,
    "uploadBandwidthMbps": 1000,
    "downloadBandwidthMbpsMin": 900,
    "uploadBandwidthMbpsMin": 900
  }
}"#
}

/// One provisioned circuit under the site. The site must already be
/// materialized (a circuit-less site is elided from the transaction), so the
/// dynamic circuit's create delta is its own leaf + Map and nothing else.
fn shaped_devices_csv() -> &'static str {
    concat!(
        "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,",
        "Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,",
        "Download Max Mbps,Upload Max Mbps,Comment\n",
        "\"static-1\",\"Static 1\",\"static-dev-1\",\"Static 1\",\"Site One\",\"\",",
        "\"\",\"aa:bb:cc:dd:ee:00\",\"203.0.113.10/32\",\"\",10,5,100,50,\"\"\n",
    )
}

/// The same world after an operator provisions a real device over the dynamic
/// subscriber's address (`static-2` owns the superseding prefix).
fn shaped_devices_csv_with_superseding_device() -> String {
    format!(
        "{}{}",
        shaped_devices_csv(),
        format!(
            "\"static-2\",\"Static 2\",\"static-dev-2\",\"Static 2\",\"Site One\",\"\",\
             \"\",\"aa:bb:cc:dd:ee:02\",\"{SUPERSEDING_SUBNET}/{SUPERSEDING_PREFIX_LEN}\",\
             \"\",10,5,100,50,\"\"\n"
        )
    )
}

/// The `ShapedDevice` the RADIUS accounting worker hands the bus on a Start.
fn radius_device() -> ShapedDevice {
    ShapedDevice {
        circuit_id: CIRCUIT_ID.to_string(),
        circuit_name: "Dynamic Sub 1".to_string(),
        device_id: CIRCUIT_DEVICE_ID.to_string(),
        device_name: "Dynamic Sub 1".to_string(),
        parent_node: SITE_NAME.to_string(),
        parent_node_id: None,
        anchor_node_id: None,
        mac: "aa-bb-cc-dd-ee-ff".to_string(),
        ipv4: vec![(CIRCUIT_IP, 32)],
        ipv6: Vec::new(),
        download_min_mbps: 10.0,
        upload_min_mbps: 5.0,
        download_max_mbps: 50.0,
        upload_max_mbps: 20.0,
        comment: "u2 lifecycle harness".to_string(),
        sqm_override: None,
        circuit_hash: 0,
        device_hash: 0,
        parent_hash: 0,
    }
}

/// A temp `lqos.conf` populated with the shipped example's defaults, pointed at
/// `dir`, carrying the dynamic-circuit section the harness needs.
fn config_text(dir: &Path) -> String {
    let example = include_str!("../../../lqos_config/src/etc/v15/example.toml");
    let with_dirs = example
        .replacen(
            "lqos_directory = \"/opt/libreqos/src\"",
            &format!("lqos_directory = {:?}", dir.to_string_lossy()),
            1,
        )
        .replacen(
            "state_directory = \"/opt/libreqos/state\"",
            &format!("state_directory = {:?}", dir.join("state").to_string_lossy()),
            1,
        );
    // `ttl_seconds` is the shipped default: this harness expires through the
    // production removal handler, never through the actor's 60 s TTL tick.
    format!("{with_dirs}\n[dynamic_circuits]\nenabled = true\nttl_seconds = 300\n")
}

/// Starts the daemon-mode actor exactly once per test binary.
///
/// The actor has no stop path (`ACTOR_SENDER` is a `OnceCell`) and keeps
/// running after the test finishes, so this is the ONE global the harness must
/// leave in a benign state — see [`Runtime`]'s `Drop`.
fn start_actor_once() {
    static ACTOR: OnceLock<Result<(), String>> = OnceLock::new();
    let started = ACTOR.get_or_init(|| {
        lqos_network_devices::start_daemon_mode(None).map_err(|error| error.to_string())
    });
    assert!(
        started.is_ok(),
        "the daemon-mode actor must start for the lifecycle harness: {started:?}"
    );
}

/// A hermetic runtime directory plus the process-global config that points at
/// it: `LQOS_CONFIG`, `LQOS_DIRECTORY`, and the cached `Config`.
///
/// The env is deliberately NOT restored on drop. The actor started by this
/// harness outlives the test and its 60 s maintenance tick calls
/// `load_config()` + the liveness checkpoint writer; restoring the machine's
/// real `LQOS_CONFIG` would aim those writes at a live install. The fixed
/// [`RUNTIME_DIR`] stays behind as a benign tombstone for that tick.
struct Runtime {
    /// Held for the test's lifetime: every env/config mutation below is a
    /// process global.
    _config_guard: MutexGuard<'static, ()>,
}

impl Runtime {
    fn start() -> Self {
        let config_guard = crate::test_support::runtime_config_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = PathBuf::from(RUNTIME_DIR);
        // A stale store from a previous run would feed the actor's startup load.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("state")).expect("create the state directory");
        std::fs::write(dir.join("network.json"), network_json()).expect("write network.json");
        std::fs::write(dir.join("ShapedDevices.csv"), shaped_devices_csv())
            .expect("write ShapedDevices.csv");
        let config_path = dir.join("lqos.conf");
        std::fs::write(&config_path, config_text(&dir)).expect("write lqos.conf");
        // SAFETY: the env is process-global; the runtime-config test lock
        // serializes every test that mutates it.
        unsafe {
            std::env::set_var("LQOS_CONFIG", &config_path);
            std::env::set_var("LQOS_DIRECTORY", &dir);
        }
        lqos_config::clear_cached_config();
        let config = lqos_config::load_config().expect("the hermetic config loads");
        assert_eq!(
            config.lqos_directory,
            dir.display().to_string(),
            "the config must point at the hermetic directory, not the host"
        );
        lqos_topology_compile::materialize_manual_topology(&config)
            .expect("the manual topology pair materializes into the one ingest");
        start_actor_once();
        Self {
            _config_guard: config_guard,
        }
    }

    fn config(&self) -> Arc<Config> {
        lqos_config::load_config().expect("the hermetic config is cached")
    }

    fn store_path(&self) -> PathBuf {
        lqos_config::dynamic_circuits_path(&self.config())
    }

    fn artifact_path(&self) -> PathBuf {
        lqos_config::topology_dataplane_transaction_path(&self.config())
    }
}

/// `(inode, mtime)` of `path` — the rewrite probe behind the churn gate: the
/// dedupe contract is "no temp file, no rename, no fsync", which leaves both
/// untouched.
fn file_identity(path: &Path) -> (u64, SystemTime) {
    let metadata = std::fs::metadata(path).expect("stat the dynamic-circuit store");
    (
        metadata.ino(),
        metadata.modified().expect("the store has an mtime"),
    )
}

/// Whether the circuit is in `dynamic_circuits.json` (the membership store).
fn store_has_circuit(config: &Config) -> bool {
    DynamicCircuitsFile::load(config)
        .circuits
        .iter()
        .any(|circuit| circuit.shaped.circuit_id == CIRCUIT_ID)
}

/// The leaf `Upsert` circuit ids in `tx` (removes and moves excluded).
fn leaf_upserts(tx: &DataplaneTransaction) -> Vec<String> {
    tx.leaf_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneLeafOp::Upsert { circuit_id, .. } => Some(circuit_id.clone()),
            _ => None,
        })
        .collect()
}

/// `leaf_upserts` as a sorted set (the compiler's leaf order is not the
/// assertion: membership is).
fn leaf_upsert_set(tx: &DataplaneTransaction) -> Vec<String> {
    let mut ids = leaf_upserts(tx);
    ids.sort();
    ids
}

fn leaf_removes(tx: &DataplaneTransaction) -> Vec<String> {
    tx.leaf_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneLeafOp::Remove { circuit_id } => Some(circuit_id.clone()),
            _ => None,
        })
        .collect()
}

fn mapped_ips(tx: &DataplaneTransaction) -> Vec<(u8, std::net::IpAddr, String)> {
    tx.mapping_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneMappingOp::Map {
                prefix_len,
                ip,
                circuit_id,
                ..
            } => Some((*prefix_len, *ip, circuit_id.clone())),
            _ => None,
        })
        .collect()
}

fn deleted_ips(tx: &DataplaneTransaction) -> Vec<(u8, std::net::IpAddr)> {
    tx.mapping_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneMappingOp::Del { prefix_len, ip } => Some((*prefix_len, *ip)),
            _ => None,
        })
        .collect()
}

/// Runs one broker publish round: the first `step` diffs the artifact into an
/// in-flight publish, the injected `Applied` ack is consumed by the second
/// `step`, which pushes the frames and folds the applied belief.
///
/// Returns the number of frames the transaction needed.
fn publish_round(state: &mut BrokerState, artifact: &PathBuf) -> usize {
    step(artifact, state);
    let pending = state
        .pending
        .as_ref()
        .expect("the step emits the delta as an in-flight publish");
    let frames = pending.frames.len();
    let ack = AckFrame::new(AckOutcome::Applied, &pending.generation, 0);
    state
        .segment
        .as_mut()
        .expect("the harness attaches a test segment")
        .push_test_ack(&ack);
    step(artifact, state);
    assert!(
        state.pending.is_none(),
        "the applied ack must clear the in-flight publish"
    );
    frames
}

/// The applied world's convergence identity for `tx` (what the broker records
/// after a successful ack fold).
fn converged_key(tx: &DataplaneTransaction) -> String {
    translate::content_key(&translate::apply_suspend_floor(tx))
}

/// Waits until `predicate` holds, or panics at the deadline with `what`.
fn wait_for(what: &str, deadline: Duration, mut predicate: impl FnMut() -> bool) {
    let start = Instant::now();
    while !predicate() {
        assert!(
            start.elapsed() < deadline,
            "timed out after {:?} waiting for {what}",
            start.elapsed()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Drives the actor's shaped-devices reload on this thread.
///
/// This is the production step that used to prune a freshly published dynamic
/// circuit (the u2 finding): it re-reads the POST-fold runtime shaping inputs,
/// publishes them, and runs the supersede rule against them. Driving it
/// synchronously keeps the gate off the directory watcher's schedule.
fn drive_shaped_devices_reload(reason: &str) {
    lqos_network_devices::request_reload_shaped_devices(reason)
        .expect("the actor must reload the folded shaped devices");
}

/// The origin flags the published artifact carries for one circuit's device
/// rows (`true` = the fold synthesized the row from the dynamic store).
fn published_device_origins(config: &Config, circuit_id: &str) -> Vec<bool> {
    lqos_config::load_active_runtime_shaping_inputs(config)
        .expect("the published shaping inputs load")
        .expect("a publish makes the shaping inputs active")
        .circuits
        .iter()
        .filter(|circuit| circuit.circuit_id == circuit_id)
        .flat_map(|circuit| circuit.devices.iter().map(|device| device.dynamic))
        .collect()
}

#[test]
fn dynamic_circuit_lifecycle_create_publish_apply_expire_and_no_churn() {
    let _seam_guard = super::test_fixtures::seam_segment_lock();
    super::TEST_ACK_TIMEOUT_MS.with(|cell| cell.set(Some(HARNESS_ACK_TIMEOUT_MS)));
    let runtime = Runtime::start();
    let config = runtime.config();
    let artifact = runtime.artifact_path();
    let store = runtime.store_path();

    // 1. Baseline: the hermetic world carries the provisioned circuit only.
    let baseline = super::oracle1_live_tests::run_pipeline(&config);
    assert_eq!(
        leaf_upserts(&baseline),
        vec![STATIC_CIRCUIT_ID.to_string()],
        "the baseline world carries the provisioned circuit and no dynamic one"
    );
    assert_eq!(
        mapped_ips(&baseline),
        vec![(
            32,
            std::net::IpAddr::V4(STATIC_IP),
            STATIC_CIRCUIT_ID.to_string()
        )],
        "the baseline world maps the provisioned circuit's address only"
    );
    assert_eq!(
        baseline
            .inner_ops
            .iter()
            .filter(|op| matches!(op, DataplaneInnerOp::Upsert { node_id, .. } if node_id == SITE_ID))
            .count(),
        1,
        "the baseline world must carry the site node"
    );

    // 2. Create through the production front door (the RADIUS bus handler).
    let response = crate::dynamic_circuits::create_dynamic_circuit(radius_device());
    assert_eq!(
        response,
        lqos_bus::BusResponse::Ack,
        "the bus handler must accept the dynamic circuit"
    );
    assert!(
        store_has_circuit(&config),
        "the circuit must be in the membership store"
    );

    // 3. CHURN GATE. Liveness-only renewals run BEFORE the folded world is
    //    published on purpose: the actor's shaped-devices reload folds the
    //    dynamic row into the published weights, so a later publish is what
    //    arms the supersede prune (see step 6). Renewals write nothing, so no
    //    directory event and no reload can interleave here.
    let generation_before =
        lqos_config::compute_topology_source_generation(&config).expect("source generation");
    let store_bytes_before = std::fs::read(&store).expect("read the store");
    let store_identity_before = file_identity(&store);
    for renewal in 0..CHURN_RENEWALS {
        let response = crate::dynamic_circuits::create_dynamic_circuit(radius_device());
        assert_eq!(
            response,
            lqos_bus::BusResponse::Ack,
            "liveness renewal {renewal} must be accepted"
        );
    }
    let generation_after =
        lqos_config::compute_topology_source_generation(&config).expect("source generation");
    let store_rewritten = file_identity(&store) != store_identity_before;
    assert_eq!(
        generation_before, generation_after,
        "a liveness-only renewal must not move the topology source generation"
    );
    assert_eq!(
        std::fs::read(&store).expect("read the store"),
        store_bytes_before,
        "a liveness-only renewal must not change the store bytes"
    );
    assert!(
        !store_rewritten,
        "a liveness-only renewal must not rewrite the store file at all"
    );

    // 4. The real publish writes the artifact the broker consumes.
    let desired = super::oracle1_live_tests::run_pipeline(&config);
    let mut expected_after_create = vec![CIRCUIT_ID.to_string(), STATIC_CIRCUIT_ID.to_string()];
    expected_after_create.sort();
    assert_eq!(
        leaf_upsert_set(&desired),
        expected_after_create,
        "the published world must carry the provisioned and the dynamic circuit"
    );
    assert_eq!(
        mapped_ips(&desired),
        vec![
            (
                32,
                std::net::IpAddr::V4(CIRCUIT_IP),
                CIRCUIT_ID.to_string()
            ),
            (
                32,
                std::net::IpAddr::V4(STATIC_IP),
                STATIC_CIRCUIT_ID.to_string()
            ),
        ],
        "the published world must map both circuits' addresses"
    );

    // 5. The semantic delta against the applied world is EXACTLY the create.
    let create_diff = translate::semantic_diff(&desired, Some(&baseline), None);
    let create_delta = create_diff.delta.expect("a create publishes a delta");
    assert_eq!(
        leaf_upserts(&create_delta),
        vec![CIRCUIT_ID.to_string()],
        "exactly one leaf Upsert"
    );
    assert_eq!(
        mapped_ips(&create_delta),
        vec![(
            32,
            std::net::IpAddr::V4(CIRCUIT_IP),
            CIRCUIT_ID.to_string()
        )],
        "exactly one Map for the circuit's address"
    );
    assert!(
        create_delta.inner_ops.is_empty(),
        "the site's operator minimum pins its commit — the delta carries no inner op, got {:?}",
        create_delta.inner_ops
    );
    assert!(
        leaf_removes(&create_delta).is_empty() && deleted_ips(&create_delta).is_empty(),
        "the create delta must not remove anything"
    );

    // 6. Broker: push the frames against a test seam segment, ack, converge.
    create_full_test_segment(SEGMENT_NAME, [0x2a; 16]);
    let segment = SeamSegment::attach(SEGMENT_NAME).expect("attach the full test segment");
    let mut state = super::test_fixtures::broker_state(Some(segment), baseline.clone());
    let create_frames = publish_round(&mut state, &artifact);
    assert!(create_frames > 0, "the create must push at least one frame");
    assert_eq!(
        state.segment.as_ref().expect("still attached").pushed_frames(),
        create_frames as u64,
        "every encoded frame must reach the seam ring"
    );
    let applied_after_create = state
        .applied
        .tx()
        .cloned()
        .expect("the ack fold leaves an applied world");
    assert_eq!(
        leaf_upsert_set(&applied_after_create),
        expected_after_create,
        "the applied world converged on the new circuit"
    );
    let create_converged = state.applied.key() == Some(converged_key(&desired).as_str());
    assert!(
        create_converged,
        "the applied belief converged on the published world's content key"
    );

    // 6b. Steady state: the same artifact emits NO further op delta.
    step(&artifact, &mut state);
    assert!(
        state.pending.is_none(),
        "a converged applied world re-emits nothing for the same artifact"
    );
    assert!(
        translate::semantic_diff(&desired, state.applied.tx(), None)
            .delta
            .is_none(),
        "the applied world IS the desired world — zero broker op delta"
    );

    // 6c. The reload that used to prune (the u2 finding). The actor re-reads the
    //     post-fold runtime shaping inputs; the supersede rule must see the
    //     dynamic row's ORIGIN instead of mistaking the fold for a provisioned
    //     device. The gate asserts all three things the finding broke: the store
    //     survives untouched, the circuit stays shaped, and no Remove moves.
    let store_identity_before_reload = file_identity(&store);
    drive_shaped_devices_reload("u3 published-dynamic-circuit-survives");
    let reload_kept_circuit = store_has_circuit(&config);
    let reload_rewrote_store = file_identity(&store) != store_identity_before_reload;
    assert!(
        reload_kept_circuit,
        "a published dynamic circuit must survive the shaped-devices reload"
    );
    assert!(
        !reload_rewrote_store,
        "surviving the reload must not rewrite the membership store (no churn)"
    );
    let after_reload = super::oracle1_live_tests::run_pipeline(&config);
    assert_eq!(
        leaf_upsert_set(&after_reload),
        expected_after_create,
        "the reload must leave the dynamic circuit shaped"
    );
    assert!(
        translate::semantic_diff(&after_reload, state.applied.tx(), None)
            .delta
            .is_none(),
        "the reload must not emit a Remove op for the surviving circuit"
    );
    let reload_origins_dynamic = published_device_origins(&config, CIRCUIT_ID);
    let reload_origins_provisioned = published_device_origins(&config, STATIC_CIRCUIT_ID);
    assert_eq!(
        reload_origins_dynamic,
        vec![true],
        "the fold must mark the synthesized row dynamic-origin in the artifact"
    );
    assert_eq!(
        reload_origins_provisioned,
        vec![false],
        "a provisioned row stays unmarked"
    );

    // 6d. REV-T1: the EDIT/apply path carries the origin forward. An operator
    //     edit (`apply_shaped_devices_snapshot`) replaces the published world
    //     with an edited copy of it, so the row origins must ride along; if they
    //     are dropped, the supersede rule reads the fold row as a provisioned
    //     device again and prunes the circuit by its own address. No broker step
    //     here: the seam has four slots and the earlier legs spend all of them.
    let live_world = lqos_network_devices::shaped_devices_snapshot();
    let mut edited = lqos_config::ConfigShapedDevices::default();
    edited.replace_with_new_data(live_world.devices.clone());
    let store_identity_before_apply = file_identity(&store);
    lqos_network_devices::apply_shaped_devices_snapshot(
        "u3 repair-round operator edit",
        edited,
        lqos_network_devices::ShapedDevicesSnapshotProvenance::ExternalSnapshot,
    )
    .expect("the operator edit must apply");
    assert!(
        store_has_circuit(&config),
        "an operator edit must not retire a live dynamic circuit"
    );
    assert!(
        file_identity(&store) == store_identity_before_apply,
        "the edit path must not rewrite the membership store"
    );
    let after_apply = super::oracle1_live_tests::run_pipeline(&config);
    assert!(
        leaf_upserts(&after_apply).contains(&CIRCUIT_ID.to_string()),
        "the circuit must stay shaped through the edit path"
    );
    assert!(
        !leaf_removes(&after_apply).contains(&CIRCUIT_ID.to_string()),
        "the edit path must not emit a Remove for the circuit"
    );

    // 7. Expiry through the production removal handler (the RADIUS Stop path).
    assert_eq!(
        crate::dynamic_circuits::remove_dynamic_circuit(CIRCUIT_ID),
        lqos_bus::BusResponse::Ack,
        "the removal handler must accept the expiry"
    );
    wait_for(
        "the expired circuit to leave the membership store",
        PRUNE_DEADLINE,
        || !store_has_circuit(&config),
    );
    let expired = super::oracle1_live_tests::run_pipeline(&config);
    assert_eq!(
        expired.generation, baseline.generation,
        "expiry returns the world to the baseline generation"
    );
    let expiry_diff = translate::semantic_diff(&expired, state.applied.tx(), None);
    let expiry_delta = expiry_diff.delta.expect("expiry publishes a delta");
    assert_eq!(
        leaf_removes(&expiry_delta),
        vec![CIRCUIT_ID.to_string()],
        "exactly one leaf Remove"
    );
    assert_eq!(
        deleted_ips(&expiry_delta),
        vec![(32, std::net::IpAddr::V4(CIRCUIT_IP))],
        "exactly one Del for the circuit's address"
    );
    assert!(
        leaf_upserts(&expiry_delta).is_empty(),
        "the expiry delta must not upsert a leaf"
    );
    let expiry_frames = publish_round(&mut state, &artifact);
    assert!(expiry_frames > 0, "the expiry must push at least one frame");
    let applied_after_expiry = state
        .applied
        .tx()
        .cloned()
        .expect("the ack fold leaves an applied world");
    assert_eq!(
        leaf_upsert_set(&applied_after_expiry),
        vec![STATIC_CIRCUIT_ID.to_string()],
        "the expired circuit must leave the applied world"
    );
    assert_eq!(
        mapped_ips(&applied_after_expiry),
        vec![(
            32,
            std::net::IpAddr::V4(STATIC_IP),
            STATIC_CIRCUIT_ID.to_string()
        )],
        "its mapping must leave the applied world too"
    );
    assert_eq!(
        state.applied.key(),
        Some(converged_key(&expired).as_str()),
        "the applied belief converged back on the baseline world"
    );

    // 8. A re-created circuit for the supersede leg, published TOGETHER with the
    //    real provisioning so the harness stays inside the fake ring's
    //    `CMD_SLOTS` (= 4) budget: nothing consumes command frames, so each
    //    publish round costs one slot. The Stop path above emptied the store, and
    //    a supersede needs a live dynamic circuit, so create it again through the
    //    same front door, then give the operator a REAL device that covers its
    //    address: `static-2` owns a /24 containing the subscriber's /32, so the
    //    circuit shadows a genuine device by longest-prefix match — exactly the
    //    case the origin filter must NOT excuse.
    assert_eq!(
        crate::dynamic_circuits::create_dynamic_circuit(radius_device()),
        lqos_bus::BusResponse::Ack,
        "the RADIUS front door must accept the re-created circuit"
    );
    std::fs::write(
        PathBuf::from(&config.lqos_directory).join("ShapedDevices.csv"),
        shaped_devices_csv_with_superseding_device(),
    )
    .expect("write the superseding ShapedDevices.csv");
    lqos_topology_compile::materialize_manual_topology(&config)
        .expect("the manual pair re-materializes into the one ingest");
    let provisioned = super::oracle1_live_tests::run_pipeline(&config);
    let mut expected_with_superseder = expected_after_create.clone();
    expected_with_superseder.push(SUPERSEDING_CIRCUIT_ID.to_string());
    expected_with_superseder.sort();
    assert_eq!(
        leaf_upsert_set(&provisioned),
        expected_with_superseder,
        "the provisioned device and the re-created circuit are both shaped"
    );
    assert!(
        store_has_circuit(&config),
        "publishing a static device does not edit the store — the supersede rule does"
    );
    assert!(
        publish_round(&mut state, &artifact) > 0,
        "the provisioned device must push a frame"
    );

    // 9. The reload now prunes: a provisioned device covers the address.
    drive_shaped_devices_reload("u3 provisioned-device-supersedes");
    wait_for(
        "the superseded circuit to leave the membership store",
        PRUNE_DEADLINE,
        || !store_has_circuit(&config),
    );
    let superseded = super::oracle1_live_tests::run_pipeline(&config);
    let supersede_diff = translate::semantic_diff(&superseded, state.applied.tx(), None);
    let supersede_delta = supersede_diff.delta.expect("the supersede publishes a delta");
    assert_eq!(
        leaf_removes(&supersede_delta),
        vec![CIRCUIT_ID.to_string()],
        "exactly one leaf Remove for the superseded circuit"
    );
    assert_eq!(
        deleted_ips(&supersede_delta),
        vec![(32, std::net::IpAddr::V4(CIRCUIT_IP))],
        "exactly one Del for the dynamic prefix"
    );
    assert!(
        leaf_upserts(&supersede_delta).is_empty(),
        "the provisioned device was already applied — the supersede only removes"
    );
    let supersede_frames = publish_round(&mut state, &artifact);
    assert!(
        supersede_frames > 0,
        "the supersede must push at least one frame"
    );
    let applied_after_supersede = state
        .applied
        .tx()
        .cloned()
        .expect("the ack fold leaves an applied world");
    let mut expected_applied_after_supersede = vec![STATIC_CIRCUIT_ID.to_string()];
    expected_applied_after_supersede.push(SUPERSEDING_CIRCUIT_ID.to_string());
    expected_applied_after_supersede.sort();
    assert_eq!(
        leaf_upsert_set(&applied_after_supersede),
        expected_applied_after_supersede,
        "the superseded circuit must leave the applied world"
    );
    assert!(
        !mapped_ips(&applied_after_supersede)
            .iter()
            .any(|(_, _, circuit_id)| circuit_id == CIRCUIT_ID),
        "the superseded circuit's mapping must leave the applied world"
    );
    assert_eq!(
        superseded
            .mapping_ops
            .iter()
            .filter(|op| matches!(
                op,
                DataplaneMappingOp::Map { prefix_len, ip, .. }
                    if *prefix_len == SUPERSEDING_PREFIX_LEN
                        && *ip == std::net::IpAddr::V4(SUPERSEDING_SUBNET)
            ))
            .count(),
        1,
        "the provisioned /24 is the mapping that superseded the circuit"
    );
    assert_eq!(
        state.applied.key(),
        Some(converged_key(&superseded).as_str()),
        "the applied belief converged on the provisioned world"
    );

    // A4: the machine-readable evidence, outside the repository.
    let evidence = serde_json::json!({
        "run_stamp_unix": lqos_utils::unix_time::unix_now().unwrap_or(0),
        "circuit_id": CIRCUIT_ID,
        "baseline": {
            "inner_ops": baseline.inner_ops.len(),
            "leaf_ops": baseline.leaf_ops.len(),
            "mapping_ops": baseline.mapping_ops.len(),
        },
        "churn": {
            "renewals": CHURN_RENEWALS,
            "generation_before": generation_before,
            "generation_after": generation_after,
            "store_bytes_unchanged": true,
            "store_rewritten": store_rewritten,
        },
        "create_delta": {
            "inner_ops": create_delta.inner_ops.len(),
            "leaf_upserts": leaf_upserts(&create_delta),
            "maps": mapped_ips(&create_delta).len(),
            "frames": create_frames,
            "applied_content_key_converged": create_converged,
        },
        "steady_state": {
            "broker_pending_after_second_step": state.pending.is_some(),
            "broker_op_delta": 0,
        },
        "expiry": {
            "path": "dynamic_circuits::remove_dynamic_circuit (RADIUS Stop handler)",
            "leaf_removes": leaf_removes(&expiry_delta),
            "prefix_dels": deleted_ips(&expiry_delta).len(),
            "frames": expiry_frames,
            "applied_leaf_ops": leaf_upsert_set(&applied_after_expiry).len(),
        },
        "reload_survival": {
            "path": "lqos_network_devices::request_reload_shaped_devices",
            "store_kept_circuit": reload_kept_circuit,
            "store_rewritten": reload_rewrote_store,
            "origins": {
                "dynamic_circuit": reload_origins_dynamic,
                "provisioned_circuit": reload_origins_provisioned,
            },
        },
        "static_supersede": {
            "superseding_circuit_id": SUPERSEDING_CIRCUIT_ID,
            "superseding_prefix": format!("{SUPERSEDING_SUBNET}/{SUPERSEDING_PREFIX_LEN}"),
            "leaf_removes": leaf_removes(&supersede_delta),
            "prefix_dels": deleted_ips(&supersede_delta).len(),
            "frames": supersede_frames,
            "applied_leaf_ops": leaf_upsert_set(&applied_after_supersede),
        },
    });
    std::fs::write(
        EVIDENCE_PATH,
        serde_json::to_string_pretty(&evidence).expect("serialize the evidence"),
    )
    .expect("write the lifecycle evidence");
    println!("u3 lifecycle evidence: {EVIDENCE_PATH}");
    println!("{}", serde_json::to_string(&evidence).expect("serialize"));

    unlink_test_segment(SEGMENT_NAME);
}
