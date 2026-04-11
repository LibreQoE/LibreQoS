use crate::dynamic::{CircuitObservation, expired_dynamic_circuit_ids};
use crate::dynamic_store::{load_dynamic_circuits_from_disk, persist_dynamic_circuits_to_disk};
use crate::state;
use crate::{DaemonHooks, ShapedDevicesCatalog, load_network_json, load_shaped_devices};
use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Receiver, Sender};
use ip_network_table::IpNetworkTable;
use once_cell::sync::OnceCell;
use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tracing::{debug, error, warn};

const RELOAD_RETRY_DELAY_MS: u64 = 500;
const RELOAD_ATTEMPTS: usize = 2;
const DYNAMIC_CIRCUITS_MAINTENANCE_TICK_SECONDS: u64 = 60;
const MAX_UNKNOWN_IP_PROMOTIONS_PER_OBSERVATION: usize = 32;

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

pub(crate) fn request_reload_shaped_devices(reason: &str) -> Result<()> {
    let (reply_tx, reply_rx) = oneshot::channel();
    sender()?.send(NetworkDevicesCommand::ReloadShapedDevices {
        reason: reason.to_string(),
        reply: reply_tx,
    })?;
    reply_rx
        .blocking_recv()
        .map_err(|_| anyhow!("ShapedDevices reload reply channel closed"))?
}

pub(crate) fn request_reload_network_json(reason: &str) -> Result<()> {
    let (reply_tx, reply_rx) = oneshot::channel();
    sender()?.send(NetworkDevicesCommand::ReloadNetworkJson {
        reason: reason.to_string(),
        reply: reply_tx,
    })?;
    reply_rx
        .blocking_recv()
        .map_err(|_| anyhow!("NetworkJson reload reply channel closed"))?
}

pub(crate) fn apply_shaped_devices_snapshot(
    reason: &str,
    shaped: lqos_config::ConfigShapedDevices,
) -> Result<()> {
    let (reply_tx, reply_rx) = oneshot::channel();
    sender()?.send(NetworkDevicesCommand::ApplyShapedDevicesSnapshot {
        reason: reason.to_string(),
        shaped: Box::new(shaped),
        reply: reply_tx,
    })?;
    reply_rx
        .blocking_recv()
        .map_err(|_| anyhow!("Apply shaped devices reply channel closed"))?
}

pub(crate) fn report_observations(observations: &[CircuitObservation]) {
    if observations.is_empty() {
        return;
    }

    let Some(sender) = ACTOR_SENDER.get().cloned() else {
        return;
    };
    let _ = sender.try_send(NetworkDevicesCommand::ReportObservations {
        observations: observations.to_vec(),
    });
}

pub(crate) fn upsert_dynamic_circuit(shaped_device: lqos_config::ShapedDevice) -> Result<()> {
    let (reply_tx, reply_rx) = oneshot::channel();
    sender()?.send(NetworkDevicesCommand::UpsertDynamicCircuit {
        shaped_device: Box::new(shaped_device),
        reply: reply_tx,
    })?;
    reply_rx
        .blocking_recv()
        .map_err(|_| anyhow!("Upsert dynamic circuit reply channel closed"))?
}

pub(crate) fn remove_dynamic_circuit(circuit_id: &str) -> Result<bool> {
    let (reply_tx, reply_rx) = oneshot::channel();
    sender()?.send(NetworkDevicesCommand::RemoveDynamicCircuit {
        circuit_id: circuit_id.to_string(),
        reply: reply_tx,
    })?;
    reply_rx
        .blocking_recv()
        .map_err(|_| anyhow!("Remove dynamic circuit reply channel closed"))?
}

enum NetworkDevicesCommand {
    ReloadShapedDevices {
        reason: String,
        reply: oneshot::Sender<Result<()>>,
    },
    ReloadNetworkJson {
        reason: String,
        reply: oneshot::Sender<Result<()>>,
    },
    ApplyShapedDevicesSnapshot {
        reason: String,
        shaped: Box<lqos_config::ConfigShapedDevices>,
        reply: oneshot::Sender<Result<()>>,
    },
    ReportObservations {
        observations: Vec<CircuitObservation>,
    },
    UpsertDynamicCircuit {
        shaped_device: Box<lqos_config::ShapedDevice>,
        reply: oneshot::Sender<Result<()>>,
    },
    RemoveDynamicCircuit {
        circuit_id: String,
        reply: oneshot::Sender<Result<bool>>,
    },
}

fn actor_loop(rx: Receiver<NetworkDevicesCommand>, hooks: Option<Arc<dyn DaemonHooks>>) {
    debug!("lqos_network_devices actor starting");

    if let Err(err) = reload_shaped_devices_inner("startup", hooks.as_deref()) {
        warn!("Initial shaped-devices load failed: {err}");
    }
    if let Err(err) = reload_network_json_inner("startup", hooks.as_deref()) {
        warn!("Initial network-json load failed: {err}");
    }
    reload_dynamic_circuits_inner("startup");

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
                        let result = reload_shaped_devices_inner(&reason, hooks.as_deref());
                        let _ = reply.send(result);
                    }
                    NetworkDevicesCommand::ReloadNetworkJson { reason, reply } => {
                        let result = reload_network_json_inner(&reason, hooks.as_deref());
                        let _ = reply.send(result);
                    }
                    NetworkDevicesCommand::ApplyShapedDevicesSnapshot {
                        reason,
                        shaped,
                        reply,
                    } => {
                        debug!("Publishing shaped-devices snapshot reason={reason}");
                        state::publish_shaped_devices(*shaped);
                        if let Some(hooks) = &hooks {
                            hooks.on_shaped_devices_updated();
                        }
                        let _ = reply.send(Ok(()));
                    }
                    NetworkDevicesCommand::ReportObservations { observations } => {
                        handle_observations(&observations, hooks.as_deref());
                    }
                    NetworkDevicesCommand::UpsertDynamicCircuit {
                        shaped_device,
                        reply,
                    } => {
                        let result = upsert_dynamic_circuit_inner(*shaped_device);
                        let _ = reply.send(result);
                    }
                    NetworkDevicesCommand::RemoveDynamicCircuit { circuit_id, reply } => {
                        let result = remove_dynamic_circuit_inner(&circuit_id);
                        let _ = reply.send(result);
                    }
                }
            }
            recv(tick) -> _ => {
                prune_expired_dynamic_circuits_inner(hooks.as_deref());
            }
        }
    }

    error!("lqos_network_devices actor stopped");
}

fn normalize_circuit_id_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn recompute_hashes(device: &mut lqos_config::ShapedDevice) {
    device.circuit_hash = lqos_utils::hash_to_i64(&device.circuit_id);
    device.device_hash = lqos_utils::hash_to_i64(&device.device_id);
    device.parent_hash = lqos_utils::hash_to_i64(&device.parent_node);
}

fn reload_dynamic_circuits_inner(reason: &str) {
    let circuits = load_dynamic_circuits_from_disk();
    debug!("Loaded {} dynamic circuits reason={reason}", circuits.len());
    state::publish_dynamic_circuits_snapshot(circuits);
}

fn prune_expired_dynamic_circuits_inner(hooks: Option<&dyn DaemonHooks>) {
    let Ok(config) = lqos_config::load_config() else {
        return;
    };
    let Some(dynamic_cfg) = config.dynamic_circuits.as_ref() else {
        return;
    };
    if !dynamic_cfg.enabled {
        return;
    }
    let ttl_seconds = dynamic_cfg.ttl_seconds;
    if ttl_seconds == 0 {
        return;
    }

    let Ok(now_unix) = lqos_utils::unix_time::unix_now() else {
        return;
    };

    let snapshot = state::dynamic_circuits_snapshot();
    if snapshot.is_empty() {
        return;
    }

    let mut expired_ids = expired_dynamic_circuit_ids(snapshot.as_ref(), now_unix, ttl_seconds);
    if expired_ids.is_empty() {
        return;
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

    if let Err(err) = persist_dynamic_circuits_to_disk(&updated) {
        warn!("Unable to persist dynamic circuits after TTL pruning: {err:?}");
        return;
    }
    state::publish_dynamic_circuits_snapshot(updated);
    debug!(
        "Pruned {} expired dynamic circuits ttl_seconds={ttl_seconds}",
        expired_ids.len()
    );

    if let Some(hooks) = hooks {
        hooks.on_dynamic_circuits_expired(&expired_ids);
    }
}

fn upsert_dynamic_circuit_inner(mut shaped_device: lqos_config::ShapedDevice) -> Result<()> {
    recompute_hashes(&mut shaped_device);
    let now_unix = lqos_utils::unix_time::unix_now().context("get unix time")?;

    let snapshot = state::dynamic_circuits_snapshot();
    let mut updated = snapshot.as_ref().clone();
    let key = normalize_circuit_id_key(&shaped_device.circuit_id);
    let mut should_persist = false;

    if let Some(pos) = updated
        .iter()
        .position(|c| normalize_circuit_id_key(&c.shaped.circuit_id) == key)
    {
        if updated[pos].shaped != shaped_device {
            should_persist = true;
        }
        updated[pos].shaped = shaped_device;
        updated[pos].last_seen_unix = now_unix;
    } else {
        updated.push(crate::DynamicCircuit {
            shaped: shaped_device,
            last_seen_unix: now_unix,
        });
        should_persist = true;
    }

    if should_persist {
        persist_dynamic_circuits_to_disk(&updated).context("persist dynamic circuits to disk")?;
    }
    state::publish_dynamic_circuits_snapshot(updated);
    Ok(())
}

fn remove_dynamic_circuit_inner(circuit_id: &str) -> Result<bool> {
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

    persist_dynamic_circuits_to_disk(&updated).context("persist dynamic circuits to disk")?;
    state::publish_dynamic_circuits_snapshot(updated);
    Ok(true)
}

fn handle_observations(observations: &[CircuitObservation], hooks: Option<&dyn DaemonHooks>) {
    if observations.is_empty() {
        return;
    }

    let catalog = state::shaped_devices_catalog();
    let dynamic_snapshot = state::dynamic_circuits_snapshot();
    let mut dynamic_device_hashes: HashSet<i64> = HashSet::new();
    let mut dynamic_circuit_hashes: HashSet<i64> = HashSet::new();
    dynamic_device_hashes.reserve(dynamic_snapshot.len());
    dynamic_circuit_hashes.reserve(dynamic_snapshot.len());
    for circuit in dynamic_snapshot.iter() {
        dynamic_device_hashes.insert(circuit.shaped.device_hash);
        dynamic_circuit_hashes.insert(circuit.shaped.circuit_hash);
    }

    let mut seen_device_hashes: HashSet<i64> = HashSet::new();
    let mut seen_circuit_hashes: HashSet<i64> = HashSet::new();
    let mut unknown_candidates: Vec<CircuitObservation> = Vec::new();

    for observation in observations {
        if catalog
            .device_by_hashes(observation.device_hash, observation.circuit_hash)
            .is_some()
        {
            continue;
        }

        if let Some(device_hash) = observation.device_hash
            && dynamic_device_hashes.contains(&device_hash)
        {
            seen_device_hashes.insert(device_hash);
            continue;
        }

        if let Some(circuit_hash) = observation.circuit_hash
            && dynamic_circuit_hashes.contains(&circuit_hash)
        {
            seen_circuit_hashes.insert(circuit_hash);
            continue;
        }

        unknown_candidates.push(*observation);
    }

    if (!seen_device_hashes.is_empty() || !seen_circuit_hashes.is_empty())
        && let Ok(now_unix) = lqos_utils::unix_time::unix_now()
    {
        state::refresh_dynamic_circuits_last_seen_for_hashes(
            &seen_device_hashes,
            &seen_circuit_hashes,
            now_unix,
        );
    }

    if !unknown_candidates.is_empty() {
        maybe_promote_unknown_ips(&unknown_candidates, &catalog, hooks);
    }
}

fn maybe_promote_unknown_ips(
    unknown_candidates: &[CircuitObservation],
    catalog: &ShapedDevicesCatalog,
    hooks: Option<&dyn DaemonHooks>,
) {
    let Ok(config) = lqos_config::load_config() else {
        return;
    };

    let Some(dynamic_cfg) = config.dynamic_circuits.as_ref() else {
        return;
    };
    if !dynamic_cfg.enabled || !dynamic_cfg.enable_unknown_ip_promotion {
        return;
    }
    if dynamic_cfg.ranges.is_empty() {
        return;
    }

    let mut trie: IpNetworkTable<usize> = IpNetworkTable::new();
    for (idx, rule) in dynamic_cfg.ranges.iter().enumerate() {
        trie.insert(rule.ip_range, idx);
    }

    let mut promoted = 0usize;
    for observation in unknown_candidates {
        if promoted >= MAX_UNKNOWN_IP_PROMOTIONS_PER_OBSERVATION {
            break;
        }

        // Never promote an IP that is already covered by static shaped devices.
        if catalog
            .device_longest_match_for_ip(&observation.ip)
            .is_some()
        {
            continue;
        }

        let ip: IpAddr = observation.ip.as_ip();
        let Some((_net, rule_idx)) = trie.longest_match(ip) else {
            continue;
        };
        let Some(rule) = dynamic_cfg.ranges.get(*rule_idx) else {
            continue;
        };

        let label = format!("[dyn] ({}) {}", rule.name.trim(), observation.ip);
        let parent_node = resolve_unknown_promotion_parent(rule.attach_to.trim());
        if parent_node.trim().is_empty() {
            continue;
        }

        let shaped_device = shaped_device_from_unknown_ip(&label, &parent_node, ip, rule);
        if let Err(err) = upsert_dynamic_circuit_inner(shaped_device.clone()) {
            warn!(
                "Unable to persist dynamic circuit from unknown IP promotion for {}: {err:?}",
                observation.ip
            );
            continue;
        }

        if let Some(hooks) = hooks {
            hooks.on_unknown_ip_promoted(&shaped_device);
        }

        promoted += 1;
    }

    if promoted > 0 {
        debug!(
            "Promoted {promoted} unknown IP(s) into dynamic circuits (candidates={})",
            unknown_candidates.len()
        );
    }
}

fn resolve_unknown_promotion_parent(attach_to: &str) -> String {
    let trimmed_attach = attach_to.trim();
    if !trimmed_attach.is_empty() {
        if let Some(resolved) = crate::resolve_parent_node(trimmed_attach) {
            return resolved.name;
        }

        // In "flat" setups (no meaningful `network.json`), attachments are just site-name strings.
        let is_flat = state::with_network_json_read(|net_json| {
            let nodes = net_json.get_nodes_when_ready();
            let has_non_root = nodes
                .iter()
                .any(|node| node.name.trim() != "Root" && !node.name.trim().is_empty());
            nodes.len() <= 1 || !has_non_root
        });
        if is_flat {
            return trimmed_attach.to_string();
        }
    }

    let (node_len, orphans, first_non_root) = state::with_network_json_read(|net_json| {
        let nodes = net_json.get_nodes_when_ready();
        let first_non_root = nodes
            .iter()
            .find(|node| node.name.trim() != "Root" && !node.name.trim().is_empty())
            .map(|node| node.name.clone());
        let orphans = nodes
            .iter()
            .find(|node| node.name.trim().eq_ignore_ascii_case("ORPHANS"))
            .map(|node| node.name.clone());

        (nodes.len(), orphans, first_non_root)
    });
    // "Flat" means `network.json` had no meaningful tree beyond Root.
    let is_flat = node_len <= 1 || first_non_root.is_none();
    if !is_flat {
        if let Some(orphans) = orphans {
            return orphans;
        }
        if let Some(first) = first_non_root {
            return first;
        }
    }

    let shaped_parent = state::shaped_devices_snapshot()
        .devices
        .iter()
        .find_map(|dev| {
            let trimmed = dev.parent_node.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_string())
        });

    shaped_parent.unwrap_or_else(|| "Root".to_string())
}

fn shaped_device_from_unknown_ip(
    label: &str,
    parent_node: &str,
    ip: IpAddr,
    rule: &lqos_config::DynamicCircuitRangeRule,
) -> lqos_config::ShapedDevice {
    let mut device = lqos_config::ShapedDevice {
        circuit_id: label.to_string(),
        circuit_name: label.to_string(),
        device_id: label.to_string(),
        device_name: label.to_string(),
        parent_node: parent_node.to_string(),
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

fn reload_shaped_devices_inner(reason: &str, hooks: Option<&dyn DaemonHooks>) -> Result<()> {
    for attempt in 1..=RELOAD_ATTEMPTS {
        match load_shaped_devices() {
            Ok(shaped) => {
                debug!("Loaded shaped devices reason={reason}");
                state::publish_shaped_devices(shaped);
                if let Some(hooks) = hooks {
                    hooks.on_shaped_devices_updated();
                }
                return Ok(());
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
