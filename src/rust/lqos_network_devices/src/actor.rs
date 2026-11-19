use crate::dynamic::ProvisionedShapedRows;
use crate::dynamic_engine::{
    HostActivity, ObservedHost, PromotionEngine, PromotionPolicy, expired_dynamic_circuit_ids,
};
use crate::dynamic_store::{
    load_dynamic_circuits_from_disk, persist_dynamic_circuits_to_disk, persist_liveness_checkpoint,
    require_store_membership,
};
use crate::state;
use crate::{
    DaemonHooks, LoadedShapedDevices, LoadedShapedDevicesSource, ShapedDevicesSnapshotProvenance,
    load_network_json, load_shaped_devices_with_source,
};
use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Receiver, RecvTimeoutError, SendTimeoutError, Sender, bounded};
use lqos_dataplane_types::DATA_PLANE_ROOT_NAME;
use lqos_utils::normalize_circuit_id_key;
use once_cell::sync::OnceCell;
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, warn};

const RELOAD_RETRY_DELAY_MS: u64 = 500;
const RELOAD_ATTEMPTS: usize = 2;
const ACTOR_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const DYNAMIC_CIRCUITS_MAINTENANCE_TICK_SECONDS: u64 = 60;

static ACTOR_SENDER: OnceCell<Sender<NetworkDevicesCommand>> = OnceCell::new();

pub(crate) fn start_actor(hooks: Option<Arc<dyn DaemonHooks>>) -> Result<()> {
    if ACTOR_SENDER.get().is_some() {
        return Ok(());
    }

    let (tx, rx) = crossbeam_channel::bounded::<NetworkDevicesCommand>(64);
    let _ = ACTOR_SENDER.set(tx);

    std::thread::Builder::new()
        .name("lqos_network_devices".to_string())
        .spawn(move || actor_loop(rx, hooks))?;

    Ok(())
}

fn sender() -> Result<Sender<NetworkDevicesCommand>> {
    ACTOR_SENDER
        .get()
        .cloned()
        .ok_or_else(|| anyhow!("lqos_network_devices runtime actor is not running"))
}

fn send_actor_command(command: NetworkDevicesCommand, operation: &str) -> Result<()> {
    sender()?
        .send_timeout(command, ACTOR_COMMAND_TIMEOUT)
        .map_err(|err| match err {
            SendTimeoutError::Timeout(_) => {
                anyhow!("{operation} timed out while queueing lqos_network_devices actor command")
            }
            SendTimeoutError::Disconnected(_) => {
                anyhow!("{operation} failed because lqos_network_devices actor stopped")
            }
        })
}

fn receive_actor_reply<T>(reply_rx: Receiver<Result<T>>, operation: &str) -> Result<T> {
    match reply_rx.recv_timeout(ACTOR_COMMAND_TIMEOUT) {
        Ok(result) => result,
        Err(RecvTimeoutError::Timeout) => {
            Err(anyhow!("{operation} timed out waiting for actor reply"))
        }
        Err(RecvTimeoutError::Disconnected) => Err(anyhow!("{operation} reply channel closed")),
    }
}

pub(crate) fn request_reload_shaped_devices(reason: &str) -> Result<()> {
    let (reply_tx, reply_rx) = bounded(1);
    send_actor_command(
        NetworkDevicesCommand::ReloadShapedDevices {
            reason: reason.to_string(),
            reply: reply_tx,
        },
        "ShapedDevices reload",
    )?;
    receive_actor_reply(reply_rx, "ShapedDevices reload")
}

pub(crate) fn request_reload_network_json(reason: &str) -> Result<()> {
    let (reply_tx, reply_rx) = bounded(1);
    send_actor_command(
        NetworkDevicesCommand::ReloadNetworkJson {
            reason: reason.to_string(),
            reply: reply_tx,
        },
        "NetworkJson reload",
    )?;
    receive_actor_reply(reply_rx, "NetworkJson reload")
}

pub(crate) fn request_reload_dataplane_tree(reason: &str) -> Result<()> {
    let (reply_tx, reply_rx) = bounded(1);
    send_actor_command(
        NetworkDevicesCommand::ReloadDataplaneTree {
            reason: reason.to_string(),
            reply: reply_tx,
        },
        "Dataplane tree reload",
    )?;
    receive_actor_reply(reply_rx, "Dataplane tree reload")
}

pub(crate) fn apply_shaped_devices_snapshot(
    reason: &str,
    shaped: lqos_config::ConfigShapedDevices,
    provenance: ShapedDevicesSnapshotProvenance,
) -> Result<()> {
    let (reply_tx, reply_rx) = bounded(1);
    send_actor_command(
        NetworkDevicesCommand::ApplyShapedDevicesSnapshot {
            reason: reason.to_string(),
            shaped: Box::new(shaped),
            provenance,
            reply: reply_tx,
        },
        "Apply shaped devices",
    )?;
    receive_actor_reply(reply_rx, "Apply shaped devices")
}

/// K1 §1.4: report the bridge's active dynamic-circuit ids — the liveness
/// backstop for the RADIUS-interim-stall class. Ids are normalized here; the
/// state layer touches only matching dynamic circuits.
pub fn report_bridge_activity(circuit_ids: &[String]) {
    if circuit_ids.is_empty() {
        return;
    }
    let Some(sender) = ACTOR_SENDER.get().cloned() else {
        return;
    };
    let _ = sender.try_send(NetworkDevicesCommand::ReportBridgeActivity {
        circuit_ids: circuit_ids.to_vec(),
    });
}

pub(crate) fn upsert_dynamic_circuit(shaped_device: lqos_config::ShapedDevice) -> Result<()> {
    let (reply_tx, reply_rx) = bounded(1);
    send_actor_command(
        NetworkDevicesCommand::UpsertDynamicCircuit {
            shaped_device: Box::new(shaped_device),
            reply: reply_tx,
        },
        "Upsert dynamic circuit",
    )?;
    receive_actor_reply(reply_rx, "Upsert dynamic circuit")
}

pub(crate) fn remove_dynamic_circuit(circuit_id: &str) -> Result<bool> {
    let (reply_tx, reply_rx) = bounded(1);
    send_actor_command(
        NetworkDevicesCommand::RemoveDynamicCircuit {
            circuit_id: circuit_id.to_string(),
            reply: reply_tx,
        },
        "Remove dynamic circuit",
    )?;
    receive_actor_reply(reply_rx, "Remove dynamic circuit")
}

/// Dynamic-ranges H2: one RMW that removes every dynamic row covering any of
/// the new device's IPs, then upserts the new row by normalized id. Used by the
/// unshaped-host promotion (idempotent) and RADIUS (the in-place plan upgrade) so a
/// session-derived id can never mint a duplicate /32 row + Map.
pub(crate) fn replace_dynamic_circuit_by_ip(shaped_device: lqos_config::ShapedDevice) -> Result<()> {
    let (reply_tx, reply_rx) = bounded(1);
    send_actor_command(
        NetworkDevicesCommand::ReplaceDynamicCircuitByIp {
            shaped_device: Box::new(shaped_device),
            reply: reply_tx,
        },
        "Replace dynamic circuit by IP",
    )?;
    receive_actor_reply(reply_rx, "Replace dynamic circuit by IP")
}

/// Dynamic-ranges §7 + u11 (SIGNAL → DECISION split): the lqosd
/// bridge_analytics signal reports one bounded batch of per-host CUMULATIVE byte
/// counters for promotion candidates. A candidate is a host whose traffic a
/// range FALLBACK carries (u11: the pre-u4 signal lqosd lost), or a genuinely
/// unmapped host inside a configured range; each carries the counter source its
/// bytes came from. The actor's pure engine owns eligibility, the
/// sustained-activity window, the counter-source/reset rules, the cap, and the
/// store write.
///
/// This is the ONLY promotion input: there is no other scanner.
///
/// Side effects: sends a message to the runtime actor without waiting for a
/// reply; a full actor queue drops the batch (the next one diffs over a longer
/// interval).
pub fn report_promotion_candidate_activity(observed_at_millis: u64, samples: &[HostActivity]) {
    if samples.is_empty() {
        return;
    }
    let Some(sender) = ACTOR_SENDER.get().cloned() else {
        return;
    };
    // try_send: the analytics cycle must never block on the actor, and a dropped
    // batch is harmless (the next one diffs over a longer interval).
    let _ = sender.try_send(NetworkDevicesCommand::ReportPromotionCandidateActivity {
        observed_at_millis,
        samples: samples.to_vec(),
    });
}

enum NetworkDevicesCommand {
    ReloadShapedDevices {
        reason: String,
        reply: Sender<Result<()>>,
    },
    ReloadNetworkJson {
        reason: String,
        reply: Sender<Result<()>>,
    },
    ReloadDataplaneTree {
        reason: String,
        reply: Sender<Result<()>>,
    },
    ApplyShapedDevicesSnapshot {
        reason: String,
        shaped: Box<lqos_config::ConfigShapedDevices>,
        provenance: ShapedDevicesSnapshotProvenance,
        reply: Sender<Result<()>>,
    },
    ReportBridgeActivity {
        circuit_ids: Vec<String>,
    },
    UpsertDynamicCircuit {
        shaped_device: Box<lqos_config::ShapedDevice>,
        reply: Sender<Result<()>>,
    },
    RemoveDynamicCircuit {
        circuit_id: String,
        reply: Sender<Result<bool>>,
    },
    ReplaceDynamicCircuitByIp {
        shaped_device: Box<lqos_config::ShapedDevice>,
        reply: Sender<Result<()>>,
    },
    ReportPromotionCandidateActivity {
        observed_at_millis: u64,
        samples: Vec<HostActivity>,
    },
}

fn actor_loop(rx: Receiver<NetworkDevicesCommand>, hooks: Option<Arc<dyn DaemonHooks>>) {
    debug!("lqos_network_devices actor starting");
    let mut active_shaped_key = None;
    // T1: tracks whether an artifact-derived tree ever loaded (MEDIUM-4:
    // keep-last-good applies only after the first good load).
    let mut dataplane_tree_loaded = false;
    // Dynamic-ranges §7: the pure engine's sustained-activity windows + cap.
    let mut promotion_engine = PromotionEngine::default();
    // u12: one warn per rule whose attachment resolves to no node.
    let mut promotion_parent_warned: HashSet<String> = HashSet::new();

    if let Err(err) =
        reload_shaped_devices_inner("startup", hooks.as_deref(), &mut active_shaped_key)
    {
        warn!("Initial shaped-devices load failed: {err}");
    }
    if tree_source_is_legacy() {
        if let Err(err) = reload_network_json_inner("startup", hooks.as_deref()) {
            warn!("Initial network-json load failed: {err}");
        }
    } else if let Err(err) =
        reload_dataplane_tree_inner("startup", &mut dataplane_tree_loaded, hooks.as_deref())
    {
        warn!("Initial dataplane-tree load failed: {err}");
    }
    match lqos_config::load_config() {
        Ok(config) => {
            if let Err(err) = reload_dynamic_circuits_inner(&config, "startup") {
                warn!("Initial dynamic-circuits load failed: {err:?}");
            }
        }
        Err(err) => {
            // The store was never read, so this process does not hold the
            // membership; membership writes stay refused until a load works.
            state::note_dynamic_store_membership(state::DynamicStoreMembership::Failed);
            warn!("Initial dynamic-circuits load failed: {err:?}");
        }
    }
    prune_dynamic_circuits_superseded_by_shaped_devices_inner();

    let tick = crossbeam_channel::tick(Duration::from_secs(
        DYNAMIC_CIRCUITS_MAINTENANCE_TICK_SECONDS,
    ));

    loop {
        crossbeam_channel::select! {
            recv(rx) -> msg => {
                let command = match msg {
                    Ok(value) => value,
                    Err(_) => break,
                };

                match command {
                    NetworkDevicesCommand::ReloadShapedDevices { reason, reply } => {
                        let result = reload_shaped_devices_inner(
                            &reason,
                            hooks.as_deref(),
                            &mut active_shaped_key,
                        );
                        let _ = reply.send(result);
                    }
                    NetworkDevicesCommand::ReloadNetworkJson { reason, reply } => {
                        let result = if tree_source_is_legacy() {
                            reload_network_json_inner(&reason, hooks.as_deref())
                        } else {
                            // T1b: no implicit legacy parse in artifact mode.
                            reload_dataplane_tree_inner(
                                &reason,
                                &mut dataplane_tree_loaded,
                                hooks.as_deref(),
                            )
                        };
                        let _ = reply.send(result);
                    }
                    NetworkDevicesCommand::ReloadDataplaneTree { reason, reply } => {
                        let result = if tree_source_is_legacy() {
                            // The named legacy escape owns the tree; the
                            // artifact watcher event is a no-op there.
                            debug!(
                                "Ignoring dataplane-tree reload reason={reason}: legacy network.json tree mode is enabled"
                            );
                            Ok(())
                        } else {
                            reload_dataplane_tree_inner(
                                &reason,
                                &mut dataplane_tree_loaded,
                                hooks.as_deref(),
                            )
                        };
                        let _ = reply.send(result);
                    }
                    NetworkDevicesCommand::ApplyShapedDevicesSnapshot {
                        reason,
                        shaped,
                        provenance,
                        reply,
                    } => {
                        let key = shaped_devices_key_for_snapshot(provenance, &shaped);
                        // An applied snapshot is an EDIT of the world it replaces
                        // (the callers derive it from the live catalog), so the
                        // row origins carry forward; a full reload re-derives
                        // them from the fold.
                        let dynamic_origin_rows =
                            state::dynamic_origin_rows().as_ref().clone();
                        let result = publish_shaped_devices_if_changed(
                            &reason,
                            *shaped,
                            dynamic_origin_rows,
                            key,
                            hooks.as_deref(),
                            &mut active_shaped_key,
                        );
                        let _ = reply.send(result);
                    }
                    NetworkDevicesCommand::ReportBridgeActivity { circuit_ids } => {
                        let ids: std::collections::HashSet<String> = circuit_ids
                            .iter()
                            .map(|id| lqos_utils::normalize_circuit_id_key(id))
                            .collect();
                        // REV-C1(b): an unavailable clock is not a timestamp.
                        // Skipping the touch leaves the previous (real) stamp in
                        // place instead of zeroing it.
                        match lqos_utils::unix_time::unix_now() {
                            Ok(now) => {
                                crate::state::refresh_dynamic_circuits_last_seen_for_circuit_ids(
                                    &ids, now,
                                );
                            }
                            Err(err) => {
                                warn!(
                                    "Bridge-activity liveness skipped: the clock is unavailable ({err:?})"
                                );
                            }
                        }
                    }
                    NetworkDevicesCommand::UpsertDynamicCircuit {
                        shaped_device,
                        reply,
                    } => {
                        let result = lqos_config::load_config()
                            .context("load lqos config for dynamic circuit upsert")
                            .and_then(|config| {
                                upsert_dynamic_circuit_inner(&config, *shaped_device)
                            });
                        let _ = reply.send(result);
                    }
                    NetworkDevicesCommand::RemoveDynamicCircuit { circuit_id, reply } => {
                        let result = lqos_config::load_config()
                            .context("load lqos config for dynamic circuit removal")
                            .and_then(|config| {
                                remove_dynamic_circuit_inner(&config, &circuit_id)
                            });
                        let _ = reply.send(result);
                    }
                    NetworkDevicesCommand::ReplaceDynamicCircuitByIp {
                        shaped_device,
                        reply,
                    } => {
                        let result = lqos_config::load_config()
                            .context("load lqos config for dynamic circuit replace")
                            .and_then(|config| {
                                replace_dynamic_circuit_by_ip_inner(&config, *shaped_device)
                            });
                        let _ = reply.send(result);
                    }
                    NetworkDevicesCommand::ReportPromotionCandidateActivity {
                        observed_at_millis,
                        samples,
                    } => {
                        match lqos_config::load_config() {
                            Ok(config) => {
                                // The catalog is the coverage authority: a host an
                                // ordinary circuit already shapes is never promoted.
                                let catalog = crate::network_devices_catalog();
                                promote_sustained_hosts(
                                    &mut promotion_engine,
                                    &mut promotion_parent_warned,
                                    &config,
                                    &catalog,
                                    observed_at_millis,
                                    &samples,
                                );
                            }
                            Err(err) => {
                                warn!(
                                    "Unable to load lqos config for sustained-host promotion: {err:?}"
                                );
                            }
                        }
                    }
                }
            }
            recv(tick) -> _ => {
                match lqos_config::load_config() {
                    Ok(config) => {
                        if let Err(err) =
                            prune_expired_dynamic_circuits_inner(&config)
                        {
                            warn!("Dynamic-circuit TTL prune skipped: {err:?}");
                        }
                        // Liveness checkpoint: bounded interval, never per
                        // RADIUS interim. REV-C1(a): written only when this
                        // process actually holds a non-empty membership for a
                        // configured feature — an empty snapshot is not
                        // evidence (a failed startup load leaves it empty while
                        // the store file is not) and an unused feature must not
                        // grow a stray checkpoint file.
                        let snapshot = state::dynamic_circuits_snapshot();
                        if crate::dynamic_store::should_checkpoint_liveness(&config, &snapshot)
                            && let Err(err) = persist_liveness_checkpoint(&config, &snapshot)
                        {
                            warn!("Unable to checkpoint dynamic-circuit liveness: {err:?}");
                        }
                    }
                    Err(err) => {
                        warn!("Unable to load lqos config on maintenance tick: {err:?}");
                    }
                }
            }
        }
    }

    error!("lqos_network_devices actor stopped");
}

fn recompute_hashes(device: &mut lqos_config::ShapedDevice) {
    device.circuit_hash = lqos_utils::hash_to_i64(&device.circuit_id);
    device.device_hash = lqos_utils::hash_to_i64(&device.device_id);
    device.parent_hash = lqos_utils::hash_to_i64(&device.parent_node);
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ShapedDevicesProvenanceKey {
    RuntimeTopology {
        shaping_generation: String,
    },
    TopologyImport {
        devices_fingerprint: u64,
    },
    ExternalSnapshot {
        devices_fingerprint: u64,
    },
}

#[derive(Eq, Hash, Ord, PartialEq, PartialOrd)]
struct ShapedDeviceSemanticKey {
    circuit_id: String,
    circuit_name: String,
    device_id: String,
    device_name: String,
    parent_node: String,
    parent_node_id: Option<String>,
    anchor_node_id: Option<String>,
    mac: String,
    ipv4: Vec<(std::net::Ipv4Addr, u32)>,
    ipv6: Vec<(std::net::Ipv6Addr, u32)>,
    download_min_mbps_bits: u32,
    upload_min_mbps_bits: u32,
    download_max_mbps_bits: u32,
    upload_max_mbps_bits: u32,
    comment: String,
    sqm_override: Option<String>,
    circuit_hash: i64,
    device_hash: i64,
    parent_hash: i64,
}

fn shaped_device_semantic_key(device: &lqos_config::ShapedDevice) -> ShapedDeviceSemanticKey {
    let mut ipv4 = device.ipv4.clone();
    ipv4.sort_unstable();
    let mut ipv6 = device.ipv6.clone();
    ipv6.sort_unstable();

    ShapedDeviceSemanticKey {
        circuit_id: device.circuit_id.clone(),
        circuit_name: device.circuit_name.clone(),
        device_id: device.device_id.clone(),
        device_name: device.device_name.clone(),
        parent_node: device.parent_node.clone(),
        parent_node_id: device.parent_node_id.clone(),
        anchor_node_id: device.anchor_node_id.clone(),
        mac: device.mac.clone(),
        ipv4,
        ipv6,
        download_min_mbps_bits: device.download_min_mbps.to_bits(),
        upload_min_mbps_bits: device.upload_min_mbps.to_bits(),
        download_max_mbps_bits: device.download_max_mbps.to_bits(),
        upload_max_mbps_bits: device.upload_max_mbps.to_bits(),
        comment: device.comment.clone(),
        sqm_override: device.sqm_override.clone(),
        circuit_hash: device.circuit_hash,
        device_hash: device.device_hash,
        parent_hash: device.parent_hash,
    }
}

fn shaped_devices_fingerprint(devices: &[lqos_config::ShapedDevice]) -> u64 {
    let mut sorted = devices
        .iter()
        .map(shaped_device_semantic_key)
        .collect::<Vec<_>>();
    sorted.sort_unstable();
    let mut hasher = DefaultHasher::new();
    for key in sorted {
        key.hash(&mut hasher);
    }
    hasher.finish()
}

fn shaped_devices_key_for_loaded(loaded: &LoadedShapedDevices) -> ShapedDevicesProvenanceKey {
    match &loaded.source {
        LoadedShapedDevicesSource::RuntimeTopology {
            shaping_generation,
            ..
        } => ShapedDevicesProvenanceKey::RuntimeTopology {
            shaping_generation: shaping_generation.clone(),
        },
        LoadedShapedDevicesSource::TopologyImport => {
            ShapedDevicesProvenanceKey::TopologyImport {
                devices_fingerprint: shaped_devices_fingerprint(&loaded.shaped.devices),
            }
        }
    }
}

fn shaped_devices_key_for_snapshot(
    provenance: ShapedDevicesSnapshotProvenance,
    shaped: &lqos_config::ConfigShapedDevices,
) -> ShapedDevicesProvenanceKey {
    match provenance {
        ShapedDevicesSnapshotProvenance::ExternalSnapshot => {
            ShapedDevicesProvenanceKey::ExternalSnapshot {
                devices_fingerprint: shaped_devices_fingerprint(&shaped.devices),
            }
        }
    }
}

/// Loads the membership store and publishes it.
///
/// A load FAILURE is not an empty membership: the snapshot is left untouched
/// (publishing an empty one would erase the live view) and the process records
/// that it does not hold the membership, so membership writes stay refused until
/// a load succeeds.
pub(crate) fn reload_dynamic_circuits_inner(
    config: &lqos_config::Config,
    reason: &str,
) -> Result<()> {
    let circuits = match load_dynamic_circuits_from_disk(config) {
        Ok(circuits) => circuits,
        Err(err) => {
            state::note_dynamic_store_membership(state::DynamicStoreMembership::Failed);
            return Err(err);
        }
    };
    debug!("Loaded {} dynamic circuits reason={reason}", circuits.len());
    state::note_dynamic_store_membership(state::DynamicStoreMembership::Loaded);
    state::publish_dynamic_circuits_snapshot(circuits);
    Ok(())
}

pub(crate) fn prune_expired_dynamic_circuits_inner(
    config: &lqos_config::Config,
) -> Result<()> {
    require_store_membership(config)?;
    let Some(dynamic_cfg) = config.dynamic_circuits.as_ref() else {
        return Ok(());
    };
    if !dynamic_cfg.enabled {
        return Ok(());
    }
    let ttl_seconds = dynamic_cfg.ttl_seconds;
    if ttl_seconds == 0 {
        return Ok(());
    }

    let Ok(now_unix) = lqos_utils::unix_time::unix_now() else {
        return Ok(());
    };

    let snapshot = state::dynamic_circuits_snapshot();
    if snapshot.is_empty() {
        return Ok(());
    }

    let mut expired_ids = expired_dynamic_circuit_ids(snapshot.as_ref(), now_unix, ttl_seconds);
    if expired_ids.is_empty() {
        return Ok(());
    }
    expired_ids.sort();
    expired_ids.dedup();

    let expired_keys: HashSet<String> = expired_ids
        .iter()
        .map(|id| normalize_circuit_id_key(id))
        .collect();
    let updated: Vec<_> = snapshot
        .iter()
        .filter(|circuit| {
            let key = normalize_circuit_id_key(&circuit.shaped.circuit_id);
            !expired_keys.contains(&key)
        })
        .cloned()
        .collect();

    persist_dynamic_circuits_to_disk(config, &updated)
        .context("persist dynamic circuits after TTL pruning")?;
    state::publish_dynamic_circuits_snapshot(updated);
    debug!(
        "Pruned {} expired dynamic circuits ttl_seconds={ttl_seconds}",
        expired_ids.len()
    );

    Ok(())
}

pub(crate) fn upsert_dynamic_circuit_inner(
    config: &lqos_config::Config,
    mut shaped_device: lqos_config::ShapedDevice,
) -> Result<()> {
    require_store_membership(config)?;
    recompute_hashes(&mut shaped_device);
    let now_unix = lqos_utils::unix_time::unix_now().context("get unix time")?;

    let snapshot = state::dynamic_circuits_snapshot();
    let mut updated = snapshot.as_ref().clone();
    let key = normalize_circuit_id_key(&shaped_device.circuit_id);

    if let Some(pos) = updated
        .iter()
        .position(|c| normalize_circuit_id_key(&c.shaped.circuit_id) == key)
    {
        updated[pos].shaped = shaped_device;
        updated[pos].last_seen_unix = now_unix;
    } else {
        updated.push(crate::DynamicCircuit {
            shaped: shaped_device,
            last_seen_unix: now_unix,
        });
    }

    // The store write is content-deduped, so a liveness-only renewal writes
    // nothing here; liveness reaches the checkpoint on the maintenance tick.
    persist_dynamic_circuits_to_disk(config, &updated)
        .context("persist dynamic circuits to disk")?;
    state::publish_dynamic_circuits_snapshot(updated);
    Ok(())
}

/// The store half of `remove_dynamic_circuit`; exposed to the crate tests so
/// removal's store + checkpoint behaviour can be asserted without an actor.
pub(crate) fn remove_dynamic_circuit_inner(
    config: &lqos_config::Config,
    circuit_id: &str,
) -> Result<bool> {
    require_store_membership(config)?;
    let snapshot = state::dynamic_circuits_snapshot();
    if snapshot.is_empty() {
        return Ok(false);
    }

    let key = normalize_circuit_id_key(circuit_id);
    let updated: Vec<_> = snapshot
        .iter()
        .filter(|c| normalize_circuit_id_key(&c.shaped.circuit_id) != key)
        .cloned()
        .collect();

    if updated.len() == snapshot.len() {
        return Ok(false);
    }

    persist_dynamic_circuits_to_disk(config, &updated)
        .context("persist dynamic circuits to disk")?;
    state::publish_dynamic_circuits_snapshot(updated);
    Ok(true)
}

/// H2 pure core: drop every dynamic row covering any of `shaped_device`'s
/// IPs, then upsert `shaped_device` by normalized circuit id. This is the
/// uniqueness guarantee — a session-derived RADIUS id and an IP-derived
/// promotion id can never coexist as two rows (and therefore two /32 Maps)
/// for the same subscriber IP. Exposed to the crate tests.
pub(crate) fn merge_dynamic_circuits_replacing_by_ip(
    snapshot: &[crate::DynamicCircuit],
    shaped_device: lqos_config::ShapedDevice,
    now_unix: u64,
) -> Vec<crate::DynamicCircuit> {
    let new_ips = device_ips(&shaped_device);
    let mut updated: Vec<_> = snapshot
        .iter()
        .filter(|c| new_ips.is_empty() || device_ips(&c.shaped).is_disjoint(&new_ips))
        .cloned()
        .collect();
    let key = normalize_circuit_id_key(&shaped_device.circuit_id);
    if let Some(pos) = updated
        .iter()
        .position(|c| normalize_circuit_id_key(&c.shaped.circuit_id) == key)
    {
        updated[pos].shaped = shaped_device;
        updated[pos].last_seen_unix = now_unix;
    } else {
        updated.push(crate::DynamicCircuit {
            shaped: shaped_device,
            last_seen_unix: now_unix,
        });
    }
    updated
}

/// H2: one RMW — drop every dynamic row that covers any of `shaped_device`'s
/// IPs, then upsert it by normalized circuit id.
///
/// This is the store half of `replace_dynamic_circuit_by_ip` (the bus path
/// RADIUS reaches); exposed to the crate tests as well.
///
/// The store write is content-deduped: a RADIUS Start/Interim that only renews
/// liveness for an already-stored plan writes no file at all.
pub(crate) fn replace_dynamic_circuit_by_ip_inner(
    config: &lqos_config::Config,
    mut shaped_device: lqos_config::ShapedDevice,
) -> Result<()> {
    require_store_membership(config)?;
    recompute_hashes(&mut shaped_device);
    let now_unix = lqos_utils::unix_time::unix_now().context("get unix time")?;
    let snapshot = state::dynamic_circuits_snapshot();
    let updated = merge_dynamic_circuits_replacing_by_ip(&snapshot, shaped_device, now_unix);
    persist_dynamic_circuits_to_disk(config, &updated)
        .context("persist dynamic circuits to disk")?;
    state::publish_dynamic_circuits_snapshot(updated);
    Ok(())
}

fn device_ips(device: &lqos_config::ShapedDevice) -> HashSet<std::net::IpAddr> {
    device
        .ipv4
        .iter()
        .map(|(ip, _)| std::net::IpAddr::V4(*ip))
        .chain(
            device
                .ipv6
                .iter()
                .map(|(ip, _)| std::net::IpAddr::V6(*ip)),
        )
        .collect()
}

/// Dynamic-ranges §7 + u11 (DECISION + WRITE): folds one pushed candidate batch
/// into the pure engine and provisions what it decides.
///
/// The engine owns every decision — eligibility, the sustained-activity window,
/// counter sources/resets, the cap. This shell supplies only what a pure engine
/// cannot know: the coverage catalog, the operator's ignore list, the resolved
/// `attach_to` parent and the store write.
///
/// u11 A5: coverage is rechecked HERE, from the published world, not trusted from
/// the sender: a host an ordinary circuit already shapes (a provisioned device or
/// an existing dynamic row) is never promoted, and an address the operator's
/// `ip_ranges.ignore_subnets` names is never promoted even when no circuit shapes
/// it. The fold-synthesized range FALLBACK still owns nothing — it is the very
/// source of the u11 signal.
///
/// u12: the row's parent is a resolved reference or the row is not written at all
/// (see [`promotion_parent_node`]); `warned_parents` keeps that warn to one line
/// per rule for the process lifetime. u13: a node is carried by NAME only — the
/// id the loaded `network.json` view reports is not the exported tree's id.
/// u15: a blank/`Root` attachment writes the fold's sentinel itself and consults
/// no view, so only a NAMED attachment can reach the warn below.
pub(crate) fn promote_sustained_hosts(
    engine: &mut PromotionEngine,
    warned_parents: &mut HashSet<String>,
    config: &lqos_config::Config,
    catalog: &crate::NetworkDevicesCatalog,
    observed_at_millis: u64,
    samples: &[HostActivity],
) {
    let Some(dynamic_cfg) = config.dynamic_circuits.as_ref() else {
        return;
    };
    if !dynamic_cfg.enabled || !dynamic_cfg.enable_unknown_ip_promotion {
        return;
    }

    // Coverage authority for promotion (REV-D2/REV-T1): ONE named predicate built
    // from the published world. A PROVISIONED row owns its address; the overlay
    // store's own rows own theirs (a promotion that has not reached the shaped
    // artifact yet); the fold-synthesized range FALLBACK owns nothing — it sits in
    // the shaped snapshot with the dynamic-origin marker and covers its whole rule
    // prefix, so counting it would stop promotion for every host inside a
    // catch-all range (the u5 gate). The combined catalog's own lookup cannot be
    // used alone here: it answers "does ANY row shape this address", which is
    // exactly the question the fallback makes uninformative.
    let origins = state::dynamic_origin_rows();
    let coverage =
        crate::dynamic::PublishedCoverage::new(catalog.shaped_devices(), origins.as_ref());
    // u11 A5: the operator's management exclusions outrank promotion. `None` means
    // a configured list that no entry described readably — fail CLOSED for the
    // whole pass rather than promote an address that list may have named.
    let ignores = promotion_ignore_table(config);
    if ignores.is_none() {
        debug!(
            "promotion skipped this pass: ip_ranges.ignore_subnets is configured but no entry parses"
        );
    }
    let observed: Vec<ObservedHost> = samples
        .iter()
        .map(|sample| {
            let address = lqos_utils::XdpIpAddress::from_ip(sample.ip);
            ObservedHost {
                ip: sample.ip,
                cumulative_bytes: sample.cumulative_bytes,
                source: sample.source,
                // An already-promoted host reads as covered through the store row,
                // so it cannot consume another promotion slot; a provisioned host
                // never promotes at all; an ignored host is never promotion
                // material whatever carried its bytes.
                ineligible: ignores.is_none()
                    || coverage.promotion_covered(catalog, &address)
                    || ignores
                        .as_ref()
                        .is_some_and(|table| table.longest_match(address.as_ip()).is_some()),
            }
        })
        .collect();
    let policy = PromotionPolicy {
        ranges: &dynamic_cfg.ranges,
        min_bytes_per_second: dynamic_cfg.promotion_min_bytes_per_second,
        window_seconds: dynamic_cfg.promotion_window_seconds,
        cap_per_range_per_minute: dynamic_cfg.promotion_cap_per_range_per_minute,
    };

    for promotion in engine.observe(observed_at_millis, &policy, &observed) {
        let range_id = promotion.rule.name.trim();
        // u12: a promoted row is a STORE row, and the store fold defers any row
        // whose parent NAME lands no exported node. Writing an unresolvable
        // parent would provision a row that defers on every publish (the live u11
        // run did exactly that), so no row is written and the operator is told
        // why — once per rule, because the engine would otherwise re-decide the
        // same host every window. u15: only a NAMED attachment can get here; the
        // blank/`Root` convention is always writable.
        let Some(parent_node) = promotion_parent_node(promotion.rule.attach_to.trim()) else {
            if warned_parents.insert(range_id.to_string()) {
                warn!(
                    "Promotion of {} in range '{range_id}' skipped: attach_to {:?} names no node in network.json, so the row would defer on every publish; no row is written (a blank or \"Root\" attach_to means the root and is always written)",
                    promotion.ip, promotion.rule.attach_to
                );
            }
            continue;
        };
        let label = format!("dynamic-range:{range_id}:{}", promotion.ip);
        let device =
            shaped_device_from_unknown_ip(&label, &parent_node, promotion.ip, &promotion.rule);
        match replace_dynamic_circuit_by_ip_inner(config, device) {
            Ok(()) => {
                debug!(
                    "Promoted sustained host {} into range '{range_id}'",
                    promotion.ip
                );
            }
            Err(err) => warn!(
                "Unable to persist dynamic-range promotion for {} in range '{range_id}': {err:?}",
                promotion.ip
            ),
        }
    }
}

/// The `ip_ranges.ignore_subnets` gate for promotion (u11 A5).
///
/// `Some(table)` is the operator's parsed exclusion set (empty when they
/// configured none). `None` means entries WERE configured and none parsed: the
/// u8 lenient parser cannot say what the operator excluded, so promotion fails
/// closed rather than enrol a management address the unreadable line named.
///
/// Side effects: `lqos_config` logs one `warn!` per malformed entry (its own
/// once-per-process gate); this function adds a `debug!` only on the fail-closed
/// path.
fn promotion_ignore_table(
    config: &lqos_config::Config,
) -> Option<ip_network_table::IpNetworkTable<bool>> {
    let Ok(table) = config.ip_ranges.ignored_network_table() else {
        // No error arm today; if one ever appears, fail closed rather than
        // promote against an unknown exclusion set.
        return None;
    };
    if !config.ip_ranges.ignore_subnets.is_empty() && table.is_empty() {
        return None;
    }
    Some(table)
}

/// The node NAME a PROMOTED row is written under (u12, u13, u15).
///
/// A promoted row is a STORE row, so it is folded by
/// `fold_dynamic_circuits_into_devices`, which resolves the row's
/// `(parent_node, parent_node_id)` against the EXPORTED EFFECTIVE tree and
/// DEFERS anything that lands no real node. The id `network.json`'s legacy
/// parser reports for a node is NOT that tree's id (u13: the live u8 run
/// deferred a row with a correct parent name because an unprovable id rode
/// along), so only a name ever leaves this function.
///
/// - blank `attach_to` (or the fold's `Root` sentinel) -> the fold's OWN root
///   convention and nothing else: the sentinel name, no id. The store fold
///   resolves that sentinel to the world's ROOT NODE (`u15` A2), so the value
///   does not depend on any view of the loaded topology — which is exactly what
///   the two live runs disproved: the old "first node after the sentinel" guess
///   ran over a store that also carries SYNTHESIZED CIRCUIT nodes, and answered
///   `dynamic-range:range-24` — a circuit id, so the row deferred forever;
/// - a named `attach_to` -> that node's canonical NAME, by name/alias precedence
///   in the loaded `network.json` (never by id, per u13);
/// - `None` -> no deterministic node, so the caller writes NO row and warns once.
///
/// Side effects: a NAMED attachment reads the process-global `network.json`
/// snapshot (one read lock). A blank/root attachment reads nothing.
fn promotion_parent_node(attach_to: &str) -> Option<String> {
    let trimmed = attach_to.trim();
    if trimmed.is_empty() || is_dataplane_root_name(trimmed) {
        return Some(DATA_PLANE_ROOT_NAME.to_string());
    }
    state::with_network_json_read(|net_json| {
        let nodes = net_json.get_nodes_when_ready();
        let resolved = crate::resolve_parent_node_reference_in_nodes(nodes, trimmed, None)?;
        let name = resolved.name;
        (!name.trim().is_empty()).then_some(name)
    })
}

/// Whether an attachment names the fold's root sentinel.
///
/// NAME only, by construction: the sentinel carries no id (`u13`), so an
/// id-shaped check could only match a node the loaded view synthesized.
fn is_dataplane_root_name(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case(DATA_PLANE_ROOT_NAME)
}

/// One promoted host's store row (u11): the rule's rates, the host `/32`
/// (`/128`) mapping, and the parent reference the rule supplies.
///
/// u13: `parent_node_id` is ALWAYS `None`, and `anchor_node_id` stays `None`. The
/// fold proves a fallback-supplied id against the EXPORTED EFFECTIVE tree
/// (`fallback_identity_resolves`), and the id the legacy `network.json` parser
/// reports for a node is not that tree's id, so an id here is unprovable: the
/// fold defers the whole row (`shaping_deferred`) even when the name is right —
/// the live u8 run's `has an unresolvable fallback parent/anchor id`. The
/// name-only shape is the one the range fallback uses and the one the fold
/// resolves by name, so the id is dropped rather than guessed.
///
/// u15: the name may be the fold's `Root` sentinel, which the fold resolves to
/// the world's root node — that is the blank-attachment convention, and carrying
/// it here is what keeps the write independent of any topology view.
fn shaped_device_from_unknown_ip(
    label: &str,
    parent_node_name: &str,
    ip: IpAddr,
    rule: &lqos_config::DynamicCircuitRangeRule,
) -> lqos_config::ShapedDevice {
    let mut device = lqos_config::ShapedDevice {
        circuit_id: label.to_string(),
        circuit_name: label.to_string(),
        device_id: label.to_string(),
        device_name: label.to_string(),
        // u13: a node NAME (or the fold's `Root` sentinel, u15), never a circuit
        // id, and never an id the exported tree cannot verify.
        parent_node: parent_node_name.to_string(),
        parent_node_id: None,
        anchor_node_id: None,
        download_min_mbps: rule.download_min_mbps,
        upload_min_mbps: rule.upload_min_mbps,
        download_max_mbps: rule.download_max_mbps,
        upload_max_mbps: rule.upload_max_mbps,
        ..Default::default()
    };

    match ip {
        IpAddr::V4(ip) => device.ipv4.push((ip, 32)),
        IpAddr::V6(ip) => device.ipv6.push((ip, 128)),
    }

    device
}

fn publish_shaped_devices_if_changed(
    reason: &str,
    shaped: lqos_config::ConfigShapedDevices,
    dynamic_origin_rows: lqos_utils::DynamicOriginRows,
    key: ShapedDevicesProvenanceKey,
    hooks: Option<&dyn DaemonHooks>,
    active_key: &mut Option<ShapedDevicesProvenanceKey>,
) -> Result<()> {
    if active_key.as_ref() == Some(&key) {
        debug!("Skipping unchanged shaped-devices snapshot reason={reason}");
        return Ok(());
    }

    debug!("Publishing shaped-devices snapshot reason={reason}");
    state::publish_shaped_devices(shaped, dynamic_origin_rows);
    *active_key = Some(key);
    prune_dynamic_circuits_superseded_by_shaped_devices_inner();
    if let Some(hooks) = hooks {
        hooks.on_shaped_devices_updated();
    }
    Ok(())
}

fn reload_shaped_devices_inner(
    reason: &str,
    hooks: Option<&dyn DaemonHooks>,
    active_key: &mut Option<ShapedDevicesProvenanceKey>,
) -> Result<()> {
    for attempt in 1..=RELOAD_ATTEMPTS {
        match load_shaped_devices_with_source() {
            Ok(loaded) => {
                debug!("Loaded shaped devices reason={reason}");
                let key = shaped_devices_key_for_loaded(&loaded);
                return publish_shaped_devices_if_changed(
                    reason,
                    loaded.shaped,
                    loaded.dynamic_origin_rows,
                    key,
                    hooks,
                    active_key,
                );
            }
            Err(err) if attempt < RELOAD_ATTEMPTS => {
                warn!(
                    "ShapedDevices reload reason={reason} attempt {attempt}/{} failed: {err}. Retrying after {} ms.",
                    RELOAD_ATTEMPTS, RELOAD_RETRY_DELAY_MS
                );
                std::thread::sleep(Duration::from_millis(RELOAD_RETRY_DELAY_MS));
            }
            Err(err) => {
                warn!(
                    "ShapedDevices reload reason={reason} failed after {} attempts: {err}. Keeping last-known-good snapshot with {} devices.",
                    RELOAD_ATTEMPTS,
                    state::shaped_devices_snapshot().devices.len()
                );
                return Err(err);
            }
        }
    }
    unreachable!("reload loop must return");
}

fn prune_dynamic_circuits_superseded_by_shaped_devices_inner() {
    let Ok(config) = lqos_config::load_config() else {
        return;
    };
    let catalog = state::shaped_devices_catalog();
    let snapshot = state::dynamic_circuits_snapshot();
    if snapshot.is_empty() {
        return;
    }
    let provisioned = ProvisionedShapedRows::new(&catalog, &state::dynamic_origin_rows());

    let mut removed_keys: HashSet<String> = HashSet::new();
    for circuit in snapshot.iter() {
        if !provisioned.is_superseded_by(circuit) {
            continue;
        }
        let circuit_id = circuit.shaped.circuit_id.trim();
        if circuit_id.is_empty() {
            continue;
        }
        removed_keys.insert(normalize_circuit_id_key(circuit_id));
    }

    if removed_keys.is_empty() {
        return;
    }

    let updated: Vec<_> = snapshot
        .iter()
        .filter(|circuit| {
            let circuit_id = circuit.shaped.circuit_id.trim();
            if circuit_id.is_empty() {
                return true;
            }
            let key = normalize_circuit_id_key(circuit_id);
            !removed_keys.contains(&key)
        })
        .cloned()
        .collect();

    if let Err(err) = persist_dynamic_circuits_to_disk(&config, &updated) {
        warn!("Unable to persist dynamic circuits after shaped-devices conflict prune: {err:?}");
    }
    state::publish_dynamic_circuits_snapshot(updated);
    debug!(
        "Pruned {} dynamic circuits superseded by ShapedDevices.csv changes",
        removed_keys.len()
    );
}

fn reload_network_json_inner(reason: &str, hooks: Option<&dyn DaemonHooks>) -> Result<()> {
    for attempt in 1..=RELOAD_ATTEMPTS {
        match load_network_json() {
            Ok(net_json) => {
                debug!("Loaded network json reason={reason}");
                state::publish_network_json(net_json);
                if let Some(hooks) = hooks {
                    hooks.on_network_json_updated();
                }
                return Ok(());
            }
            Err(err) if attempt < RELOAD_ATTEMPTS => {
                warn!(
                    "NetworkJson reload reason={reason} attempt {attempt}/{} failed: {err}. Retrying after {} ms.",
                    RELOAD_ATTEMPTS, RELOAD_RETRY_DELAY_MS
                );
                std::thread::sleep(Duration::from_millis(RELOAD_RETRY_DELAY_MS));
            }
            Err(err) => {
                warn!(
                    "NetworkJson reload reason={reason} failed after {} attempts: {err}. Keeping last-known-good snapshot.",
                    RELOAD_ATTEMPTS
                );
                return Err(err);
            }
        }
    }
    unreachable!("reload loop must return");
}

/// T1 source selection (t1_tree_repoint_derivation T1b): the v3 compiler
/// artifact is the only tree source unless the operator enabled the named
/// legacy escape (`legacy_network_json_tree = true`). No implicit
/// fallback to a stale `network.json` — that would recreate the drift
/// class the artifact retires.
fn tree_source_is_legacy() -> bool {
    lqos_config::legacy_network_json_tree_enabled()
}

/// The MEDIUM-4 degrade decision, extracted pure for the gate tests:
/// what should the tree store do given one artifact read outcome?
pub(crate) enum TreeReadOutcome {
    /// Parse + publish the artifact payload.
    Build(String),
    /// Missing/unreadable/unparseable AFTER a good load: keep the
    /// last-known-good tree (a transient mid-write read must not blank a
    /// working UI). `why` is logged.
    KeepLastGood(&'static str),
    /// Missing/unparseable and NOTHING has ever loaded: publish the
    /// structure-less store (honest-empty posture), with a WARN.
    PublishStructureless(&'static str),
}

fn decide_tree_read(
    read: std::io::Result<String>,
    parse: impl FnOnce(&str) -> Result<(), serde_json::Error>,
    ever_loaded: bool,
) -> TreeReadOutcome {
    let raw = match read {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return if ever_loaded {
                TreeReadOutcome::KeepLastGood("artifact missing after a good load")
            } else {
                TreeReadOutcome::PublishStructureless("artifact not present yet")
            };
        }
        Err(_) => {
            return if ever_loaded {
                TreeReadOutcome::KeepLastGood("artifact unreadable after a good load")
            } else {
                TreeReadOutcome::PublishStructureless("artifact unreadable")
            };
        }
    };
    match parse(&raw) {
        Ok(()) => TreeReadOutcome::Build(raw),
        Err(_) if ever_loaded => {
            TreeReadOutcome::KeepLastGood("artifact unparseable after a good load")
        }
        Err(_) => TreeReadOutcome::PublishStructureless("artifact unparseable at first load"),
    }
}

/// Populates the tree store from the v3 compiler artifact
/// (`state/topology/dataplane_transaction.json`).
///
/// Degrade posture (MEDIUM-4): a missing/unparseable artifact KEEPS the
/// last-known-good tree once one has loaded; only the never-loaded case
/// publishes a structure-less root-only store, with one named WARN.
fn reload_dataplane_tree_inner(
    reason: &str,
    ever_loaded: &mut bool,
    hooks: Option<&dyn DaemonHooks>,
) -> Result<()> {
    let config = lqos_config::load_config().context("Unable to load /etc/lqos.conf")?;
    // T5 rung 1: the tree source is the E2 operator view (the T1
    // transaction repoint is retired — never both transports live).
    let path = lqos_config::topology_operator_view_path(&config);
    let read = std::fs::read_to_string(&path);
    let probe = |raw: &str| {
        serde_json::from_str::<lqos_config::TopologyOperatorViewFile>(raw).map(|_| ())
    };
    let outcome = decide_tree_read(read, probe, *ever_loaded);
    let tree = match outcome {
        TreeReadOutcome::KeepLastGood(why) => {
            warn!(
                "{why}; keeping last-known-good tree (reason={reason})"
            );
            return Ok(());
        }
        TreeReadOutcome::PublishStructureless(why) => {
            warn!(
                "{why}; dashboard tree starts structure-less until a good publish (reason={reason})"
            );
            *ever_loaded = true;
            crate::dataplane_tree::structureless_tree()
        }
        TreeReadOutcome::Build(raw) => {
            let view: lqos_config::TopologyOperatorViewFile = serde_json::from_str(&raw)
                .map_err(|err| anyhow!("Unable to parse {}: {err}", path.display()))?;
            let device_parents: Vec<String> = state::shaped_devices_snapshot()
                .devices
                .iter()
                .map(|device| device.parent_node.clone())
                .collect();
            let build = crate::dataplane_tree::build_tree_from_operator_view(&view, &device_parents);
            if !build.skipped_nodes.is_empty() {
                warn!(
                    "Dataplane tree skipped {} node(s) (root-like/dangling/unreachable): {:?} (reason={reason})",
                    build.skipped_nodes.len(),
                    build.skipped_nodes
                );
            }
            if !build.divergent_parent_nodes.is_empty() {
                warn!(
                    "Dataplane tree identity divergence: {} device parent_node(s) match no artifact node (those devices render zero live data): {:?} (reason={reason})",
                    build.divergent_parent_nodes.len(),
                    build.divergent_parent_nodes
                );
            }
            *ever_loaded = true;
            debug!(
                "Dataplane tree populated from {} (reason={reason})",
                path.display()
            );
            build.tree
        }
    };
    state::publish_network_json(tree);
    if let Some(hooks) = hooks {
        hooks.on_network_json_updated();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::shaped_devices_fingerprint;

    fn shaped_device(circuit_id: &str, device_id: &str) -> lqos_config::ShapedDevice {
        let mut device = lqos_config::ShapedDevice {
            circuit_id: circuit_id.to_string(),
            circuit_name: circuit_id.to_string(),
            device_id: device_id.to_string(),
            device_name: device_id.to_string(),
            parent_node: "Tower A".to_string(),
            ipv4: vec![("192.0.2.1".parse().expect("valid IP"), 32)],
            download_min_mbps: 10.0,
            upload_min_mbps: 10.0,
            download_max_mbps: 100.0,
            upload_max_mbps: 100.0,
            ..lqos_config::ShapedDevice::default()
        };
        device.circuit_hash = lqos_utils::hash_to_i64(&device.circuit_id);
        device.device_hash = lqos_utils::hash_to_i64(&device.device_id);
        device.parent_hash = lqos_utils::hash_to_i64(&device.parent_node);
        device
    }

    #[test]
    fn shaped_devices_fingerprint_ignores_row_order() {
        let first = vec![
            shaped_device("circuit-1", "device-1"),
            shaped_device("circuit-2", "device-2"),
        ];
        let second = vec![
            shaped_device("circuit-2", "device-2"),
            shaped_device("circuit-1", "device-1"),
        ];

        assert_eq!(
            shaped_devices_fingerprint(&first),
            shaped_devices_fingerprint(&second)
        );
    }

    #[test]
    fn shaped_devices_fingerprint_ignores_address_order() {
        let first = vec![shaped_device("circuit-1", "device-1")];
        let mut second = vec![shaped_device("circuit-1", "device-1")];
        second[0].ipv4 = vec![
            ("192.0.2.2".parse().expect("valid IPv4"), 32),
            ("192.0.2.1".parse().expect("valid IPv4"), 32),
        ];
        second[0].ipv6 = vec![
            ("2001:db8::2".parse().expect("valid IPv6"), 128),
            ("2001:db8::1".parse().expect("valid IPv6"), 128),
        ];
        let mut first = first;
        first[0].ipv4 = vec![
            ("192.0.2.1".parse().expect("valid IPv4"), 32),
            ("192.0.2.2".parse().expect("valid IPv4"), 32),
        ];
        first[0].ipv6 = vec![
            ("2001:db8::1".parse().expect("valid IPv6"), 128),
            ("2001:db8::2".parse().expect("valid IPv6"), 128),
        ];

        assert_eq!(
            shaped_devices_fingerprint(&first),
            shaped_devices_fingerprint(&second)
        );
    }

    #[test]
    fn shaped_devices_fingerprint_detects_payload_changes() {
        let first = vec![shaped_device("circuit-1", "device-1")];
        let mut second = vec![shaped_device("circuit-1", "device-1")];
        second[0].download_max_mbps = 200.0;

        assert_ne!(
            shaped_devices_fingerprint(&first),
            shaped_devices_fingerprint(&second)
        );
    }
}

#[cfg(test)]
mod tree_degrade_tests {
    use super::decide_tree_read;
    use super::TreeReadOutcome;

    fn parse_ok(_raw: &str) -> Result<(), serde_json::Error> {
        Ok(())
    }

    fn parse_err(_raw: &str) -> Result<(), serde_json::Error> {
        Err(serde_json::from_str::<serde_json::Value>("not json").unwrap_err())
    }

    #[test]
    fn good_read_builds_regardless_of_history() {
        for ever_loaded in [false, true] {
            assert!(matches!(
                decide_tree_read(Ok("{\"generation\":\"g\"}".to_string()), parse_ok, ever_loaded),
                TreeReadOutcome::Build(_)
            ));
        }
    }

    #[test]
    fn corrupt_after_good_keeps_last_good() {
        // MEDIUM-4: a transient mid-write read must not blank a working UI.
        assert!(matches!(
            decide_tree_read(Ok("garbage".to_string()), parse_err, true),
            TreeReadOutcome::KeepLastGood(_)
        ));
        assert!(matches!(
            decide_tree_read(
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "gone")),
                parse_ok,
                true
            ),
            TreeReadOutcome::KeepLastGood(_)
        ));
    }

    #[test]
    fn never_loaded_degrades_to_structureless_with_named_reasons() {
        assert!(matches!(
            decide_tree_read(Ok("garbage".to_string()), parse_err, false),
            TreeReadOutcome::PublishStructureless(_)
        ));
        assert!(matches!(
            decide_tree_read(
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "gone")),
                parse_ok,
                false
            ),
            TreeReadOutcome::PublishStructureless(_)
        ));
    }
}
