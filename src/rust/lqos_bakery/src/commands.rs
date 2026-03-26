use crate::MQ_CREATED;
use crate::qdisc_handles::{InfraQdiscSlot, infra_qdisc_handle};
use crate::queue_math::{
    format_rate_for_tc, format_rate_for_tc_f32, quantum, r2q, sqm_as_vec, sqm_tokens_for,
};
use allocative::Allocative;
use lqos_bus::TcHandle;
use lqos_config::LazyQueueMode;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::Sender as ReplySender;
use tracing::{debug, info};

/// Runtime TreeGuard node-operation action tracked by Bakery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Allocative)]
pub enum RuntimeNodeOperationAction {
    /// Virtualize the target node/subtree.
    Virtualize,
    /// Restore the target node/subtree to the physical tree.
    Restore,
}

/// Runtime TreeGuard node-operation status tracked by Bakery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Allocative)]
pub enum RuntimeNodeOperationStatus {
    /// Operation was accepted but has not started applying yet.
    Submitted,
    /// Operation was not started because Bakery runtime-node capacity is currently saturated.
    Deferred,
    /// Operation is currently applying reparent/restore work.
    Applying,
    /// Traffic-carrying work succeeded; cleanup is still pending.
    AppliedAwaitingCleanup,
    /// Operation completed successfully.
    Completed,
    /// Operation failed and can be retried.
    Failed,
    /// Operation could not be reconciled safely and the subtree is now frozen.
    Dirty,
}

/// Structured failure reason for a Bakery-tracked TreeGuard runtime node operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Allocative)]
pub enum RuntimeNodeOperationFailureReason {
    /// The node has no direct child sites or direct circuits to promote.
    StructuralIneligibleNoPromotableChildren,
    /// The node has only one promotable direct child, so v1 flattening has no deterministic split.
    StructuralIneligibleSinglePromotableChild,
}

/// Snapshot of a Bakery-tracked TreeGuard runtime node operation.
#[derive(Debug, Clone, PartialEq, Eq, Allocative)]
pub struct RuntimeNodeOperationSnapshot {
    /// Monotonic Bakery-local operation identifier.
    pub operation_id: u64,
    /// Stable Bakery site hash derived from the node name.
    pub site_hash: i64,
    /// Requested runtime action.
    pub action: RuntimeNodeOperationAction,
    /// Current operation status.
    pub status: RuntimeNodeOperationStatus,
    /// Number of retry/apply attempts performed so far.
    pub attempt_count: u32,
    /// Unix timestamp when the operation was submitted.
    pub submitted_at_unix: u64,
    /// Unix timestamp when the operation last changed state.
    pub updated_at_unix: u64,
    /// Optional unix timestamp for the next retry, if waiting.
    pub next_retry_at_unix: Option<u64>,
    /// Last error observed by Bakery for this operation, if any.
    pub last_error: Option<String>,
    /// Structured failure reason observed by Bakery for this operation, if any.
    pub failure_reason: Option<RuntimeNodeOperationFailureReason>,
}

#[derive(Debug, Clone, Copy, Allocative)]
struct AddSiteParams {
    site_hash: i64,
    parent_class_id: TcHandle,
    up_parent_class_id: TcHandle,
    class_minor: u16,
    download_bandwidth_min: f32,
    upload_bandwidth_min: f32,
    download_bandwidth_max: f32,
    upload_bandwidth_max: f32,
}

#[derive(Debug, Clone, Allocative)]
struct AddCircuitParams {
    circuit_hash: i64,
    parent_class_id: TcHandle,
    up_parent_class_id: TcHandle,
    class_minor: u16,
    download_bandwidth_min: f32,
    upload_bandwidth_min: f32,
    download_bandwidth_max: f32,
    upload_bandwidth_max: f32,
    class_major: u16,
    up_class_major: u16,
    down_qdisc_handle: Option<u16>,
    up_qdisc_handle: Option<u16>,
    // Optional per-circuit SQM override: "cake" or "fq_codel"
    sqm_override: Option<String>,
}

/// Execution Mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Allocative)]
pub enum ExecutionMode {
    /// We're constructing the tree
    Builder,
    /// Live Update
    LiveUpdate,
}

/// List of commands that the Bakery system can handle.
#[derive(Debug, Clone, Allocative)]
pub enum BakeryCommands {
    /// Notification that the bus socket is ready; bakery can seed mappings
    BusReady,
    /// Add or update an IP mapping (mirrors `MapIpToFlow` from the bus)
    MapIp {
        /// The IP address to map (may include CIDR prefix)
        ip_address: String,
        /// Classifier handle (major:minor)
        tc_handle: TcHandle,
        /// CPU index
        cpu: u32,
        /// Upload map (on-a-stick second map)
        upload: bool,
    },
    /// Delete an IP mapping (mirrors `DelIpFlow` from the bus)
    DelIp {
        /// The IP address to unmap (may include CIDR prefix)
        ip_address: String,
        /// Upload map (on-a-stick second map)
        upload: bool,
    },
    /// Clear all IP mappings (mirrors `ClearIpFlow` from the bus)
    ClearIpAll,
    /// Commit the current set of staged IP mappings and perform stale cleanup.
    CommitMappings,
    /// Send this when circuits are seen by the throughput tracker
    OnCircuitActivity {
        /// All active circuit IDs
        circuit_ids: HashSet<i64>,
    },
    /// Periodic tick
    Tick,
    /// Change an existing site's HTB rates live without a rebuild.
    ///
    /// Updates the min/ceil rates for both download (ISP-facing) and upload
    /// (Internet-facing) classes associated with the specified site.
    ChangeSiteSpeedLive {
        /// Unique identifier for the target site.
        site_hash: i64,
        /// New minimum (guaranteed) download rate in Mbps.
        download_bandwidth_min: f32,
        /// New minimum (guaranteed) upload rate in Mbps.
        upload_bandwidth_min: f32,
        /// New maximum (ceiling) download rate in Mbps.
        download_bandwidth_max: f32,
        /// New maximum (ceiling) upload rate in Mbps.
        upload_bandwidth_max: f32,
    },
    /// Begin a batch of changes; subsequent commands are queued until commit.
    StartBatch,
    /// Commit the current batch, diffing and applying queued changes.
    CommitBatch,
    /// Set up MQ roots and per-queue parents on one or both interfaces.
    MqSetup {
        /// Total number of MQ queues to create per interface.
        queues_available: usize,
        /// Offset applied to queue indices on the Internet-facing side
        /// when operating in on-a-stick configurations.
        stick_offset: usize,
    },
    /// Add or update a top-level site class pair under the given parents.
    AddSite {
        /// Unique identifier for the site.
        site_hash: i64,
        /// Parent class handle on the ISP-facing interface (downlink side).
        parent_class_id: TcHandle,
        /// Parent class handle on the Internet-facing interface (uplink side).
        up_parent_class_id: TcHandle,
        /// Minor class ID shared by uplink/downlink site classes.
        class_minor: u16,
        /// Minimum (guaranteed) download rate in Mbps.
        download_bandwidth_min: f32,
        /// Minimum (guaranteed) upload rate in Mbps.
        upload_bandwidth_min: f32,
        /// Maximum (ceiling) download rate in Mbps.
        download_bandwidth_max: f32,
        /// Maximum (ceiling) upload rate in Mbps.
        upload_bandwidth_max: f32,
    },
    /// Add or update a circuit beneath a site; may add SQM depending on mode.
    AddCircuit {
        /// Unique identifier for the circuit.
        circuit_hash: i64,
        /// Parent class handle on the ISP-facing interface (downlink side).
        parent_class_id: TcHandle,
        /// Parent class handle on the Internet-facing interface (uplink side).
        up_parent_class_id: TcHandle,
        /// Minor class ID used for both uplink and downlink circuit classes.
        class_minor: u16,
        /// Minimum (guaranteed) download rate in Mbps.
        download_bandwidth_min: f32,
        /// Minimum (guaranteed) upload rate in Mbps.
        upload_bandwidth_min: f32,
        /// Maximum (ceiling) download rate in Mbps.
        download_bandwidth_max: f32,
        /// Maximum (ceiling) upload rate in Mbps.
        upload_bandwidth_max: f32,
        /// Major class ID (downlink) used when attaching SQM/HTB.
        class_major: u16,
        /// Major class ID (uplink) used when attaching SQM/HTB.
        up_class_major: u16,
        /// Explicit qdisc handle major for the downlink leaf qdisc, if assigned.
        down_qdisc_handle: Option<u16>,
        /// Explicit qdisc handle major for the uplink leaf qdisc, if assigned.
        up_qdisc_handle: Option<u16>,
        /// Concatenated list of all IPs for this circuit.
        ip_addresses: String, // Concatenated list of all IPs for this circuit
        /// Optional per-circuit SQM override: "cake" or "fq_codel"
        sqm_override: Option<String>,
    },
    /// Change a specific HTB class rate on-the-fly; optionally dry-run.
    StormGuardAdjustment {
        /// If true, log the tc command instead of executing it.
        dry_run: bool,
        /// Network interface name (e.g., `eth0`) containing the class.
        interface_name: String,
        /// Fully qualified class identifier (e.g., `1:2`).
        class_id: String,
        /// New class ceiling rate in Mbps (the handler sets ceil and rate-1).
        new_rate: u64,
    },
    /// Runtime TreeGuard request to virtualize or restore a non-top-level site without a full reload.
    TreeGuardSetNodeVirtual {
        /// Stable Bakery site hash derived from the node name.
        site_hash: i64,
        /// Whether the site should be virtualized (`true`) or restored (`false`).
        virtualized: bool,
        /// Optional synchronous reply channel for immediate operation-state reporting.
        #[allocative(skip)]
        reply: Option<ReplySender<RuntimeNodeOperationSnapshot>>,
    },
}

impl BakeryCommands {
    /// Translate this command into concrete `tc` argument vectors.
    ///
    /// Returns a list of `tc` argv arrays in execution order, or `None`
    /// when the command does not directly emit `tc` operations (e.g.,
    /// batch control) or when, given `execution_mode` and the current
    /// configuration (lazy queue settings), no immediate changes are required.
    ///
    /// Arguments:
    /// - `config`: Current loaded configuration used for interfaces, rates and SQM.
    /// - `execution_mode`: Whether we're building the tree or applying live updates.
    ///
    /// Returns:
    /// - `Some(Vec<Vec<String>>)` where each inner `Vec<String>` is a single
    ///   `tc` invocation's argument list (without the binary), or `None` if
    ///   nothing should be executed for this command.
    pub fn to_commands(
        &self,
        config: &Arc<lqos_config::Config>,
        execution_mode: ExecutionMode,
    ) -> Option<Vec<Vec<String>>> {
        match self {
            BakeryCommands::MqSetup {
                queues_available,
                stick_offset,
            } => Self::mq_setup(config, *queues_available, *stick_offset),
            BakeryCommands::AddSite {
                site_hash,
                parent_class_id,
                up_parent_class_id,
                class_minor,
                download_bandwidth_min,
                upload_bandwidth_min,
                download_bandwidth_max,
                upload_bandwidth_max,
            } => Self::add_site(
                config,
                AddSiteParams {
                    site_hash: *site_hash,
                    parent_class_id: *parent_class_id,
                    up_parent_class_id: *up_parent_class_id,
                    class_minor: *class_minor,
                    download_bandwidth_min: *download_bandwidth_min,
                    upload_bandwidth_min: *upload_bandwidth_min,
                    download_bandwidth_max: *download_bandwidth_max,
                    upload_bandwidth_max: *upload_bandwidth_max,
                },
            ),
            BakeryCommands::AddCircuit {
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
                down_qdisc_handle,
                up_qdisc_handle,
                ip_addresses: _,
                sqm_override,
            } => Self::add_circuit(
                execution_mode,
                config,
                AddCircuitParams {
                    circuit_hash: *circuit_hash,
                    parent_class_id: *parent_class_id,
                    up_parent_class_id: *up_parent_class_id,
                    class_minor: *class_minor,
                    download_bandwidth_min: *download_bandwidth_min,
                    upload_bandwidth_min: *upload_bandwidth_min,
                    download_bandwidth_max: *download_bandwidth_max,
                    upload_bandwidth_max: *upload_bandwidth_max,
                    class_major: *class_major,
                    up_class_major: *up_class_major,
                    down_qdisc_handle: *down_qdisc_handle,
                    up_qdisc_handle: *up_qdisc_handle,
                    sqm_override: sqm_override.clone(),
                },
            ),
            _ => None,
        }
    }

    fn mq_setup(
        config: &Arc<lqos_config::Config>,
        queues_available: usize,
        stick_offset: usize,
    ) -> Option<Vec<Vec<String>>> {
        let mut result = Vec::new();
        let observe_only = config.queues.queue_mode.is_observe();
        let mq_already_created = MQ_CREATED.load(std::sync::atomic::Ordering::Relaxed);

        if observe_only {
            if !mq_already_created {
                result.push(root_mq_add_command(config.isp_interface()));
                if !config.on_a_stick_mode() {
                    result.push(root_mq_add_command(config.internet_interface()));
                }
            }
            MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
            return Some(result);
        }
        info!(
            "Setting up MQ with {} queues and stick offset {}",
            queues_available, stick_offset
        );
        // Root MQ preflight is handled outside the strict builder batch during
        // full reloads. The builder path only bootstraps root mq when runtime
        // state is empty and otherwise rebuilds child HTB/SQM state beneath
        // the prepared root.
        let sqm_strings = sqm_as_vec(config);
        let r2q = r2q(u64::max(
            config.queues.uplink_bandwidth_mbps,
            config.queues.downlink_bandwidth_mbps,
        ));

        // ISP-facing interface (interface_a in Python)
        if !mq_already_created {
            result.push(root_mq_add_command(config.isp_interface()));
        }

        /*
        for queue in range(queuesAvailable):
            command = 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+1) + ' handle ' + hex(queue+1) + ': htb default 2'
            linuxTCcommands.append(command)
            command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ': classid ' + hex(queue+1) + ':1 htb rate '+ format_rate_for_tc(upstream_bandwidth_capacity_download_mbps()) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_download_mbps()) + quantum(upstream_bandwidth_capacity_download_mbps())
            linuxTCcommands.append(command)
            command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 ' + sqm()
            linuxTCcommands.append(command)
            # Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
            # Technically, that should not even happen. So don't expect much if any traffic in this default class.
            # Only 1/4 of defaultClassCapacity is guaranteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
            command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 classid ' + hex(queue+1) + ':2 htb rate ' + format_rate_for_tc(round((upstream_bandwidth_capacity_download_mbps()-1)/4)) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_download_mbps()-1) + ' prio 5' + quantum(upstream_bandwidth_capacity_download_mbps())
            linuxTCcommands.append(command)
            command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':2 ' + sqm()
            linuxTCcommands.append(command)
         */

        for queue in 0..queues_available {
            // command = 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+1) + ' handle ' + hex(queue+1) + ': htb default 2'
            result.push(vec![
                "qdisc".to_string(),
                "add".to_string(),
                "dev".to_string(),
                config.isp_interface(),
                "parent".to_string(),
                format!("7FFF:0x{:x}", queue + 1),
                "handle".to_string(),
                format!("0x{:x}:", queue + 1),
                "htb".to_string(),
                "default".to_string(),
                "2".to_string(),
            ]);
            // command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ': classid ' + hex(queue+1) + ':1 htb rate '+ format_rate_for_tc(upstream_bandwidth_capacity_download_mbps()) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_download_mbps()) + quantum(upstream_bandwidth_capacity_download_mbps())
            result.push(vec![
                "class".to_string(),
                "add".to_string(),
                "dev".to_string(),
                config.isp_interface(),
                "parent".to_string(),
                format!("0x{:x}:", queue + 1),
                "classid".to_string(),
                format!("0x{:x}:1", queue + 1),
                "htb".to_string(),
                "rate".to_string(),
                // On ISP-facing (downlink) side, use downlink capacity
                format_rate_for_tc(config.queues.downlink_bandwidth_mbps),
                "ceil".to_string(),
                format_rate_for_tc(config.queues.downlink_bandwidth_mbps),
                "quantum".to_string(),
                quantum(config.queues.downlink_bandwidth_mbps, r2q),
            ]);
            // command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 ' + sqm()
            let mut class = vec![
                "qdisc".to_string(),
                "add".to_string(),
                "dev".to_string(),
                config.isp_interface(),
                "parent".to_string(),
                format!("0x{:x}:1", queue + 1),
                "handle".to_string(),
                format!(
                    "0x{:x}:",
                    infra_qdisc_handle((queue + 1) as u16, InfraQdiscSlot::Primary)
                ),
            ];
            class.extend(sqm_strings.clone());
            result.push(class);

            // Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
            // command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 classid ' + hex(queue+1) + ':2 htb rate ' + format_rate_for_tc(round((upstream_bandwidth_capacity_download_mbps()-1)/4)) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_download_mbps()-1) + ' prio 5' + quantum(upstream_bandwidth_capacity_download_mbps())
            // Default class parameters should also reflect downlink capacity on ISP-facing side
            let mbps = config.queues.downlink_bandwidth_mbps as f64;
            let mbps_quarter = (mbps - 1.0) / 4.0;
            let mbps_minus_one = mbps - 1.0;
            result.push(vec![
                "class".to_string(),
                "add".to_string(),
                "dev".to_string(),
                config.isp_interface(),
                "parent".to_string(),
                format!("0x{:x}:1", queue + 1),
                "classid".to_string(),
                format!("0x{:x}:2", queue + 1),
                "htb".to_string(),
                "rate".to_string(),
                format_rate_for_tc(mbps_quarter as u64),
                "ceil".to_string(),
                format_rate_for_tc(mbps_minus_one as u64),
                "prio".to_string(),
                "5".to_string(),
                "quantum".to_string(),
                quantum(config.queues.downlink_bandwidth_mbps, r2q),
            ]);
            // command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':2 ' + sqm()
            let mut default_class = vec![
                "qdisc".to_string(),
                "add".to_string(),
                "dev".to_string(),
                config.isp_interface(),
                "parent".to_string(),
                format!("0x{:x}:2", queue + 1),
                "handle".to_string(),
                format!(
                    "0x{:x}:",
                    infra_qdisc_handle((queue + 1) as u16, InfraQdiscSlot::Default)
                ),
            ];
            default_class.extend(sqm_strings.clone());
            result.push(default_class);
        }

        // Internet-facing interface (interface_b in Python)
        if !config.on_a_stick_mode() && !mq_already_created {
            result.push(root_mq_add_command(config.internet_interface()));
        }

        /*
        for queue in range(queuesAvailable):
            command = 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+stickOffset+1) + ' handle ' + hex(queue+stickOffset+1) + ': htb default 2'
            linuxTCcommands.append(command)
            command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ': classid ' + hex(queue+stickOffset+1) + ':1 htb rate '+ format_rate_for_tc(upstream_bandwidth_capacity_upload_mbps()) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_upload_mbps()) + quantum(upstream_bandwidth_capacity_upload_mbps())
            linuxTCcommands.append(command)
            command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':1 ' + sqm()
            linuxTCcommands.append(command)
            # Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
            # Technically, that should not even happen. So don't expect much if any traffic in this default class.
            # Only 1/4 of defaultClassCapacity is guarenteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
            command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':1 classid ' + hex(queue+stickOffset+1) + ':2 htb rate ' + format_rate_for_tc(round((upstream_bandwidth_capacity_upload_mbps()-1)/4)) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_upload_mbps()-1) + ' prio 5' + quantum(upstream_bandwidth_capacity_upload_mbps())
            linuxTCcommands.append(command)
            command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':2 ' + sqm()
            linuxTCcommands.append(command)
         */
        for queue in 0..queues_available {
            // command = 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+stickOffset+1) + ' handle ' + hex(queue+stickOffset+1) + ': htb default 2'
            result.push(vec![
                "qdisc".to_string(),
                "add".to_string(),
                "dev".to_string(),
                config.internet_interface(),
                "parent".to_string(),
                format!("7FFF:0x{:x}", queue + stick_offset + 1),
                "handle".to_string(),
                format!("0x{:x}:", queue + stick_offset + 1),
                "htb".to_string(),
                "default".to_string(),
                "2".to_string(),
            ]);
            // command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ': classid ' + hex(queue+stickOffset+1) + ':1 htb rate '+ format_rate_for_tc(upstream_bandwidth_capacity_upload_mbps()) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_upload_mbps()) + quantum(upstream_bandwidth_capacity_upload_mbps())
            result.push(vec![
                "class".to_string(),
                "add".to_string(),
                "dev".to_string(),
                config.internet_interface(),
                "parent".to_string(),
                format!("0x{:x}:", queue + stick_offset + 1),
                "classid".to_string(),
                format!("0x{:x}:1", queue + stick_offset + 1),
                "htb".to_string(),
                "rate".to_string(),
                // Internet-facing (uplink) side should use uplink capacity
                format_rate_for_tc(config.queues.uplink_bandwidth_mbps),
                "ceil".to_string(),
                format_rate_for_tc(config.queues.uplink_bandwidth_mbps),
                "quantum".to_string(),
                quantum(config.queues.uplink_bandwidth_mbps, r2q),
            ]);
            // command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':1 ' + sqm()
            let mut class = vec![
                "qdisc".to_string(),
                "add".to_string(),
                "dev".to_string(),
                config.internet_interface(),
                "parent".to_string(),
                format!("0x{:x}:1", queue + stick_offset + 1),
                "handle".to_string(),
                format!(
                    "0x{:x}:",
                    infra_qdisc_handle((queue + stick_offset + 1) as u16, InfraQdiscSlot::Primary)
                ),
            ];
            class.extend(sqm_strings.clone());
            result.push(class);
            // Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
            // command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':1 classid ' + hex(queue+stickOffset+1) + ':2 htb rate ' + format_rate_for_tc(round((upstream_bandwidth_capacity_upload_mbps()-1)/4)) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_upload_mbps()-1) + ' prio 5' + quantum(upstream_bandwidth_capacity_upload_mbps())
            // Default class parameters should reflect uplink capacity on Internet-facing side
            let mbps = config.queues.uplink_bandwidth_mbps as f64;
            let mbps_quarter = (mbps - 1.0) / 4.0;
            let mbps_minus_one = mbps - 1.0;
            result.push(vec![
                "class".to_string(),
                "add".to_string(),
                "dev".to_string(),
                config.internet_interface(),
                "parent".to_string(),
                format!("0x{:x}:1", queue + stick_offset + 1),
                "classid".to_string(),
                format!("0x{:x}:2", queue + stick_offset + 1),
                "htb".to_string(),
                "rate".to_string(),
                format_rate_for_tc(mbps_quarter as u64),
                "ceil".to_string(),
                format_rate_for_tc(mbps_minus_one as u64),
                "prio".to_string(),
                "5".to_string(),
                "quantum".to_string(),
                quantum(config.queues.uplink_bandwidth_mbps, r2q),
            ]);
            // command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':2 ' + sqm()
            let mut default_class = vec![
                "qdisc".to_string(),
                "add".to_string(),
                "dev".to_string(),
                config.internet_interface(),
                "parent".to_string(),
                format!("0x{:x}:2", queue + stick_offset + 1),
                "handle".to_string(),
                format!(
                    "0x{:x}:",
                    infra_qdisc_handle((queue + stick_offset + 1) as u16, InfraQdiscSlot::Default)
                ),
            ];
            default_class.extend(sqm_strings.clone());
            result.push(default_class);
        }
        MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);

        Some(result)
    }

    fn add_site(
        config: &Arc<lqos_config::Config>,
        params: AddSiteParams,
    ) -> Option<Vec<Vec<String>>> {
        if config.queues.queue_mode.is_observe() {
            return None;
        }

        let mut result = Vec::new();
        // Derive major IDs from parent handles so classids are fully qualified
        // and consistent with queuingStructure.json (classMajor/classMinor).
        let (down_major, _) = params.parent_class_id.get_major_minor();
        let (up_major, _) = params.up_parent_class_id.get_major_minor();

        /*
        bakery.add_site(data[node]['parentClassID'], data[node]['up_parentClassID'], data[node]['classMinor'], format_rate_for_tc(data[node]['downloadBandwidthMbpsMin']), format_rate_for_tc(data[node]['uploadBandwidthMbpsMin']), format_rate_for_tc(data[node]['downloadBandwidthMbps']), format_rate_for_tc(data[node]['uploadBandwidthMbps']), quantum(data[node]['downloadBandwidthMbps']), quantum(data[node]['uploadBandwidthMbps']))

        command = 'class add dev ' + interface_a() + ' parent ' + data[node]['parentClassID'] + ' classid ' + data[node]['classMinor'] + ' htb rate '+ format_rate_for_tc(data[node]['downloadBandwidthMbpsMin']) + ' ceil '+ format_rate_for_tc(data[node]['downloadBandwidthMbps']) + ' prio 3' + quantum(data[node]['downloadBandwidthMbps'])
        linuxTCcommands.append(command)
        logging.info("Up ParentClassID: " + data[node]['up_parentClassID'])
        logging.info("ClassMinor: " + data[node]['classMinor'])
        command = 'class add dev ' + interface_b() + ' parent ' + data[node]['up_parentClassID'] + ' classid ' + data[node]['classMinor'] + ' htb rate '+ format_rate_for_tc(data[node]['uploadBandwidthMbpsMin']) + ' ceil '+ format_rate_for_tc(data[node]['uploadBandwidthMbps']) + ' prio 3' + quantum(data[node]['uploadBandwidthMbps'])
                 */

        // Use 'replace' for idempotency: it adds when absent and updates when present.
        result.push(vec![
            "class".to_string(),
            "replace".to_string(),
            "dev".to_string(),
            config.isp_interface(),
            "parent".to_string(),
            params.parent_class_id.as_tc_string(),
            "classid".to_string(),
            format!("0x{:x}:0x{:x}", down_major, params.class_minor),
            "htb".to_string(),
            "rate".to_string(),
            format_rate_for_tc_f32(params.download_bandwidth_min),
            "ceil".to_string(),
            format_rate_for_tc_f32(params.download_bandwidth_max),
            "prio".to_string(),
            "3".to_string(),
            "quantum".to_string(),
            quantum(
                params.download_bandwidth_max as u64,
                r2q(config.queues.downlink_bandwidth_mbps),
            ),
        ]);
        result.push(vec![
            "class".to_string(),
            "replace".to_string(),
            "dev".to_string(),
            config.internet_interface(),
            "parent".to_string(),
            params.up_parent_class_id.as_tc_string(),
            "classid".to_string(),
            format!("0x{:x}:0x{:x}", up_major, params.class_minor),
            "htb".to_string(),
            "rate".to_string(),
            format_rate_for_tc_f32(params.upload_bandwidth_min),
            "ceil".to_string(),
            format_rate_for_tc_f32(params.upload_bandwidth_max),
            "prio".to_string(),
            "3".to_string(),
            "quantum".to_string(),
            quantum(
                params.upload_bandwidth_max as u64,
                r2q(config.queues.uplink_bandwidth_mbps),
            ),
        ]);

        Some(result)
    }

    fn add_circuit(
        execution_mode: ExecutionMode,
        config: &Arc<lqos_config::Config>,
        params: AddCircuitParams,
    ) -> Option<Vec<Vec<String>>> {
        if config.queues.queue_mode.is_observe() {
            return None;
        }

        if let Some(ref s) = params.sqm_override
            && s.eq_ignore_ascii_case("fq_codel")
        {
            debug!(
                "Bakery: building AddCircuit with fq_codel override (circuit_hash={}, class_minor=0x{:x}, class_major=0x{:x}, up_class_major=0x{:x})",
                params.circuit_hash, params.class_minor, params.class_major, params.up_class_major
            );
        }
        let do_htb;
        let do_sqm;

        if execution_mode == ExecutionMode::Builder {
            // Initial tree build: always create HTB + SQM classes for circuits,
            // regardless of lazy queue mode. Laziness applies to live updates
            // (ExecutionMode::LiveUpdate) and pruning, not the first full build.
            do_htb = true;
            do_sqm = true;
        } else {
            // We're in live update mode
            match config.queues.lazy_queues.as_ref() {
                None | Some(LazyQueueMode::No) => {
                    debug!("Builder should not encounter lazy updates when lazy is disabled!");
                    // Set both modes to false, avoiding clashes
                    do_htb = false;
                    do_sqm = false;
                }
                Some(LazyQueueMode::Htb) => {
                    // The HTB will already have been created, so we're just making the SQM
                    do_htb = false;
                    do_sqm = true;
                }
                Some(LazyQueueMode::Full) => {
                    // In full lazy mode, we only create the HTB and SQM if they don't exist
                    do_htb = true;
                    do_sqm = true;
                }
            }
        }

        // Parse per-direction override tokens: single token applies to both;
        // directional form is "down_sqm/up_sqm" with either side optionally empty.
        let (down_override_opt, up_override_opt) = match &params.sqm_override {
            None => (None, None),
            Some(s) => {
                if s.contains('/') {
                    let mut it = s.splitn(2, '/');
                    let down = it.next().unwrap_or("").trim();
                    let up = it.next().unwrap_or("").trim();
                    let map = |t: &str| -> Option<String> {
                        if t.is_empty() {
                            None
                        } else {
                            Some(t.to_string())
                        }
                    };
                    (map(down), map(up))
                } else {
                    (Some(s.clone()), Some(s.clone()))
                }
            }
        };

        let mut result = Vec::new();
        /*
        bakery.add_circuit(data[node]['classid'], data[node]['up_classid'], circuit['classMinor'], format_rate_for_tc(min_down), format_rate_for_tc(min_up), format_rate_for_tc(circuit['maxDownload']), format_rate_for_tc(circuit['maxUpload']), quantum(circuit['maxDownload']), quantum(circuit['maxUpload']), circuit['classMajor'], circuit['up_classMajor'], sqmFixupRate(circuit['maxDownload'], sqm()), sqmFixupRate(circuit['maxUpload'], sqm()), tcComment)
        command = 'class add dev ' + interface_a() + ' parent ' + data[node]['classid'] + ' classid ' + circuit['classMinor'] + ' htb rate '+ format_rate_for_tc(min_down) + ' ceil '+ format_rate_for_tc(circuit['maxDownload']) + ' prio 3' + quantum(circuit['maxDownload']) + tcComment
        linuxTCcommands.append(command)
        # Only add CAKE / fq_codel qdisc if monitorOnlyMode is Off
        if monitor_mode_only() == False:
            # SQM Fixup for lower rates
            useSqm = sqmFixupRate(circuit['maxDownload'], sqm())
            command = 'qdisc add dev ' + interface_a() + ' parent ' + circuit['classMajor'] + ':' + circuit['classMinor'] + ' ' + useSqm
            linuxTCcommands.append(command)
        command = 'class add dev ' + interface_b() + ' parent ' + data[node]['up_classid'] + ' classid ' + circuit['classMinor'] + ' htb rate '+ format_rate_for_tc(min_up) + ' ceil '+ format_rate_for_tc(circuit['maxUpload']) + ' prio 3' + quantum(circuit['maxUpload'])
        linuxTCcommands.append(command)
        # Only add CAKE / fq_codel qdisc if monitorOnlyMode is Off
        if monitor_mode_only() == False:
            # SQM Fixup for lower rates
            useSqm = sqmFixupRate(circuit['maxUpload'], sqm())
            command = 'qdisc add dev ' + interface_b() + ' parent ' + circuit['up_classMajor'] + ':' + circuit['classMinor'] + ' ' + useSqm
            linuxTCcommands.append(command)
            pass
         */
        if do_htb {
            // Use 'replace' for idempotency across repeated batches
            let verb = "replace";
            result.push(vec![
                "class".to_string(),
                verb.to_string(),
                "dev".to_string(),
                config.isp_interface(),
                "parent".to_string(),
                params.parent_class_id.as_tc_string(),
                "classid".to_string(),
                format!("0x{:x}:0x{:x}", params.class_major, params.class_minor),
                "htb".to_string(),
                "rate".to_string(),
                format_rate_for_tc_f32(params.download_bandwidth_min),
                "ceil".to_string(),
                format_rate_for_tc_f32(params.download_bandwidth_max),
                "prio".to_string(),
                "3".to_string(),
                "quantum".to_string(),
                quantum(
                    params.download_bandwidth_max as u64,
                    r2q(config.queues.downlink_bandwidth_mbps),
                ),
            ]);
        }
        if !config.queues.queue_mode.is_observe()
            && do_sqm
            && !matches!(down_override_opt.as_deref(), Some(s) if s.eq_ignore_ascii_case("none"))
        {
            let mut sqm_command = vec![
                "qdisc".to_string(),
                "replace".to_string(),
                "dev".to_string(),
                config.isp_interface(),
                "parent".to_string(),
                format!("0x{:x}:0x{:x}", params.class_major, params.class_minor),
            ];
            if let Some(handle) = params.down_qdisc_handle {
                sqm_command.push("handle".to_string());
                sqm_command.push(format!("0x{:x}:", handle));
            }
            sqm_command.extend(sqm_tokens_for(
                params.download_bandwidth_max,
                config,
                &down_override_opt,
            ));
            result.push(sqm_command);
        }

        if do_htb {
            // Use 'replace' for idempotency across repeated batches
            let verb = "replace";
            result.push(vec![
                "class".to_string(),
                verb.to_string(),
                "dev".to_string(),
                config.internet_interface(),
                "parent".to_string(),
                params.up_parent_class_id.as_tc_string(),
                "classid".to_string(),
                format!("0x{:x}:0x{:x}", params.up_class_major, params.class_minor),
                "htb".to_string(),
                "rate".to_string(),
                format_rate_for_tc_f32(params.upload_bandwidth_min),
                "ceil".to_string(),
                format_rate_for_tc_f32(params.upload_bandwidth_max),
                "prio".to_string(),
                "3".to_string(),
                "quantum".to_string(),
                quantum(
                    params.upload_bandwidth_max as u64,
                    r2q(config.queues.uplink_bandwidth_mbps),
                ),
            ]);
        }

        if !config.queues.queue_mode.is_observe()
            && do_sqm
            && !config.on_a_stick_mode()
            && !matches!(up_override_opt.as_deref(), Some(s) if s.eq_ignore_ascii_case("none"))
        {
            let mut sqm_command = vec![
                "qdisc".to_string(),
                "replace".to_string(),
                "dev".to_string(),
                config.internet_interface(),
                "parent".to_string(),
                format!("0x{:x}:0x{:x}", params.up_class_major, params.class_minor),
            ];
            if let Some(handle) = params.up_qdisc_handle {
                sqm_command.push("handle".to_string());
                sqm_command.push(format!("0x{:x}:", handle));
            }
            sqm_command.extend(sqm_tokens_for(
                params.upload_bandwidth_max,
                config,
                &up_override_opt,
            ));
            result.push(sqm_command);
        }

        Some(result)
    }

    /// Translate this circuit definition into `tc` deletions to prune it.
    ///
    /// Builds the sequence of `tc` argument lists to remove SQM qdiscs and/or
    /// HTB classes corresponding to this circuit. This only applies when
    /// `self` is `BakeryCommands::AddCircuit`; otherwise returns `None`.
    ///
    /// Behavior depends on `force` and the lazy-queue mode in `config`:
    /// - When `force` is `true`, both SQM qdiscs and HTB classes are removed.
    /// - When `force` is `false` and `LazyQueueMode::Htb`, only SQM is pruned.
    /// - When `force` is `false` and `LazyQueueMode::Full`, both are pruned.
    /// - If lazy queues are disabled, returns `None` (no pruning to do).
    ///
    /// Returns `Some(Vec<Vec<String>>)` of `tc` argv arrays in execution
    /// order, or `None` if no actions are required or the command is not a
    /// circuit.
    pub fn to_prune(
        &self,
        config: &Arc<lqos_config::Config>,
        force: bool, // Force removal of all classes and qdiscs to ensure removal.
    ) -> Option<Vec<Vec<String>>> {
        let BakeryCommands::AddCircuit {
            parent_class_id,
            up_parent_class_id,
            class_minor,
            class_major,
            up_class_major,
            sqm_override,
            ..
        } = self
        else {
            debug!("to_prune called on non-circuit command!");
            return None;
        };

        let prune_htb;
        let prune_sqm;
        let mut result = Vec::new();

        if force {
            prune_htb = true;
            prune_sqm = true;
        } else {
            match config.queues.lazy_queues.as_ref() {
                None | Some(LazyQueueMode::No) => {
                    debug!("Builder should not encounter lazy updates when lazy is disabled!");
                    // Set both modes to false, avoiding clashes
                    return None;
                }
                Some(LazyQueueMode::Htb) => {
                    // The HTB will already have been created, so we're just making the SQM
                    prune_htb = false;
                    prune_sqm = true;
                }
                Some(LazyQueueMode::Full) => {
                    // In full lazy mode, we only create the HTB and SQM if they don't exist
                    prune_htb = true;
                    prune_sqm = true;
                }
            }
        }

        if prune_sqm {
            // Determine per-direction pruning based on override tokens
            let (down_override_opt, up_override_opt) = match sqm_override.as_ref() {
                None => (None, None),
                Some(s) => {
                    if s.contains('/') {
                        let mut it = s.splitn(2, '/');
                        let down = it.next().unwrap_or("").trim();
                        let up = it.next().unwrap_or("").trim();
                        let map = |t: &str| -> Option<String> {
                            if t.is_empty() {
                                None
                            } else {
                                Some(t.to_string())
                            }
                        };
                        (map(down), map(up))
                    } else {
                        (Some(s.clone()), Some(s.clone()))
                    }
                }
            };

            let prune_down =
                !matches!(down_override_opt.as_deref(), Some(s) if s.eq_ignore_ascii_case("none"));
            let prune_up =
                !matches!(up_override_opt.as_deref(), Some(s) if s.eq_ignore_ascii_case("none"));

            if prune_up && !config.on_a_stick_mode() {
                result.push(vec![
                    "qdisc".to_string(),
                    "del".to_string(),
                    "dev".to_string(),
                    config.internet_interface(),
                    "parent".to_string(),
                    format!("0x{:x}:0x{:x}", up_class_major, class_minor),
                ]);
            }
            if prune_down {
                result.push(vec![
                    "qdisc".to_string(),
                    "del".to_string(),
                    "dev".to_string(),
                    config.isp_interface(),
                    "parent".to_string(),
                    format!("0x{:x}:0x{:x}", class_major, class_minor),
                ]);
            }
        }

        if prune_htb {
            // Prune the HTB class
            result.push(vec![
                "class".to_string(),
                "del".to_string(),
                "dev".to_string(),
                config.isp_interface(),
                "parent".to_string(),
                parent_class_id.as_tc_string(),
                "classid".to_string(),
                format!(
                    "0x{:x}:0x{:x}",
                    parent_class_id.get_major_minor().0,
                    class_minor
                ),
            ]);
            result.push(vec![
                "class".to_string(),
                "del".to_string(),
                "dev".to_string(),
                config.internet_interface(),
                "parent".to_string(),
                up_parent_class_id.as_tc_string(),
                "classid".to_string(),
                format!(
                    "0x{:x}:0x{:x}",
                    up_parent_class_id.get_major_minor().0,
                    class_minor
                ),
            ]);
        }

        Some(result)
    }
}

fn root_mq_add_command(interface_name: String) -> Vec<String> {
    vec![
        "qdisc".to_string(),
        "add".to_string(),
        "dev".to_string(),
        interface_name,
        "root".to_string(),
        "handle".to_string(),
        "7FFF:".to_string(),
        "mq".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::{BakeryCommands, ExecutionMode};
    use crate::MQ_CREATED;
    use crate::test_state_lock;
    use lqos_config::{Config, SingleInterfaceConfig};
    use std::sync::Arc;

    fn is_root_delete(cmd: &[String], interface: &str) -> bool {
        cmd.len() == 5
            && cmd[0] == "qdisc"
            && cmd[1] == "del"
            && cmd[2] == "dev"
            && cmd[3] == interface
            && cmd[4] == "root"
    }

    fn is_root_add_mq(cmd: &[String], interface: &str) -> bool {
        cmd.len() == 8
            && cmd[0] == "qdisc"
            && cmd[1] == "add"
            && cmd[2] == "dev"
            && cmd[3] == interface
            && cmd[4] == "root"
            && cmd[5] == "handle"
            && cmd[6] == "7FFF:"
            && cmd[7] == "mq"
    }

    #[test]
    fn mq_setup_bridge_mode_bootstraps_root_add_commands_when_runtime_state_is_empty() {
        let _guard = test_state_lock().lock().expect("lock");
        MQ_CREATED.store(false, std::sync::atomic::Ordering::Relaxed);
        let config = Arc::new(Config::default());
        let commands = BakeryCommands::MqSetup {
            queues_available: 1,
            stick_offset: 0,
        }
        .to_commands(&config, ExecutionMode::Builder)
        .expect("mq setup should emit commands");

        assert!(is_root_add_mq(&commands[0], &config.isp_interface()));
        let internet_add_idx = commands
            .iter()
            .position(|cmd| is_root_add_mq(cmd, &config.internet_interface()))
            .expect("expected internet-interface root add");
        assert!(
            internet_add_idx > 0,
            "internet-interface root add should occur after the ISP-interface root add"
        );
        MQ_CREATED.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn mq_setup_on_a_stick_emits_single_root_add_when_runtime_state_is_empty() {
        let _guard = test_state_lock().lock().expect("lock");
        MQ_CREATED.store(false, std::sync::atomic::Ordering::Relaxed);
        let config = Arc::new(Config {
            bridge: None,
            single_interface: Some(SingleInterfaceConfig::default()),
            ..Config::default()
        });

        let commands = BakeryCommands::MqSetup {
            queues_available: 1,
            stick_offset: 3,
        }
        .to_commands(&config, ExecutionMode::Builder)
        .expect("mq setup should emit commands");

        assert!(is_root_add_mq(&commands[0], &config.isp_interface()));
        assert_eq!(
            commands
                .iter()
                .filter(|cmd| is_root_add_mq(cmd, &config.isp_interface()))
                .count(),
            1,
            "expected a single root add on the shared interface"
        );
        MQ_CREATED.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn mq_setup_shape_mode_skips_root_add_when_runtime_state_is_ready() {
        let _guard = test_state_lock().lock().expect("lock");
        MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
        let config = Arc::new(Config::default());
        let commands = BakeryCommands::MqSetup {
            queues_available: 1,
            stick_offset: 0,
        }
        .to_commands(&config, ExecutionMode::Builder)
        .expect("mq setup should emit commands");

        assert!(
            commands
                .iter()
                .all(|cmd| !is_root_add_mq(cmd, &config.isp_interface()))
        );
        assert!(
            commands
                .iter()
                .all(|cmd| !is_root_add_mq(cmd, &config.internet_interface()))
        );
        MQ_CREATED.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn mq_setup_observe_mode_bootstraps_root_mq_when_runtime_state_is_empty() {
        let _guard = test_state_lock().lock().expect("lock");
        let mut cfg = Config::default();
        cfg.queues.set_queue_mode(lqos_config::QueueMode::Observe);
        let config = Arc::new(cfg);
        MQ_CREATED.store(false, std::sync::atomic::Ordering::Relaxed);

        let commands = BakeryCommands::MqSetup {
            queues_available: 1,
            stick_offset: 0,
        }
        .to_commands(&config, ExecutionMode::Builder)
        .expect("mq setup should emit commands in observe mode");

        assert!(
            commands
                .iter()
                .all(|cmd| !is_root_delete(cmd, &config.isp_interface()))
        );
        assert!(
            commands
                .iter()
                .all(|cmd| !is_root_delete(cmd, &config.internet_interface()))
        );
        assert!(
            commands
                .iter()
                .any(|cmd| is_root_add_mq(cmd, &config.isp_interface()))
        );
        assert!(
            commands
                .iter()
                .any(|cmd| is_root_add_mq(cmd, &config.internet_interface()))
        );
        assert_eq!(
            commands.len(),
            2,
            "observe mode should bootstrap root mq when runtime state is empty"
        );
        MQ_CREATED.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn mq_setup_observe_mode_reuses_retained_root_when_mq_exists() {
        let _guard = test_state_lock().lock().expect("lock");
        let mut cfg = Config::default();
        cfg.queues.set_queue_mode(lqos_config::QueueMode::Observe);
        let config = Arc::new(cfg);
        MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);

        let commands = BakeryCommands::MqSetup {
            queues_available: 2,
            stick_offset: 0,
        }
        .to_commands(&config, ExecutionMode::Builder)
        .expect("mq setup should emit commands in observe mode");

        assert!(
            commands
                .iter()
                .all(|cmd| !is_root_delete(cmd, &config.isp_interface()))
        );
        assert!(
            commands
                .iter()
                .all(|cmd| !is_root_delete(cmd, &config.internet_interface()))
        );
        assert!(
            commands
                .iter()
                .all(|cmd| !is_root_add_mq(cmd, &config.isp_interface()))
        );
        assert!(
            commands
                .iter()
                .all(|cmd| !is_root_add_mq(cmd, &config.internet_interface()))
        );
        assert_eq!(
            commands.len(),
            0,
            "observe mode should rely on full-reload preflight when the root mq already exists"
        );
        MQ_CREATED.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn mq_setup_observe_mode_reuses_retained_root_on_a_stick() {
        let _guard = test_state_lock().lock().expect("lock");
        let mut cfg = Config {
            bridge: None,
            single_interface: Some(SingleInterfaceConfig::default()),
            ..Config::default()
        };
        cfg.queues.set_queue_mode(lqos_config::QueueMode::Observe);
        let config = Arc::new(cfg);
        MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);

        let commands = BakeryCommands::MqSetup {
            queues_available: 2,
            stick_offset: 2,
        }
        .to_commands(&config, ExecutionMode::Builder)
        .expect("mq setup should emit commands in observe on-a-stick mode");

        assert!(
            commands
                .iter()
                .all(|cmd| !is_root_delete(cmd, &config.isp_interface()))
        );
        assert!(
            commands
                .iter()
                .all(|cmd| !is_root_add_mq(cmd, &config.isp_interface()))
        );
        assert_eq!(
            commands.len(),
            0,
            "observe mode should rely on full-reload preflight for retained on-a-stick roots"
        );
        MQ_CREATED.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}
