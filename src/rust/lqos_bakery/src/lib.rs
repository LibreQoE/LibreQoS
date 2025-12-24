//! The Bakery is where CAKE is made!
//!
//! More specifically, this crate provides a tracker of TC queues - described by the LibreQoS.py process,
//! but tracked for changes. We're at phase 3.
//!
//! In phase 1, the Bakery will build queues and a matching structure to track them. It will act exactly
//! like the LibreQoS.py process.
//!
//! In phase 2, the Bakery will *not* create CAKE queues - just the HTB hierarchy. When circuits are
//! detected as having traffic, the associated queue will be created. Ideally, some form of timeout
//! will be used to remove queues that are no longer in use. (Saving resources)
//!
//! In phase 3, the Bakery will - after initial creation - track the queues and update them as needed.
//! This will take a "diff" approach, finding differences and only applying those changes.
//!
//! In phase 4, the Bakery will implement "live move" --- allowing queues to be moved losslessly. This will
//! complete the NLNet project goals.

#![deny(clippy::unwrap_used)]
#![warn(missing_docs)]

mod commands;
mod diff;
mod queue_math;
mod utils;

use crossbeam_channel::{Receiver, Sender};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use tracing::{debug, error, info, warn};
use utils::current_timestamp;
pub(crate) const CHANNEL_CAPACITY: usize = 65536; // 64k capacity for Bakery commands
use crate::commands::ExecutionMode;
use crate::diff::{CircuitDiffResult, SiteDiffResult, diff_circuits, diff_sites};
use crate::queue_math::format_rate_for_tc_f32;
use crate::utils::{execute_in_memory, write_command_file};
pub use commands::BakeryCommands;
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient, TcHandle};
use lqos_config::{Config, LazyQueueMode};
use lqos_sys; // direct mapping control for live-move to avoid bus full-sync side-effects

// ---------------------- Live-Move Types and Helpers (module scope) ----------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MigrationStage {
    PrepareShadow,
    SwapToShadow,
    BuildFinal,
    SwapToFinal,
    TeardownShadow,
    Done,
}

#[derive(Clone, Debug)]
struct Migration {
    circuit_hash: i64,
    // Parent handles and majors
    parent_class_id: TcHandle,
    up_parent_class_id: TcHandle,
    class_major: u16,
    up_class_major: u16,
    // Old and new rates
    old_down_min: f32,
    old_down_max: f32,
    old_up_min: f32,
    old_up_max: f32,
    new_down_min: f32,
    new_down_max: f32,
    new_up_min: f32,
    new_up_max: f32,
    // Minors
    old_minor: u16,
    shadow_minor: u16,
    final_minor: u16,
    // IP list for remapping
    ips: Vec<String>,
    // Per-circuit SQM override ("cake" or "fq_codel"), if any
    sqm_override: Option<String>,
    stage: MigrationStage,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct StormguardOverrideKey {
    interface: String,
    class: TcHandle,
}

fn parse_ip_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

/*fn parse_ip_and_prefix(ip: &str) -> (String, u32) {
    if let Some((addr, pfx)) = ip.split_once('/') {
        if let Ok(n) = pfx.parse::<u32>() {
            return (addr.to_string(), n);
        }
    }
    if ip.contains(':') { (ip.to_string(), 128) } else { (ip.to_string(), 32) }
}*/

fn tc_handle_from_major_minor(major: u16, minor: u16) -> TcHandle {
    TcHandle::from_u32(((major as u32) << 16) | (minor as u32))
}

fn used_minors_for_parent(
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    parent: &TcHandle,
) -> HashSet<u16> {
    let mut set = HashSet::new();
    for (_k, v) in circuits.iter() {
        if let BakeryCommands::AddCircuit {
            parent_class_id,
            class_minor,
            ..
        } = v.as_ref()
        {
            if parent_class_id == parent {
                set.insert(*class_minor);
            }
        }
    }
    set
}

fn find_free_minor(
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    down_parent: &TcHandle,
    up_parent: &TcHandle,
) -> Option<u16> {
    let used_down = used_minors_for_parent(circuits, down_parent);
    let used_up = used_minors_for_parent(circuits, up_parent);
    for start in [0x2000u16, 0x4000, 0x6000, 0x8000, 0xA000, 0xC000, 0xE000] {
        let end = start.saturating_add(0x1FFF);
        for m in start..=end.min(0xFFFE) {
            if !used_down.contains(&m) && !used_up.contains(&m) {
                return Some(m);
            }
        }
    }
    for m in 1..=0xFFFEu16 {
        if !used_down.contains(&m) && !used_up.contains(&m) {
            return Some(m);
        }
    }
    None
}

fn add_commands_for_circuit(
    cmd: &BakeryCommands,
    config: &Arc<Config>,
    mode: ExecutionMode,
) -> Option<Vec<Vec<String>>> {
    cmd.to_commands(config, mode)
}

fn build_temp_add_cmd(
    base: &BakeryCommands,
    minor: u16,
    down_min: f32,
    down_max: f32,
    up_min: f32,
    up_max: f32,
) -> Option<BakeryCommands> {
    if let BakeryCommands::AddCircuit {
        circuit_hash,
        parent_class_id,
        up_parent_class_id,
        class_major,
        up_class_major,
        ip_addresses,
        sqm_override,
        ..
    } = base
    {
        Some(BakeryCommands::AddCircuit {
            circuit_hash: *circuit_hash,
            parent_class_id: *parent_class_id,
            up_parent_class_id: *up_parent_class_id,
            class_minor: minor,
            download_bandwidth_min: down_min,
            upload_bandwidth_min: up_min,
            download_bandwidth_max: down_max,
            upload_bandwidth_max: up_max,
            class_major: *class_major,
            up_class_major: *up_class_major,
            ip_addresses: ip_addresses.clone(),
            sqm_override: sqm_override.clone(),
        })
    } else {
        None
    }
}

/// Count of Bakery-Managed circuits that are currently active.
pub static ACTIVE_CIRCUITS: AtomicUsize = AtomicUsize::new(0);

/// Message Queue sender for the bakery
pub static BAKERY_SENDER: OnceLock<Sender<BakeryCommands>> = OnceLock::new();
static MQ_CREATED: AtomicBool = AtomicBool::new(false);
/// Indicates that at least one command batch has been processed and applied.
/// Used to avoid racing live activation against initial class creation.
static FIRST_COMMIT_APPLIED: AtomicBool = AtomicBool::new(false);

/// Starts the Bakery system, returning a channel sender for sending commands to the Bakery.
pub fn start_bakery() -> anyhow::Result<crossbeam_channel::Sender<BakeryCommands>> {
    let (tx, rx) = crossbeam_channel::bounded(CHANNEL_CAPACITY);
    let inner_sender = tx.clone();
    if BAKERY_SENDER.set(tx.clone()).is_err() {
        return Err(anyhow::anyhow!("Bakery sender is already initialized."));
    }
    std::thread::Builder::new()
        .name("lqos_bakery".to_string())
        .spawn(move || {
            bakery_main(rx, inner_sender);
        })
        .map_err(|e| anyhow::anyhow!("Failed to start Bakery thread: {}", e))?;
    Ok(tx)
}

fn bakery_main(rx: Receiver<BakeryCommands>, tx: Sender<BakeryCommands>) {
    // Current operation batch
    let mut batch: Option<Vec<Arc<BakeryCommands>>> = None;
    let mut sites: HashMap<i64, Arc<BakeryCommands>> = HashMap::new();
    let mut circuits: HashMap<i64, Arc<BakeryCommands>> = HashMap::new();
    let mut live_circuits: HashMap<i64, u64> = HashMap::new();
    // Persist latest StormGuard ceilings keyed by interface + class so we can replay after rebuilds.
    let mut stormguard_overrides: HashMap<StormguardOverrideKey, u64> = HashMap::new();

    // Mapping state
    #[derive(Clone, Hash, PartialEq, Eq, Debug)]
    struct MappingKey {
        ip: String,
        prefix: u32,
        upload: bool,
    }
    #[derive(Clone, Debug)]
    struct MappingVal {
        #[allow(dead_code)]
        handle: TcHandle,
        cpu: u32,
    }
    // Current kernel view (authoritative state) as tracked by the bakery
    let mut mapping_current: HashMap<MappingKey, MappingVal> = HashMap::new();
    // Next desired set staged during a batch (Python batches or other tools)
    let mut mapping_staged: Option<HashMap<MappingKey, MappingVal>> = None;
    // Keys that exist in the kernel but we couldn't classify to a known circuit (never delete automatically)
    let mut mapping_unknown: HashSet<MappingKey> = HashSet::new();
    let mut mapping_seeded: bool = false;

    let mut migrations: HashMap<i64, Migration> = HashMap::new();
    const MIGRATIONS_PER_TICK: usize = 16;

    fn parse_ip_and_prefix(ip: &str) -> (String, u32) {
        if let Some((addr, pfx)) = ip.split_once('/') {
            if let Ok(n) = pfx.parse::<u32>() {
                return (addr.to_string(), n);
            }
        }
        // No prefix provided; infer by address family
        // Simple heuristic: ':' suggests IPv6
        if ip.contains(':') {
            (ip.to_string(), 128)
        } else {
            (ip.to_string(), 32)
        }
    }

    fn handle_map_ip(
        ip_address: &str,
        tc_handle: TcHandle,
        cpu: u32,
        upload: bool,
        mapping_staged: &mut Option<HashMap<MappingKey, MappingVal>>,
    ) {
        let (ip, prefix) = parse_ip_and_prefix(ip_address);
        let key = MappingKey { ip, prefix, upload };
        let val = MappingVal {
            handle: tc_handle,
            cpu,
        };
        if mapping_staged.is_none() {
            *mapping_staged = Some(HashMap::new());
        }
        if let Some(stage) = mapping_staged.as_mut() {
            stage.insert(key, val);
        }
    }

    fn handle_del_ip(
        ip_address: &str,
        upload: bool,
        mapping_staged: &mut Option<HashMap<MappingKey, MappingVal>>,
        mapping_current: &mut HashMap<MappingKey, MappingVal>,
    ) {
        // Best-effort deletion: if exact prefix was provided, remove that, else try common host prefixes
        let (ip, prefix) = parse_ip_and_prefix(ip_address);
        let key = MappingKey {
            ip: ip.clone(),
            prefix,
            upload,
        };
        if let Some(stage) = mapping_staged.as_mut() {
            stage.remove(&key);
        }
        mapping_current.remove(&key);
    }

    fn build_handle_sets(
        circuits: &HashMap<i64, Arc<BakeryCommands>>,
    ) -> (HashSet<TcHandle>, HashSet<TcHandle>) {
        let mut down = HashSet::new();
        let mut up = HashSet::new();
        for (_k, v) in circuits.iter() {
            if let BakeryCommands::AddCircuit {
                class_minor,
                class_major,
                up_class_major,
                ..
            } = v.as_ref()
            {
                let down_tc =
                    TcHandle::from_u32(((*class_major as u32) << 16) | (*class_minor as u32));
                let up_tc =
                    TcHandle::from_u32(((*up_class_major as u32) << 16) | (*class_minor as u32));
                down.insert(down_tc);
                up.insert(up_tc);
            }
        }
        (down, up)
    }

    fn attempt_seed_mappings(
        circuits: &HashMap<i64, Arc<BakeryCommands>>,
        mapping_current: &mut HashMap<MappingKey, MappingVal>,
        mapping_unknown: &mut HashSet<MappingKey>,
    ) -> anyhow::Result<()> {
        // Build classification sets
        let (down_set, up_set) = build_handle_sets(circuits);

        // Create a small runtime to make a one-shot bus request
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(async {
            let mut bus = LibreqosBusClient::new().await?;
            let reply = bus.request(vec![BusRequest::ListIpFlow]).await?;
            for r in reply.iter() {
                if let BusResponse::MappedIps(list) = r {
                    for m in list.iter() {
                        // m.ip_address does not include prefix, prefix_length is provided
                        let key = MappingKey {
                            ip: m.ip_address.clone(),
                            prefix: m.prefix_length,
                            upload: if up_set.contains(&m.tc_handle) {
                                true
                            } else if down_set.contains(&m.tc_handle) {
                                false
                            } else {
                                // Unknown mapping (do not delete automatically)
                                let k = MappingKey {
                                    ip: m.ip_address.clone(),
                                    prefix: m.prefix_length,
                                    upload: false, // default; upload is unknown
                                };
                                mapping_unknown.insert(k.clone());
                                mapping_current.insert(
                                    k,
                                    MappingVal {
                                        handle: m.tc_handle,
                                        cpu: m.cpu,
                                    },
                                );
                                continue;
                            },
                        };
                        mapping_current.insert(
                            key,
                            MappingVal {
                                handle: m.tc_handle,
                                cpu: m.cpu,
                            },
                        );
                    }
                }
            }
            anyhow::Ok(())
        })
    }

    {
        let Ok(config) = lqos_config::load_config() else {
            error!("Failed to load configuration, exiting Bakery thread.");
            return;
        };
        info!(
            "Bakery thread starting. Mode: {:?}, expiration: {}s",
            config.queues.lazy_queues,
            config.queues.lazy_expire_seconds.unwrap_or(600)
        );
    }

    while let Ok(command) = rx.recv() {
        debug!("Bakery received command: {:?}", command);

        match command {
            // Mapping events (mirrored from lqosd bus handling)
            BakeryCommands::MapIp {
                ip_address,
                tc_handle,
                cpu,
                upload,
            } => {
                handle_map_ip(&ip_address, tc_handle, cpu, upload, &mut mapping_staged);
            }
            BakeryCommands::BusReady => {
                if !mapping_seeded {
                    match attempt_seed_mappings(
                        &circuits,
                        &mut mapping_current,
                        &mut mapping_unknown,
                    ) {
                        Ok(_) => {
                            let total = mapping_current.len();
                            let unknown = mapping_unknown.len();
                            info!(
                                "Bakery mappings seeded: total={}, unknown={}",
                                total, unknown
                            );
                            mapping_seeded = true;
                        }
                        Err(e) => warn!("Bakery: Failed to seed IP mappings at bus-ready: {:?}", e),
                    }
                }
            }
            BakeryCommands::DelIp { ip_address, upload } => {
                handle_del_ip(
                    &ip_address,
                    upload,
                    &mut mapping_staged,
                    &mut mapping_current,
                );
            }
            BakeryCommands::ClearIpAll => {
                mapping_current.clear();
                mapping_unknown.clear();
                mapping_staged = None;
            }
            BakeryCommands::CommitMappings => {
                // Ensure we are seeded before first commit to avoid assuming empty kernel state.
                if !mapping_seeded {
                    match attempt_seed_mappings(
                        &circuits,
                        &mut mapping_current,
                        &mut mapping_unknown,
                    ) {
                        Ok(_) => {
                            let total = mapping_current.len();
                            let unknown = mapping_unknown.len();
                            info!(
                                "Bakery mappings seeded: total={}, unknown={}",
                                total, unknown
                            );
                            mapping_seeded = true;
                        }
                        Err(e) => warn!("Bakery: Failed to seed IP mappings: {:?}", e),
                    }
                }

                if let Some(staged) = mapping_staged.take() {
                    // Remove stale mappings: present in current, not in staged; never delete unknowns
                    let mut stale = Vec::new();
                    for k in mapping_current.keys() {
                        if mapping_unknown.contains(k) {
                            continue; // don't touch unknowns
                        }
                        if !staged.contains_key(k) {
                            stale.push(k.clone());
                        }
                    }

                    if !stale.is_empty() {
                        // Batch deletions via the bus client
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build();
                        if let Ok(rt) = rt {
                            let stale_to_delete = stale.clone();
                            let _ = rt.block_on(async move {
                                if let Ok(mut bus) = LibreqosBusClient::new().await {
                                    // chunk operations to keep request sizes reasonable
                                    const CHUNK: usize = 512;
                                    for chunk in stale_to_delete.chunks(CHUNK) {
                                        let mut reqs = Vec::with_capacity(chunk.len());
                                        for k in chunk.iter() {
                                            // Recompose an IP string with prefix if not host (/32 or /128)
                                            let ip = if k.prefix == 32 || k.prefix == 128 {
                                                k.ip.clone()
                                            } else {
                                                format!("{}/{}", k.ip, k.prefix)
                                            };
                                            reqs.push(BusRequest::DelIpFlow {
                                                ip_address: ip,
                                                upload: k.upload,
                                            });
                                        }
                                        let _ = bus.request(reqs).await;
                                    }
                                }
                            });
                        } else {
                            warn!("Bakery: Unable to create runtime for stale IP deletions");
                        }

                        for k in stale.into_iter() {
                            mapping_current.remove(&k);
                        }
                    }

                    // Merge staged into current (they are already applied in kernel by lqosd)
                    for (k, v) in staged.into_iter() {
                        mapping_current.insert(k, v);
                    }
                }
            }
            BakeryCommands::StartBatch => {
                batch = Some(Vec::new());
            }
            BakeryCommands::CommitBatch => {
                handle_commit_batch(
                    &mut batch,
                    &mut sites,
                    &mut circuits,
                    &mut live_circuits,
                    &tx,
                    &mut migrations,
                    &stormguard_overrides,
                );
            }
            BakeryCommands::MqSetup { .. } => {
                if let Some(batch) = &mut batch {
                    batch.push(Arc::new(command));
                }
            }
            BakeryCommands::AddSite { .. } => {
                if let Some(batch) = &mut batch {
                    batch.push(Arc::new(command));
                }
            }
            BakeryCommands::AddCircuit { .. } => {
                if let Some(batch) = &mut batch {
                    batch.push(Arc::new(command));
                }
            }
            BakeryCommands::OnCircuitActivity { circuit_ids } => {
                handle_circuit_activity(circuit_ids, &circuits, &mut live_circuits);
            }
            BakeryCommands::Tick => {
                // Reset per-cycle counters at the start of the tick
                handle_tick(&mut circuits, &mut live_circuits, &mut sites);

                // Advance live-move migrations
                let Ok(config) = lqos_config::load_config() else {
                    error!("Failed to load configuration, exiting Bakery thread.");
                    continue;
                };
                let mut advanced = 0usize;
                let mut to_remove = Vec::new();
                for (_hash, mig) in migrations.iter_mut() {
                    if advanced >= MIGRATIONS_PER_TICK {
                        break;
                    }
                    match mig.stage {
                        MigrationStage::PrepareShadow => {
                            // Create shadow HTB+CAKE with OLD rates
                            if let Some(temp) = build_temp_add_cmd(
                                &BakeryCommands::AddCircuit {
                                    circuit_hash: mig.circuit_hash,
                                    parent_class_id: mig.parent_class_id,
                                    up_parent_class_id: mig.up_parent_class_id,
                                    class_minor: mig.shadow_minor,
                                    download_bandwidth_min: mig.old_down_min,
                                    upload_bandwidth_min: mig.old_up_min,
                                    download_bandwidth_max: mig.old_down_max,
                                    upload_bandwidth_max: mig.old_up_max,
                                    class_major: mig.class_major,
                                    up_class_major: mig.up_class_major,
                                    ip_addresses: "".to_string(),
                                    sqm_override: mig.sqm_override.clone(),
                                },
                                mig.shadow_minor,
                                mig.old_down_min,
                                mig.old_down_max,
                                mig.old_up_min,
                                mig.old_up_max,
                            ) {
                                let mut cmds = Vec::new();
                                match config.queues.lazy_queues.as_ref() {
                                    None | Some(LazyQueueMode::No) => {
                                        if let Some(c) = add_commands_for_circuit(
                                            &temp,
                                            &config,
                                            ExecutionMode::Builder,
                                        ) {
                                            cmds.extend(c);
                                        }
                                    }
                                    Some(LazyQueueMode::Htb) => {
                                        if let Some(c) = add_commands_for_circuit(
                                            &temp,
                                            &config,
                                            ExecutionMode::Builder,
                                        ) {
                                            cmds.extend(c);
                                        }
                                        if let Some(c) = add_commands_for_circuit(
                                            &temp,
                                            &config,
                                            ExecutionMode::LiveUpdate,
                                        ) {
                                            cmds.extend(c);
                                        }
                                    }
                                    Some(LazyQueueMode::Full) => {
                                        if let Some(c) = add_commands_for_circuit(
                                            &temp,
                                            &config,
                                            ExecutionMode::LiveUpdate,
                                        ) {
                                            cmds.extend(c);
                                        }
                                    }
                                }
                                if !cmds.is_empty() {
                                    execute_in_memory(&cmds, "live-move: create shadow");
                                }
                                mig.stage = MigrationStage::SwapToShadow;
                                advanced += 1;
                            } else {
                                warn!(
                                    "live-move: failed to build shadow add cmd for {}",
                                    mig.circuit_hash
                                );
                                mig.stage = MigrationStage::Done;
                                advanced += 1;
                            }
                        }
                        MigrationStage::SwapToShadow => {
                            // Remap all IPs to shadow handles using existing CPU
                            for ip in &mig.ips {
                                let (ip_s, prefix) = parse_ip_and_prefix(ip);
                                for &upload in &[false, true] {
                                    let key = MappingKey {
                                        ip: ip_s.clone(),
                                        prefix,
                                        upload,
                                    };
                                    let cpu = mapping_current.get(&key).map(|v| v.cpu).unwrap_or(0);
                                    let handle = if upload {
                                        tc_handle_from_major_minor(
                                            mig.up_class_major,
                                            mig.shadow_minor,
                                        )
                                    } else {
                                        tc_handle_from_major_minor(
                                            mig.class_major,
                                            mig.shadow_minor,
                                        )
                                    };
                                    let _ = lqos_sys::add_ip_to_tc(&ip_s, handle, cpu, upload);
                                    // Update local mapping view
                                    mapping_current.insert(key.clone(), MappingVal { handle, cpu });
                                }
                            }
                            // Clear the hot cache directly
                            let _ = lqos_sys::clear_hot_cache();
                            mig.stage = MigrationStage::BuildFinal;
                            advanced += 1;
                        }
                        MigrationStage::BuildFinal => {
                            // Delete old classes/qdiscs and create final with NEW rates at original minor
                            if let Some(old_cmd) = build_temp_add_cmd(
                                &BakeryCommands::AddCircuit {
                                    circuit_hash: mig.circuit_hash,
                                    parent_class_id: mig.parent_class_id,
                                    up_parent_class_id: mig.up_parent_class_id,
                                    class_minor: mig.old_minor,
                                    download_bandwidth_min: mig.old_down_min,
                                    upload_bandwidth_min: mig.old_up_min,
                                    download_bandwidth_max: mig.old_down_max,
                                    upload_bandwidth_max: mig.old_up_max,
                                    class_major: mig.class_major,
                                    up_class_major: mig.up_class_major,
                                    ip_addresses: "".to_string(),
                                    sqm_override: mig.sqm_override.clone(),
                                },
                                mig.old_minor,
                                mig.old_down_min,
                                mig.old_down_max,
                                mig.old_up_min,
                                mig.old_up_max,
                            ) {
                                let mut cmds = Vec::new();
                                if let Some(prune) = old_cmd.to_prune(&config, true) {
                                    cmds.extend(prune);
                                }
                                // Final add (new rates) at final_minor
                                if let Some(final_cmd) = build_temp_add_cmd(
                                    &old_cmd,
                                    mig.final_minor,
                                    mig.new_down_min,
                                    mig.new_down_max,
                                    mig.new_up_min,
                                    mig.new_up_max,
                                ) {
                                    match config.queues.lazy_queues.as_ref() {
                                        None | Some(LazyQueueMode::No) => {
                                            if let Some(c) = add_commands_for_circuit(
                                                &final_cmd,
                                                &config,
                                                ExecutionMode::Builder,
                                            ) {
                                                cmds.extend(c);
                                            }
                                        }
                                        Some(LazyQueueMode::Htb) => {
                                            if let Some(c) = add_commands_for_circuit(
                                                &final_cmd,
                                                &config,
                                                ExecutionMode::Builder,
                                            ) {
                                                cmds.extend(c);
                                            }
                                            if let Some(c) = add_commands_for_circuit(
                                                &final_cmd,
                                                &config,
                                                ExecutionMode::LiveUpdate,
                                            ) {
                                                cmds.extend(c);
                                            }
                                        }
                                        Some(LazyQueueMode::Full) => {
                                            if let Some(c) = add_commands_for_circuit(
                                                &final_cmd,
                                                &config,
                                                ExecutionMode::LiveUpdate,
                                            ) {
                                                cmds.extend(c);
                                            }
                                        }
                                    }
                                }
                                if !cmds.is_empty() {
                                    execute_in_memory(&cmds, "live-move: build final");
                                }
                                mig.stage = MigrationStage::SwapToFinal;
                                advanced += 1;
                            } else {
                                warn!(
                                    "live-move: failed to build old prune cmd for {}",
                                    mig.circuit_hash
                                );
                                mig.stage = MigrationStage::Done;
                                advanced += 1;
                            }
                        }
                        MigrationStage::SwapToFinal => {
                            for ip in &mig.ips {
                                let (ip_s, prefix) = parse_ip_and_prefix(ip);
                                for &upload in &[false, true] {
                                    let key = MappingKey {
                                        ip: ip_s.clone(),
                                        prefix,
                                        upload,
                                    };
                                    let cpu = mapping_current.get(&key).map(|v| v.cpu).unwrap_or(0);
                                    let handle = if upload {
                                        tc_handle_from_major_minor(
                                            mig.up_class_major,
                                            mig.final_minor,
                                        )
                                    } else {
                                        tc_handle_from_major_minor(mig.class_major, mig.final_minor)
                                    };
                                    let _ = lqos_sys::add_ip_to_tc(&ip_s, handle, cpu, upload);
                                    mapping_current.insert(key.clone(), MappingVal { handle, cpu });
                                }
                            }
                            let _ = lqos_sys::clear_hot_cache();
                            mig.stage = MigrationStage::TeardownShadow;
                            advanced += 1;
                        }
                        MigrationStage::TeardownShadow => {
                            // Remove shadow classes
                            if let Some(shadow_cmd) = build_temp_add_cmd(
                                &BakeryCommands::AddCircuit {
                                    circuit_hash: mig.circuit_hash,
                                    parent_class_id: mig.parent_class_id,
                                    up_parent_class_id: mig.up_parent_class_id,
                                    class_minor: mig.shadow_minor,
                                    download_bandwidth_min: mig.old_down_min,
                                    upload_bandwidth_min: mig.old_up_min,
                                    download_bandwidth_max: mig.old_down_max,
                                    upload_bandwidth_max: mig.old_up_max,
                                    class_major: mig.class_major,
                                    up_class_major: mig.up_class_major,
                                    ip_addresses: "".to_string(),
                                    sqm_override: mig.sqm_override.clone(),
                                },
                                mig.shadow_minor,
                                mig.old_down_min,
                                mig.old_down_max,
                                mig.old_up_min,
                                mig.old_up_max,
                            ) {
                                if let Some(prune) = shadow_cmd.to_prune(&config, true) {
                                    execute_in_memory(&prune, "live-move: prune shadow");
                                }
                            }
                            mig.stage = MigrationStage::Done;
                            advanced += 1;
                        }
                        MigrationStage::Done => {
                            to_remove.push(mig.circuit_hash);
                        }
                    }
                }
                for h in to_remove {
                    migrations.remove(&h);
                }
            }
            BakeryCommands::ChangeSiteSpeedLive {
                site_hash,
                download_bandwidth_min,
                upload_bandwidth_min,
                download_bandwidth_max,
                upload_bandwidth_max,
            } => {
                handle_change_site_speed_live(
                    site_hash,
                    download_bandwidth_min,
                    upload_bandwidth_min,
                    download_bandwidth_max,
                    upload_bandwidth_max,
                    &mut sites,
                );
            }
            BakeryCommands::StormGuardAdjustment {
                dry_run,
                interface_name,
                class_id,
                new_rate,
            } => {
                let has_mq_run = MQ_CREATED.load(Relaxed);
                if !has_mq_run {
                    debug!("StormGuardAdjustment received before MQ setup, skipping.");
                    continue;
                }
                let Ok(tc_handle) = TcHandle::from_string(&class_id) else {
                    warn!("StormGuardAdjustment has invalid class_id [{}], skipping.", class_id);
                    continue;
                };
                if !dry_run {
                    let key = StormguardOverrideKey {
                        interface: interface_name.to_string(),
                        class: tc_handle,
                    };
                    stormguard_overrides.insert(key, new_rate);
                }
                let normalized_class = tc_handle.as_tc_string();
                // Build the HTB command
                let args = vec![
                    "class".to_string(),
                    "replace".to_string(),
                    "dev".to_string(),
                    interface_name.to_string(),
                    "classid".to_string(),
                    normalized_class.clone(),
                    "htb".to_string(),
                    "rate".to_string(),
                    format!("{}mbit", new_rate.saturating_sub(1)),
                    "ceil".to_string(),
                    format!("{}mbit", new_rate),
                ];
                if dry_run {
                    info!("DRY RUN: /sbin/tc {}", args.join(" "));
                } else {
                    let output = std::process::Command::new("/sbin/tc").args(&args).output();
                    match output {
                        Err(e) => {
                            warn!("Failed to run tc command: {}", e);
                        }
                        Ok(out) => {
                            if !out.status.success() {
                                warn!(
                                    "tc command failed: {}",
                                    String::from_utf8_lossy(&out.stderr)
                                );
                            } else {
                                debug!(
                                    "tc command succeeded: {}",
                                    String::from_utf8_lossy(&out.stdout)
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    error!("Bakery thread exited unexpectedly.");
}

fn handle_commit_batch(
    batch: &mut Option<Vec<Arc<BakeryCommands>>>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &mut HashMap<i64, u64>,
    tx: &Sender<BakeryCommands>,
    migrations: &mut HashMap<i64, Migration>,
    stormguard_overrides: &HashMap<StormguardOverrideKey, u64>,
) {
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration, exiting Bakery thread.");
        return;
    };

    let Some(new_batch) = batch.take() else {
        debug!("CommitBatch received without a batch to commit.");
        return;
    };

    let has_mq_been_setup = MQ_CREATED.load(std::sync::atomic::Ordering::Relaxed);
    if !has_mq_been_setup {
        // If the MQ hasn't been created, we need to do this as a full, unadjusted run.
        info!("MQ not created, performing full reload.");
        full_reload(
            batch,
            sites,
            circuits,
            live_circuits,
            &config,
            new_batch,
            &stormguard_overrides,
        );
        MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
        return;
    }

    let site_change_mode = diff_sites(&new_batch, sites);
    if matches!(site_change_mode, SiteDiffResult::RebuildRequired) {
        // If the site structure has changed, we need to rebuild everything.
        info!("Bakery full reload: site_struct=1, circuit_struct=0");
        full_reload(
            batch,
            sites,
            circuits,
            live_circuits,
            &config,
            new_batch,
            &stormguard_overrides,
        );
        MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
        return;
    }

    let circuit_change_mode = diff_circuits(&new_batch, circuits);

    // If neither has changed, there's nothing to do.
    if matches!(site_change_mode, SiteDiffResult::NoChange)
        && matches!(circuit_change_mode, CircuitDiffResult::NoChange)
    {
        // No changes detected, skip processing
        info!("No changes detected in batch, skipping processing.");
        return;
    }

    // If any structural changes occurred, do a full reload
    if let CircuitDiffResult::Categorized(categories) = &circuit_change_mode {
        if !categories.structural_changed.is_empty() {
            info!(
                "Bakery full reload: site_struct=0, circuit_struct={}",
                categories.structural_changed.len()
            );
            full_reload(
                batch,
                sites,
                circuits,
                live_circuits,
                &config,
                new_batch,
                &stormguard_overrides,
            );
            MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
            return;
        }
    }

    // Declare any site speed changes that need to be applied. We're sending them
    // to ourselves as future commands via the BakeryCommands channel.
    let mut site_speed_change_count = 0usize;
    if let SiteDiffResult::SpeedChanges { changes } = site_change_mode {
        site_speed_change_count = changes.len();
        for change in &changes {
            let BakeryCommands::AddSite {
                site_hash,
                download_bandwidth_min,
                upload_bandwidth_min,
                download_bandwidth_max,
                upload_bandwidth_max,
                ..
            } = change
            else {
                debug!(
                    "ChangeSiteSpeedLive received a non-site command: {:?}",
                    change
                );
                continue;
            };
            if let Err(e) = tx.try_send(BakeryCommands::ChangeSiteSpeedLive {
                site_hash: *site_hash,
                download_bandwidth_min: *download_bandwidth_min,
                upload_bandwidth_min: *upload_bandwidth_min,
                download_bandwidth_max: *download_bandwidth_max,
                upload_bandwidth_max: *upload_bandwidth_max,
            }) {
                error!("Channel full, falling back to full rebuild: {}", e);
                info!("Bakery full reload: site_struct=0, circuit_struct=0");
                full_reload(
                    batch,
                    sites,
                    circuits,
                    live_circuits,
                    &config,
                    new_batch.clone(),
                    &stormguard_overrides,
                );
                return; // Skip the rest of this CommitBatch processing
            }
        }
    }

    // Now we can process circuit changes incrementally
    if let CircuitDiffResult::Categorized(categories) = circuit_change_mode {
        // One-line summary of changes (info!)
        info!(
            "Bakery changes: sites_speed={}, circuits_added={}, removed={}, speed={}, ip={}",
            site_speed_change_count,
            categories.newly_added.len(),
            categories.removed_circuits.len(),
            categories.speed_changed.len(),
            categories.ip_changed.len()
        );

        // 1) Removals
        if !categories.removed_circuits.is_empty() {
            for circuit_hash in categories.removed_circuits {
                if let Some(circuit) = circuits.remove(&circuit_hash) {
                    let was_activated = live_circuits.contains_key(&circuit_hash);
                    let commands = match config.queues.lazy_queues.as_ref() {
                        None | Some(LazyQueueMode::No) => circuit.to_prune(&config, true),
                        Some(LazyQueueMode::Htb) => {
                            if was_activated {
                                circuit.to_prune(&config, false)
                            } else {
                                None
                            }
                        }
                        Some(LazyQueueMode::Full) => {
                            if was_activated {
                                circuit.to_prune(&config, true)
                            } else {
                                None
                            }
                        }
                    };
                    if let Some(cmd) = commands {
                        execute_in_memory(&cmd, "removing circuit");
                    }
                    live_circuits.remove(&circuit_hash);
                } else {
                    debug!("RemoveCircuit received for unknown circuit: {}", circuit_hash);
                }
            }
        }

        // 2) Speed changes (avoid linux TC deadlock by removing qdisc first)
        if !categories.speed_changed.is_empty() {
            let mut immediate_commands = Vec::new();
            for cmd in &categories.speed_changed {
                if let BakeryCommands::AddCircuit {
                    circuit_hash,
                    parent_class_id,
                    up_parent_class_id,
                    class_minor,
                    download_bandwidth_min,
                    upload_bandwidth_min,
                    download_bandwidth_max,
                    upload_bandwidth_max,
                    class_major,
                    up_class_major,
                    ip_addresses,
                    sqm_override,
                } = cmd.as_ref()
                {
                    let was_activated = live_circuits.contains_key(circuit_hash);
                    if was_activated {
                        // Attempt live-move
                        if let Some(shadow_minor) =
                            find_free_minor(circuits, parent_class_id, up_parent_class_id)
                        {
                            // Find old command for old rates
                            if let Some(old_cmd) = circuits.get(circuit_hash) {
                                if let BakeryCommands::AddCircuit {
                                    download_bandwidth_min: old_down_min,
                                    upload_bandwidth_min: old_up_min,
                                    download_bandwidth_max: old_down_max,
                                    upload_bandwidth_max: old_up_max,
                                    ..
                                } = old_cmd.as_ref()
                                {
                                    let mig = Migration {
                                        circuit_hash: *circuit_hash,
                                        parent_class_id: *parent_class_id,
                                        up_parent_class_id: *up_parent_class_id,
                                        class_major: *class_major,
                                        up_class_major: *up_class_major,
                                        old_down_min: *old_down_min,
                                        old_down_max: *old_down_max,
                                        old_up_min: *old_up_min,
                                        old_up_max: *old_up_max,
                                        new_down_min: *download_bandwidth_min,
                                        new_down_max: *download_bandwidth_max,
                                        new_up_min: *upload_bandwidth_min,
                                        new_up_max: *upload_bandwidth_max,
                                        old_minor: *class_minor,
                                        shadow_minor,
                                        final_minor: *class_minor,
                                        ips: parse_ip_list(ip_addresses),
                                        sqm_override: sqm_override.clone(),
                                        stage: MigrationStage::PrepareShadow,
                                    };
                                    migrations.insert(*circuit_hash, mig);
                                    // Update desired circuit definition now
                                    circuits.insert(*circuit_hash, Arc::clone(cmd));
                                    continue; // skip immediate path
                                }
                            }
                        }
                    }
                    // Fallback: immediate safe update
                    match config.queues.lazy_queues.as_ref() {
                        None | Some(LazyQueueMode::No) => {
                            if let Some(prune) = cmd.to_prune(&config, true) {
                                immediate_commands.extend(prune);
                            }
                            if let Some(add) = cmd.to_commands(&config, ExecutionMode::Builder) {
                                immediate_commands.extend(add);
                            }
                        }
                        Some(LazyQueueMode::Htb) => {
                            if was_activated {
                                if let Some(prune) = cmd.to_prune(&config, false) {
                                    immediate_commands.extend(prune);
                                }
                                if let Some(add_htb) =
                                    cmd.to_commands(&config, ExecutionMode::Builder)
                                {
                                    immediate_commands.extend(add_htb);
                                }
                                if let Some(add_qdisc) =
                                    cmd.to_commands(&config, ExecutionMode::LiveUpdate)
                                {
                                    immediate_commands.extend(add_qdisc);
                                }
                            } else {
                                if let Some(add_htb) =
                                    cmd.to_commands(&config, ExecutionMode::Builder)
                                {
                                    immediate_commands.extend(add_htb);
                                }
                            }
                        }
                        Some(LazyQueueMode::Full) => {
                            if was_activated {
                                if let Some(prune) = cmd.to_prune(&config, true) {
                                    immediate_commands.extend(prune);
                                }
                                if let Some(add_all) =
                                    cmd.to_commands(&config, ExecutionMode::LiveUpdate)
                                {
                                    immediate_commands.extend(add_all);
                                }
                            } else {
                                // No TC ops
                            }
                        }
                    }
                    circuits.insert(*circuit_hash, Arc::clone(cmd));
                }
            }
            if !immediate_commands.is_empty() {
                execute_in_memory(&immediate_commands, "updating circuit speeds (fallback)");
            }
        }

        // 3) Additions
        if !categories.newly_added.is_empty() {
            let commands: Vec<Vec<String>> = categories
                .newly_added
                .iter()
                .filter_map(|c| c.to_commands(&config, ExecutionMode::Builder))
                .flatten()
                .collect();
            if !commands.is_empty() {
                execute_in_memory(&commands, "adding new circuits");
            }
            for command in categories.newly_added {
                if let BakeryCommands::AddCircuit { circuit_hash, .. } = command.as_ref() {
                    circuits.insert(*circuit_hash, Arc::clone(command));
                }
            }
        }

        // 4) IP-only changes require no TC commands; mappings already handled by mapping engine
        // We still refresh the stored circuit snapshot for those entries
        if !categories.ip_changed.is_empty() {
            for command in categories.ip_changed {
                if let BakeryCommands::AddCircuit { circuit_hash, .. } = command.as_ref() {
                    circuits.insert(*circuit_hash, Arc::clone(command));
                }
            }
        }
    }
}

fn handle_circuit_activity(
    circuit_ids: HashSet<i64>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &mut HashMap<i64, u64>,
) {
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration, exiting Bakery thread.");
        return;
    };
    match config.queues.lazy_queues.as_ref() {
        None | Some(LazyQueueMode::No) => return,
        _ => {}
    }

    // Defer live activation until MQ and at least one commit has fully applied.
    if !MQ_CREATED.load(Ordering::Relaxed) || !FIRST_COMMIT_APPLIED.load(Ordering::Relaxed) {
        debug!(
            "Skipping live activation: MQ_CREATED={}, FIRST_COMMIT_APPLIED={}",
            MQ_CREATED.load(Ordering::Relaxed),
            FIRST_COMMIT_APPLIED.load(Ordering::Relaxed)
        );
        return;
    }

    let mut commands = Vec::new();
    for circuit_id in circuit_ids {
        if let Some(circuit) = live_circuits.get_mut(&circuit_id) {
            *circuit = current_timestamp();
            continue;
        }

        if let Some(command) = circuits.get(&circuit_id) {
            // On first activation, ensure HTB exists in HTB-lazy mode by prepending
            // Builder-mode HTB class creation (idempotent via "class replace").
            let mut cmd = Vec::new();
            match config.queues.lazy_queues.as_ref() {
                Some(LazyQueueMode::Htb) => {
                    if let Some(builder_cmds) = command.to_commands(&config, ExecutionMode::Builder)
                    {
                        cmd.extend(builder_cmds);
                    }
                    if let Some(live_cmds) = command.to_commands(&config, ExecutionMode::LiveUpdate)
                    {
                        cmd.extend(live_cmds);
                    }
                    if cmd.is_empty() {
                        // No commands to apply for this circuit
                        continue;
                    }
                }
                _ => {
                    // Full lazy mode handles both HTB and SQM in LiveUpdate; or other modes
                    let Some(live_cmds) = command.to_commands(&config, ExecutionMode::LiveUpdate)
                    else {
                        continue;
                    };
                    cmd.extend(live_cmds);
                }
            }
            live_circuits.insert(circuit_id, current_timestamp());
            commands.extend(cmd);
        }
    }
    if commands.is_empty() {
        return; // No commands to write
    }
    execute_in_memory(&commands, "enabling live circuits");
}

fn handle_tick(
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &mut HashMap<i64, u64>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
) {
    // This is a periodic tick to expire lazy queues
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration, exiting Bakery thread.");
        return;
    };

    // Periodically shrink HashMap capacity if it's much larger than needed
    static mut TICK_COUNT: u64 = 0;
    unsafe {
        TICK_COUNT += 1;
        if TICK_COUNT.is_multiple_of(60) {
            // Every minute
            // Shrink if capacity is more than 2x the size
            if circuits.capacity() > circuits.len() * 2 && circuits.capacity() > 100 {
                debug!(
                    "Shrinking circuits HashMap: {} entries, {} capacity",
                    circuits.len(),
                    circuits.capacity()
                );
                circuits.shrink_to_fit();
            }
            if live_circuits.capacity() > live_circuits.len() * 2 && live_circuits.capacity() > 100
            {
                debug!(
                    "Shrinking live_circuits HashMap: {} entries, {} capacity",
                    live_circuits.len(),
                    live_circuits.capacity()
                );
                live_circuits.shrink_to_fit();
            }
            if sites.capacity() > sites.len() * 2 && sites.capacity() > 100 {
                debug!(
                    "Shrinking sites HashMap: {} entries, {} capacity",
                    sites.len(),
                    sites.capacity()
                );
                sites.shrink_to_fit();
            }
        }
    }

    match config.queues.lazy_queues.as_ref() {
        None | Some(LazyQueueMode::No) => {
            ACTIVE_CIRCUITS.store(circuits.len(), Ordering::Relaxed);
            return;
        }
        _ => {
            ACTIVE_CIRCUITS.store(live_circuits.len(), Ordering::Relaxed);
        }
    }

    // Now we know that lazy queues are enabled, we can expire them!
    let max_age_seconds = config.queues.lazy_expire_seconds.unwrap_or(600);
    if max_age_seconds == 0 {
        // If max_age_seconds is 0, we do not expire queues
        return;
    }

    let mut to_destroy = Vec::new();
    let now = current_timestamp();
    for (circuit_id, last_activity) in live_circuits.iter() {
        if now - *last_activity > max_age_seconds {
            to_destroy.push(*circuit_id);
        }
    }

    if to_destroy.is_empty() {
        return; // No queues to expire
    }

    let mut commands = Vec::new();
    for circuit_id in to_destroy {
        if let Some(command) = circuits.get(&circuit_id) {
            let Some(cmd) = command.to_prune(&config, false) else {
                continue;
            };
            live_circuits.remove(&circuit_id);
            commands.extend(cmd);
        }
    }

    if commands.is_empty() {
        return; // No commands to write
    }
    execute_in_memory(&commands, "pruning lazy queues");
}

fn handle_change_site_speed_live(
    site_hash: i64,
    download_bandwidth_min: f32,
    upload_bandwidth_min: f32,
    download_bandwidth_max: f32,
    upload_bandwidth_max: f32,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
) {
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration, exiting Bakery thread.");
        return;
    };
    if let Some(site_arc) = sites.get(&site_hash) {
        let BakeryCommands::AddSite {
            site_hash: _,
            parent_class_id,
            up_parent_class_id,
            class_minor,
            ..
        } = site_arc.as_ref()
        else {
            debug!(
                "ChangeSiteSpeedLive received a non-site command: {:?}",
                site_arc
            );
            return;
        };
        let to_internet = config.internet_interface();
        let to_isp = config.isp_interface();
        let class_id = format!(
            "0x{:x}:0x{:x}",
            parent_class_id.get_major_minor().0,
            class_minor
        );
        let up_class_id = format!(
            "0x{:x}:0x{:x}",
            up_parent_class_id.get_major_minor().0,
            class_minor
        );
        let upload_bandwidth_min = if upload_bandwidth_min >= (upload_bandwidth_max - 0.5) {
            upload_bandwidth_max - 1.0
        } else {
            upload_bandwidth_min
        };
        let download_bandwidth_min = if download_bandwidth_min >= (download_bandwidth_max - 0.5) {
            download_bandwidth_max - 1.0
        } else {
            download_bandwidth_min
        };
        let commands = vec![
            vec![
                "class".to_string(),
                "change".to_string(),
                "dev".to_string(),
                to_internet,
                "classid".to_string(),
                up_class_id,
                "htb".to_string(),
                "rate".to_string(),
                format_rate_for_tc_f32(upload_bandwidth_min),
                "ceil".to_string(),
                format_rate_for_tc_f32(upload_bandwidth_max),
            ],
            vec![
                "class".to_string(),
                "change".to_string(),
                "dev".to_string(),
                to_isp,
                "classid".to_string(),
                class_id,
                "htb".to_string(),
                "rate".to_string(),
                format_rate_for_tc_f32(download_bandwidth_min),
                "ceil".to_string(),
                format_rate_for_tc_f32(download_bandwidth_max),
            ],
        ];
        execute_in_memory(&commands, "changing site speed live");
        // Update the site speeds in the site map - create a new Arc with updated values
        let new_site = Arc::new(BakeryCommands::AddSite {
            site_hash,
            parent_class_id: *parent_class_id,
            up_parent_class_id: *up_parent_class_id,
            class_minor: *class_minor,
            download_bandwidth_min,
            upload_bandwidth_min,
            download_bandwidth_max,
            upload_bandwidth_max,
        });
        sites.insert(site_hash, new_site);
    } else {
        info!(
            "ChangeSiteSpeedLive received for unknown site: {}",
            site_hash
        );
    }
}

fn full_reload(
    batch: &mut Option<Vec<Arc<BakeryCommands>>>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &mut HashMap<i64, u64>,
    config: &Arc<Config>,
    new_batch: Vec<Arc<BakeryCommands>>,
    stormguard_overrides: &HashMap<StormguardOverrideKey, u64>,
) {
    warn!("Bakery: Full reload triggered due to site or circuit changes.");
    sites.clear();
    circuits.clear();
    live_circuits.clear();
    process_batch(new_batch, config, sites, circuits);
    *batch = None;
    apply_stormguard_overrides(stormguard_overrides, config);
}

fn process_batch(
    batch: Vec<Arc<BakeryCommands>>,
    config: &Arc<lqos_config::Config>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
) {
    info!("Bakery: Processing batch of {} commands", batch.len());
    let mut circuit_count = 0u64;
    let commands = batch
        .into_iter()
        .filter_map(|b| {
            // Ensure that our state map is up to date with the latest commands
            match b.as_ref() {
                BakeryCommands::AddSite { site_hash, .. } => {
                    sites.insert(*site_hash, Arc::clone(&b));
                }
                BakeryCommands::AddCircuit { circuit_hash, .. } => {
                    circuits.insert(*circuit_hash, Arc::clone(&b));
                    circuit_count += 1;
                }
                _ => {}
            }
            b.to_commands(config, ExecutionMode::Builder)
        })
        .flatten()
        .collect::<Vec<Vec<String>>>();

    let path = Path::new(&config.lqos_directory).join("linux_tc_rust.txt");
    write_command_file(&path, &commands);
    execute_in_memory(&commands, "processing batch");

    // Mark that at least one batch has been applied, unblocking live activation.
    FIRST_COMMIT_APPLIED.store(true, Ordering::Relaxed);
}

fn apply_stormguard_overrides(
    overrides: &HashMap<StormguardOverrideKey, u64>,
    config: &Arc<Config>,
) {
    let _ = config; // currently unused but kept for future interface-specific logic
    if overrides.is_empty() {
        return;
    }
    let mut commands = Vec::new();
    for (key, rate) in overrides.iter() {
        commands.push(vec![
            "class".to_string(),
            "replace".to_string(),
            "dev".to_string(),
            key.interface.clone(),
            "classid".to_string(),
            key.class.as_tc_string(),
            "htb".to_string(),
            "rate".to_string(),
            format!("{}mbit", rate.saturating_sub(1)),
            "ceil".to_string(),
            format!("{}mbit", rate),
        ]);
    }
    execute_in_memory(&commands, "replaying StormGuard overrides");
}
