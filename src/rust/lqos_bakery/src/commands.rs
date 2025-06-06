use std::collections::HashSet;
use std::sync::Arc;
use tracing::warn;
use lqos_bus::TcHandle;
use lqos_config::LazyQueueMode;
use crate::queue_math::{format_rate_for_tc, format_rate_for_tc_f32, quantum, r2q, sqm_as_vec, sqm_rate_fixup};

/// Execution Mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// We're constructing the tree
    Builder,
    /// Live Update
    LiveUpdate,
}

/// List of commands that the Bakery system can handle.
#[derive(Debug, Clone)]
pub enum BakeryCommands {
    OnCircuitActivity { circuit_ids: HashSet<i64> },
    Tick,
    ChangeSiteSpeedLive {
        site_hash: i64,
        download_bandwidth_min: f32,
        upload_bandwidth_min: f32,
        download_bandwidth_max: f32,
        upload_bandwidth_max: f32,
    },
    StartBatch,
    CommitBatch,
    MqSetup { queues_available: usize, stick_offset: usize },
    AddSite {
        site_hash: i64,
        parent_class_id: TcHandle,
        up_parent_class_id: TcHandle,
        class_minor: u16,
        download_bandwidth_min: f32,
        upload_bandwidth_min: f32,
        download_bandwidth_max: f32,
        upload_bandwidth_max: f32,
    },
    AddCircuit {
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
    }
}

impl BakeryCommands {
    pub fn to_commands(&self, config: &Arc<lqos_config::Config>, execution_mode: ExecutionMode) -> Option<Vec<Vec<String>>> {
        match self {
            BakeryCommands::MqSetup { queues_available, stick_offset } => Self::mq_setup(config, *queues_available, *stick_offset),
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
                config, *site_hash, *parent_class_id, *up_parent_class_id,
                *class_minor, *download_bandwidth_min,
                *upload_bandwidth_min, *download_bandwidth_max,
                *upload_bandwidth_max,
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
            } => Self::add_circuit(
                execution_mode,
                config, *circuit_hash, *parent_class_id, *up_parent_class_id,
                *class_minor, *download_bandwidth_min,
                *upload_bandwidth_min, *download_bandwidth_max,
                *upload_bandwidth_max, *class_major, *up_class_major,
            ),
            _ => None,
        }
    }

    fn mq_setup(
        config: &Arc<lqos_config::Config>,
        queues_available: usize,
        stick_offset: usize,
    ) -> Option<Vec<Vec<String>>> {
        // command = 'qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq'
        let mut result = Vec::new();
        let sqm_strings = sqm_as_vec(config);
        let r2q = r2q(u64::max(config.queues.uplink_bandwidth_mbps, config.queues.downlink_bandwidth_mbps));

        // ISP-facing interface (interface_a in Python)
        result.push(vec![
            "qdisc".to_string(), "replace".to_string(), "dev".to_string(),
            config.isp_interface(),
            "root".to_string(), "handle".to_string(), "7FFF:".to_string(),
            "mq".to_string(),
        ]);

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

        for queue in 0 .. queues_available {
            // command = 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+1) + ' handle ' + hex(queue+1) + ': htb default 2'
            result.push(vec![
                "qdisc".to_string(), "replace".to_string(), "dev".to_string(),
                config.isp_interface(),
                "parent".to_string(), format!("7FFF:0x{:x}", queue + 1),
                "handle".to_string(), format!("0x{:x}:", queue + 1), "htb".to_string(),
                "default".to_string(), "2".to_string(),
            ]);
            // command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ': classid ' + hex(queue+1) + ':1 htb rate '+ format_rate_for_tc(upstream_bandwidth_capacity_download_mbps()) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_download_mbps()) + quantum(upstream_bandwidth_capacity_download_mbps())
            result.push(vec![
                "class".to_string(), "replace".to_string(), "dev".to_string(),
                config.isp_interface(),
                "parent".to_string(), format!("0x{:x}:", queue + 1),
                "classid".to_string(), format!("0x{:x}:1", queue + 1), "htb".to_string(),
                "rate".to_string(), format_rate_for_tc(config.queues.uplink_bandwidth_mbps),
                "ceil".to_string(), format_rate_for_tc(config.queues.uplink_bandwidth_mbps),
                "quantum".to_string(), quantum(config.queues.uplink_bandwidth_mbps, r2q),
            ]);
            // command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 ' + sqm()
            let mut class = vec![
                "qdisc".to_string(), "replace".to_string(), "dev".to_string(),
                config.isp_interface(),
                "parent".to_string(), format!("0x{:x}:1", queue + 1),
            ];
            class.extend(sqm_strings.clone());
            result.push(class);

            // Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
            // command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 classid ' + hex(queue+1) + ':2 htb rate ' + format_rate_for_tc(round((upstream_bandwidth_capacity_download_mbps()-1)/4)) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_download_mbps()-1) + ' prio 5' + quantum(upstream_bandwidth_capacity_download_mbps())
            let mbps = config.queues.uplink_bandwidth_mbps as f64;
            let mbps_quarter = (mbps - 1.0) / 4.0;
            let mbps_minus_one = mbps - 1.0;
            result.push(vec![
                "class".to_string(), "replace".to_string(), "dev".to_string(),
                config.isp_interface(),
                "parent".to_string(), format!("0x{:x}:1", queue + 1),
                "classid".to_string(), format!("0x{:x}:2", queue + 1), "htb".to_string(),
                "rate".to_string(), format_rate_for_tc(mbps_quarter as u64),
                "ceil".to_string(), format_rate_for_tc(mbps_minus_one as u64),
                "prio".to_string(), "5".to_string(),
                "quantum".to_string(), quantum(config.queues.uplink_bandwidth_mbps, r2q),
            ]);
            // command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':2 ' + sqm()
            let mut default_class = vec![
                "qdisc".to_string(), "replace".to_string(), "dev".to_string(),
                config.isp_interface(),
                "parent".to_string(), format!("0x{:x}:2", queue + 1),
            ];
            default_class.extend(sqm_strings.clone());
            result.push(default_class);
        }

        // Internet-facing interface (interface_b in Python)
        if !config.on_a_stick_mode() {
            result.push(vec![
                "qdisc".to_string(), "replace".to_string(), "dev".to_string(),
                config.internet_interface(),
                "root".to_string(), "handle".to_string(), "7FFF:".to_string(),
                "mq".to_string(),
            ]);
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
        for queue in 0 .. queues_available {
            // command = 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+stickOffset+1) + ' handle ' + hex(queue+stickOffset+1) + ': htb default 2'
            result.push(vec![
                "qdisc".to_string(), "replace".to_string(), "dev".to_string(),
                config.internet_interface(),
                "parent".to_string(), format!("7FFF:0x{:x}", queue + stick_offset + 1),
                "handle".to_string(), format!("0x{:x}:", queue + stick_offset + 1), "htb".to_string(),
                "default".to_string(), "2".to_string(),
            ]);
            // command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ': classid ' + hex(queue+stickOffset+1) + ':1 htb rate '+ format_rate_for_tc(upstream_bandwidth_capacity_upload_mbps()) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_upload_mbps()) + quantum(upstream_bandwidth_capacity_upload_mbps())
            result.push(vec![
                "class".to_string(), "replace".to_string(), "dev".to_string(),
                config.internet_interface(),
                "parent".to_string(), format!("0x{:x}:", queue + stick_offset + 1),
                "classid".to_string(), format!("0x{:x}:1", queue + stick_offset + 1), "htb".to_string(),
                "rate".to_string(), format_rate_for_tc(config.queues.downlink_bandwidth_mbps),
                "ceil".to_string(), format_rate_for_tc(config.queues.downlink_bandwidth_mbps),
                "quantum".to_string(), quantum(config.queues.downlink_bandwidth_mbps, r2q),
            ]);
            // command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':1 ' + sqm()
            let mut class = vec![
                "qdisc".to_string(), "replace".to_string(), "dev".to_string(),
                config.internet_interface(),
                "parent".to_string(), format!("0x{:x}:1", queue + stick_offset + 1),
            ];
            class.extend(sqm_strings.clone());
            result.push(class);
            // Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
            // command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':1 classid ' + hex(queue+stickOffset+1) + ':2 htb rate ' + format_rate_for_tc(round((upstream_bandwidth_capacity_upload_mbps()-1)/4)) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_upload_mbps()-1) + ' prio 5' + quantum(upstream_bandwidth_capacity_upload_mbps())
            let mbps = config.queues.downlink_bandwidth_mbps as f64;
            let mbps_quarter = (mbps - 1.0) / 4.0;
            let mbps_minus_one = mbps - 1.0;
            result.push(vec![
                "class".to_string(), "replace".to_string(), "dev".to_string(),
                config.internet_interface(),
                "parent".to_string(), format!("0x{:x}:1", queue + stick_offset + 1),
                "classid".to_string(), format!("0x{:x}:2", queue + stick_offset + 1), "htb".to_string(),
                "rate".to_string(), format_rate_for_tc(mbps_quarter as u64),
                "ceil".to_string(), format_rate_for_tc(mbps_minus_one as u64),
                "prio".to_string(), "5".to_string(),
                "quantum".to_string(), quantum(config.queues.downlink_bandwidth_mbps, r2q),
            ]);
            // command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':2 ' + sqm()
            let mut default_class = vec![
                "qdisc".to_string(), "replace".to_string(), "dev".to_string(),
                config.internet_interface(),
                "parent".to_string(), format!("0x{:x}:2", queue + stick_offset + 1),
            ];
            default_class.extend(sqm_strings.clone());
            result.push(default_class);
        }

        Some(result)
    }

    fn add_site(
        config: &Arc<lqos_config::Config>,
        _site_hash: i64,
        parent_class_id: TcHandle,
        up_parent_class_id: TcHandle,
        class_minor: u16,
        download_bandwidth_min: f32,
        upload_bandwidth_min: f32,
        download_bandwidth_max: f32,
        upload_bandwidth_max: f32,
    ) -> Option<Vec<Vec<String>>> {
        let mut result = Vec::new();

        /*
bakery.add_site(data[node]['parentClassID'], data[node]['up_parentClassID'], data[node]['classMinor'], format_rate_for_tc(data[node]['downloadBandwidthMbpsMin']), format_rate_for_tc(data[node]['uploadBandwidthMbpsMin']), format_rate_for_tc(data[node]['downloadBandwidthMbps']), format_rate_for_tc(data[node]['uploadBandwidthMbps']), quantum(data[node]['downloadBandwidthMbps']), quantum(data[node]['uploadBandwidthMbps']))

command = 'class add dev ' + interface_a() + ' parent ' + data[node]['parentClassID'] + ' classid ' + data[node]['classMinor'] + ' htb rate '+ format_rate_for_tc(data[node]['downloadBandwidthMbpsMin']) + ' ceil '+ format_rate_for_tc(data[node]['downloadBandwidthMbps']) + ' prio 3' + quantum(data[node]['downloadBandwidthMbps'])
linuxTCcommands.append(command)
logging.info("Up ParentClassID: " + data[node]['up_parentClassID'])
logging.info("ClassMinor: " + data[node]['classMinor'])
command = 'class add dev ' + interface_b() + ' parent ' + data[node]['up_parentClassID'] + ' classid ' + data[node]['classMinor'] + ' htb rate '+ format_rate_for_tc(data[node]['uploadBandwidthMbpsMin']) + ' ceil '+ format_rate_for_tc(data[node]['uploadBandwidthMbps']) + ' prio 3' + quantum(data[node]['uploadBandwidthMbps'])
         */

        result.push(vec![
            "class".to_string(), "replace".to_string(), "dev".to_string(), config.isp_interface(),
            "parent".to_string(), parent_class_id.as_tc_string(),
            "classid".to_string(), format!("0x{class_minor:x}"), "htb".to_string(),
            "rate".to_string(), format_rate_for_tc_f32(download_bandwidth_min),
            "ceil".to_string(), format_rate_for_tc_f32(download_bandwidth_max),
            "prio".to_string(), "3".to_string(),
            "quantum".to_string(), quantum(
                download_bandwidth_max as u64,
                r2q(config.queues.downlink_bandwidth_mbps),
            ),
        ]);
        result.push(vec![
            "class".to_string(), "replace".to_string(), "dev".to_string(), config.internet_interface(),
            "parent".to_string(), up_parent_class_id.as_tc_string(),
            "classid".to_string(), format!("0x{class_minor:x}"),
            "htb".to_string(), "rate".to_string(), format_rate_for_tc_f32(upload_bandwidth_min),
            "ceil".to_string(), format_rate_for_tc_f32(upload_bandwidth_max),
            "prio".to_string(), "3".to_string(),
            "quantum".to_string(), quantum(
                upload_bandwidth_max as u64,
                r2q(config.queues.uplink_bandwidth_mbps),
            ),
        ]);

        Some(result)
    }

    fn add_circuit(
        execution_mode: ExecutionMode,
        config: &Arc<lqos_config::Config>,
        _circuit_hash: i64,
        parent_class_id: TcHandle,
        up_parent_class_id: TcHandle,
        class_minor: u16,
        download_bandwidth_min: f32,
        upload_bandwidth_min: f32,
        download_bandwidth_max: f32,
        upload_bandwidth_max: f32,
        class_major: u16,
        up_class_major: u16,
    ) -> Option<Vec<Vec<String>>> {
        let do_htb ;
        let do_sqm ;

        if execution_mode == ExecutionMode::Builder {
            // In builder mode, if we're fully lazy - we don't do anything.
            match config.queues.lazy_queues.as_ref() {
                None | Some(LazyQueueMode::No) => {
                    do_htb = true;
                    do_sqm = true;
                },
                Some(LazyQueueMode::Full) => return None,
                Some(LazyQueueMode::Htb) => {
                    do_htb = true;
                    do_sqm = false; // Only HTB, no SQM
                }
            }
        } else {
            // We're in live update mode
            match config.queues.lazy_queues.as_ref() {
                None | Some(LazyQueueMode::No) => {
                    warn!("Builder should not encounter lazy updates when lazy is disabled!");
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
            result.push(vec![
                "class".to_string(), "replace".to_string(), "dev".to_string(), config.isp_interface(),
                "parent".to_string(), parent_class_id.as_tc_string(),
                "classid".to_string(), format!("0x{class_minor:x}"), "htb".to_string(),
                "rate".to_string(), format_rate_for_tc_f32(download_bandwidth_min),
                "ceil".to_string(), format_rate_for_tc_f32(download_bandwidth_max),
                "prio".to_string(), "3".to_string(),
                "quantum".to_string(), quantum(
                    download_bandwidth_max as u64,
                    r2q(config.queues.downlink_bandwidth_mbps),
                ),
            ]);
        }
        if !config.queues.monitor_only && do_sqm {
            let mut sqm_command = vec![
                "qdisc".to_string(), "replace".to_string(), "dev".to_string(),
                config.isp_interface(),
                "parent".to_string(), format!("0x{:x}:0x{:x}", class_major, class_minor),
            ];
            sqm_command.extend(sqm_rate_fixup(download_bandwidth_max, config));
            result.push(sqm_command);
        }

        if do_htb {
            result.push(vec![
                "class".to_string(), "replace".to_string(), "dev".to_string(), config.internet_interface(),
                "parent".to_string(), up_parent_class_id.as_tc_string(),
                "classid".to_string(), format!("0x{class_minor:x}"),
                "htb".to_string(), "rate".to_string(), format_rate_for_tc_f32(upload_bandwidth_min),
                "ceil".to_string(), format_rate_for_tc_f32(upload_bandwidth_max),
                "prio".to_string(), "3".to_string(),
                "quantum".to_string(), quantum(
                    upload_bandwidth_max as u64,
                    r2q(config.queues.uplink_bandwidth_mbps),
                ),
            ]);
        }

        if !config.queues.monitor_only && do_sqm {
            let mut sqm_command = vec![
                "qdisc".to_string(), "replace".to_string(), "dev".to_string(),
                config.internet_interface(),
                "parent".to_string(), format!("0x{:x}:0x{:x}", up_class_major, class_minor),
            ];
            sqm_command.extend(sqm_rate_fixup(upload_bandwidth_max, config));
            result.push(sqm_command);
        }

        Some(result)
    }

    pub fn to_prune(
        &self,
        config: &Arc<lqos_config::Config>,
        force: bool, // Force removal of all classes and qdiscs to ensure removal.
    ) -> Option<Vec<Vec<String>>> {
        let BakeryCommands::AddCircuit { parent_class_id, up_parent_class_id, class_minor, class_major, up_class_major, .. } = self else {
            warn!("to_prune called on non-circuit command!");
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
                    warn!("Builder should not encounter lazy updates when lazy is disabled!");
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
            // Prune the SQM qdisc
            if !config.on_a_stick_mode() {
                result.push(vec![
                    "qdisc".to_string(), "del".to_string(), "dev".to_string(), config.internet_interface(),
                    "parent".to_string(), format!("0x{:x}:0x{:x}", up_class_major, class_minor),
                ]);
            }
            result.push(vec![
                "qdisc".to_string(), "del".to_string(), "dev".to_string(), config.isp_interface(),
                "parent".to_string(), format!("0x{:x}:0x{:x}", class_major, class_minor),
            ]);
        }

        if prune_htb {
            // Prune the HTB class
            result.push(vec![
                "class".to_string(), "del".to_string(), "dev".to_string(), config.isp_interface(),
                "parent".to_string(), parent_class_id.as_tc_string(),
                "classid".to_string(), format!("0x{class_minor:x}"),
            ]);
            result.push(vec![
                "class".to_string(), "del".to_string(), "dev".to_string(), config.internet_interface(),
                "parent".to_string(), up_parent_class_id.as_tc_string(),
                "classid".to_string(), format!("0x{class_minor:x}"),
            ]);
        }

        Some(result)
    }
}