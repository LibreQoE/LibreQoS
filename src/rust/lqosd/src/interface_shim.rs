//! Builds the veth and Linux-bridge compatibility shim used when the selected
//! physical interfaces cannot host LibreQoS XDP programs directly.

use lqos_config::{
    Config, SHIM_INTERNET_BRIDGE, SHIM_INTERNET_LQOS, SHIM_INTERNET_PEER, SHIM_NETWORK_BRIDGE,
    SHIM_NETWORK_LQOS, SHIM_NETWORK_PEER,
};
use std::num::{ParseIntError, TryFromIntError};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use thiserror::Error;
use tracing::{debug, info, warn};

const IP_COMMAND: &str = "/bin/ip";

#[derive(Clone, Debug, PartialEq, Eq)]
struct ShimTopology {
    physical_to_internet: String,
    physical_to_network: String,
    queue_count: u32,
    mtu: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ShimPhysicalInterfaces {
    to_internet: String,
    to_network: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CommandSpec {
    program: &'static str,
    args: Vec<String>,
}

impl CommandSpec {
    fn ip(args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            program: IP_COMMAND,
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    fn display(&self) -> String {
        format!("{} {}", self.program, self.args.join(" "))
    }
}

/// Errors returned while planning or creating the interface compatibility shim.
#[derive(Debug, Error)]
pub(crate) enum CompatibilityShimError {
    /// The bridge configuration cannot form a safe shim topology.
    #[error("invalid interface compatibility shim configuration: {0}")]
    InvalidConfig(String),
    /// The selected shaping CPU count cannot be represented by iproute2.
    #[error("interface compatibility shim queue count is too large")]
    QueueCount(#[from] TryFromIntError),
    /// A physical interface MTU could not be read.
    #[error("unable to read MTU from {path}: {source}")]
    ReadMtu {
        /// Sysfs path that could not be read.
        path: PathBuf,
        /// Underlying filesystem error.
        source: std::io::Error,
    },
    /// A physical interface MTU was not a valid integer.
    #[error("unable to parse MTU from {path}: {source}")]
    ParseMtu {
        /// Sysfs path containing the invalid value.
        path: PathBuf,
        /// Integer parsing error.
        source: ParseIntError,
    },
    /// iproute2 could not be started.
    #[error("unable to run {command}: {source}")]
    Launch {
        /// Command that failed to launch.
        command: String,
        /// Underlying process-launch error.
        source: std::io::Error,
    },
    /// iproute2 returned a failure status.
    #[error("{command} failed with status {status}: {stderr}")]
    CommandFailed {
        /// Command that returned a failure status.
        command: String,
        /// Process status code, if the process reported one.
        status: String,
        /// Standard error emitted by iproute2.
        stderr: String,
    },
}

/// Owns the active compatibility shim and removes it when startup unwinds.
pub(crate) struct CompatibilityShimGuard {
    active: bool,
}

impl CompatibilityShimGuard {
    /// Returns whether this guard owns an active compatibility shim.
    pub(crate) fn is_active(&self) -> bool {
        self.active
    }
}

impl Drop for CompatibilityShimGuard {
    fn drop(&mut self) {
        if self.active {
            cleanup();
        }
    }
}

fn read_interface_mtu(interface: &str) -> Result<u32, CompatibilityShimError> {
    let path = PathBuf::from(format!("/sys/class/net/{interface}/mtu"));
    let raw = std::fs::read_to_string(&path).map_err(|source| CompatibilityShimError::ReadMtu {
        path: path.clone(),
        source,
    })?;
    raw.trim()
        .parse::<u32>()
        .map_err(|source| CompatibilityShimError::ParseMtu { path, source })
}

fn reserved_interface_names() -> [&'static str; 6] {
    [
        SHIM_INTERNET_LQOS,
        SHIM_INTERNET_PEER,
        SHIM_NETWORK_LQOS,
        SHIM_NETWORK_PEER,
        SHIM_INTERNET_BRIDGE,
        SHIM_NETWORK_BRIDGE,
    ]
}

fn planned_queue_count(
    config: &Config,
    shaping_cpu_count: usize,
) -> Result<u32, CompatibilityShimError> {
    let requested_queue_count = config
        .queues
        .override_available_queues
        .map(|count| count as usize)
        .unwrap_or(shaping_cpu_count);
    Ok(u32::try_from(
        requested_queue_count.min(shaping_cpu_count).max(2),
    )?)
}

fn physical_interfaces_from_config(
    config: &Config,
) -> Result<Option<ShimPhysicalInterfaces>, CompatibilityShimError> {
    let Some(bridge) = config.bridge.as_ref() else {
        return Ok(None);
    };
    if !bridge.compatibility_shim_enabled() {
        return Ok(None);
    }
    bridge
        .validate_compatibility_shim()
        .map_err(|message| CompatibilityShimError::InvalidConfig(message.to_string()))?;

    let physical_to_internet = bridge.to_internet.trim();
    let physical_to_network = bridge.to_network.trim();
    if physical_to_internet.is_empty() || physical_to_network.is_empty() {
        return Err(CompatibilityShimError::InvalidConfig(
            "both physical interface names are required".to_string(),
        ));
    }
    if physical_to_internet == physical_to_network {
        return Err(CompatibilityShimError::InvalidConfig(
            "physical interface names must be different".to_string(),
        ));
    }
    if reserved_interface_names()
        .iter()
        .any(|reserved| *reserved == physical_to_internet || *reserved == physical_to_network)
    {
        return Err(CompatibilityShimError::InvalidConfig(
            "physical interfaces cannot use LibreQoS shim device names".to_string(),
        ));
    }

    Ok(Some(ShimPhysicalInterfaces {
        to_internet: physical_to_internet.to_string(),
        to_network: physical_to_network.to_string(),
    }))
}

fn topology_with_mtu(
    config: &Config,
    physical: ShimPhysicalInterfaces,
    mut read_mtu: impl FnMut(&str) -> Result<u32, CompatibilityShimError>,
) -> Result<ShimTopology, CompatibilityShimError> {
    let shaping_cpu_count = lqos_config::detect_shaping_cpus(config)
        .shaping
        .len()
        .max(1);
    let queue_count = planned_queue_count(config, shaping_cpu_count)?;
    let mtu = read_mtu(&physical.to_internet)?.min(read_mtu(&physical.to_network)?);

    Ok(ShimTopology {
        physical_to_internet: physical.to_internet,
        physical_to_network: physical.to_network,
        queue_count,
        mtu,
    })
}

fn create_veth_commands(name: &str, peer: &str, queues: u32) -> Vec<CommandSpec> {
    let queue_count = queues.to_string();
    vec![CommandSpec::ip([
        "link",
        "add",
        "name",
        name,
        "numrxqueues",
        queue_count.as_str(),
        "numtxqueues",
        queue_count.as_str(),
        "type",
        "veth",
        "peer",
        "name",
        peer,
        "numrxqueues",
        queue_count.as_str(),
        "numtxqueues",
        queue_count.as_str(),
    ])]
}

fn set_link_value(device: &str, field: &str, value: impl ToString) -> CommandSpec {
    CommandSpec::ip([
        "link".to_string(),
        "set".to_string(),
        "dev".to_string(),
        device.to_string(),
        field.to_string(),
        value.to_string(),
    ])
}

fn set_link_up(device: &str) -> CommandSpec {
    CommandSpec::ip(["link", "set", "dev", device, "up"])
}

fn create_bridge_commands(name: &str, members: [&str; 2], mtu: u32) -> Vec<CommandSpec> {
    let mut commands = vec![
        CommandSpec::ip([
            "link",
            "add",
            "name",
            name,
            "type",
            "bridge",
            "stp_state",
            "0",
            "vlan_filtering",
            "0",
            "mcast_snooping",
            "0",
        ]),
        set_link_value(name, "mtu", mtu),
    ];
    for member in members {
        commands.push(set_link_value(member, "master", name));
        commands.push(set_link_up(member));
    }
    commands.push(set_link_up(name));
    commands
}

fn setup_commands(topology: &ShimTopology) -> Vec<CommandSpec> {
    let mut commands = Vec::new();
    commands.extend(create_veth_commands(
        SHIM_INTERNET_LQOS,
        SHIM_INTERNET_PEER,
        topology.queue_count,
    ));
    commands.extend(create_veth_commands(
        SHIM_NETWORK_LQOS,
        SHIM_NETWORK_PEER,
        topology.queue_count,
    ));
    for veth in [
        SHIM_INTERNET_LQOS,
        SHIM_INTERNET_PEER,
        SHIM_NETWORK_LQOS,
        SHIM_NETWORK_PEER,
    ] {
        commands.push(set_link_value(veth, "mtu", topology.mtu));
        commands.push(set_link_up(veth));
    }
    commands.extend(create_bridge_commands(
        SHIM_INTERNET_BRIDGE,
        [SHIM_INTERNET_PEER, topology.physical_to_internet.as_str()],
        topology.mtu,
    ));
    commands.extend(create_bridge_commands(
        SHIM_NETWORK_BRIDGE,
        [SHIM_NETWORK_PEER, topology.physical_to_network.as_str()],
        topology.mtu,
    ));
    commands
}

fn cleanup_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::ip(["link", "delete", SHIM_INTERNET_BRIDGE, "type", "bridge"]),
        CommandSpec::ip(["link", "delete", SHIM_NETWORK_BRIDGE, "type", "bridge"]),
        CommandSpec::ip(["link", "delete", SHIM_INTERNET_LQOS, "type", "veth"]),
        CommandSpec::ip(["link", "delete", SHIM_NETWORK_LQOS, "type", "veth"]),
    ]
}

fn run_checked(command: &CommandSpec) -> Result<(), CompatibilityShimError> {
    let output = Command::new(command.program)
        .args(&command.args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|source| CompatibilityShimError::Launch {
            command: command.display(),
            source,
        })?;
    if output.status.success() {
        return Ok(());
    }

    Err(CompatibilityShimError::CommandFailed {
        command: command.display(),
        status: output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "terminated by signal".to_string()),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

/// Removes any LibreQoS interface compatibility shim devices.
///
/// This function changes host network-device state. Missing devices and other
/// cleanup failures are logged and ignored so a later setup can recover.
pub(crate) fn cleanup() {
    cleanup_with(&mut run_checked);
}

fn cleanup_with(run_command: &mut impl FnMut(&CommandSpec) -> Result<(), CompatibilityShimError>) {
    for command in cleanup_commands() {
        match run_command(&command) {
            Ok(()) => debug!(command = %command.display(), "Removed compatibility shim device"),
            Err(error) => debug!(%error, "Compatibility shim cleanup command did not succeed"),
        }
    }
}

fn prepare_with(
    config: &Config,
    read_mtu: impl FnMut(&str) -> Result<u32, CompatibilityShimError>,
    mut run_command: impl FnMut(&CommandSpec) -> Result<(), CompatibilityShimError>,
) -> Result<bool, CompatibilityShimError> {
    let physical = physical_interfaces_from_config(config)?;
    cleanup_with(&mut run_command);
    let Some(physical) = physical else {
        return Ok(false);
    };
    let topology = topology_with_mtu(config, physical, read_mtu)?;

    info!(
        internet_interface = %topology.physical_to_internet,
        network_interface = %topology.physical_to_network,
        queues = topology.queue_count,
        mtu = topology.mtu,
        "Creating LibreQoS interface compatibility shim"
    );
    for command in setup_commands(&topology) {
        if let Err(error) = run_command(&command) {
            warn!(%error, "Unable to create interface compatibility shim; rolling back");
            cleanup_with(&mut run_command);
            return Err(error);
        }
    }

    Ok(true)
}

/// Removes stale shim devices, then creates the configured compatibility shim.
///
/// This function changes host network-device state by creating veth pairs,
/// Linux bridges, and bridge membership. The returned guard removes those
/// devices if startup unwinds or the daemon shuts down normally.
pub(crate) fn prepare(config: &Config) -> Result<CompatibilityShimGuard, CompatibilityShimError> {
    let active = prepare_with(config, read_interface_mtu, run_checked)?;
    Ok(CompatibilityShimGuard { active })
}

#[cfg(test)]
mod tests {
    use super::{
        CommandSpec, CompatibilityShimError, ShimTopology, cleanup_commands, planned_queue_count,
        prepare_with, setup_commands,
    };
    use lqos_config::{
        BridgeConfig, Config, SHIM_INTERNET_BRIDGE, SHIM_INTERNET_LQOS, SHIM_NETWORK_BRIDGE,
        SHIM_NETWORK_LQOS,
    };

    fn topology() -> ShimTopology {
        ShimTopology {
            physical_to_internet: "bond0".to_string(),
            physical_to_network: "enp2s0".to_string(),
            queue_count: 8,
            mtu: 9000,
        }
    }

    fn command_text(command: &CommandSpec) -> String {
        command.display()
    }

    fn enabled_config() -> Config {
        Config {
            bridge: Some(BridgeConfig {
                use_xdp_bridge: true,
                to_internet: "bond0".to_string(),
                to_network: "enp2s0".to_string(),
                compatibility_shim: true,
                ..BridgeConfig::default()
            }),
            ..Config::default()
        }
    }

    #[test]
    fn setup_plan_uses_two_multiqueue_veth_pairs_and_two_bridges() {
        let commands = setup_commands(&topology());
        let command_text = commands.iter().map(command_text).collect::<Vec<_>>();

        assert_eq!(
            command_text
                .iter()
                .filter(|command| command.contains(" type veth peer "))
                .count(),
            2
        );
        assert_eq!(
            command_text
                .iter()
                .filter(|command| command.contains(" type bridge "))
                .count(),
            2
        );
        assert!(
            command_text
                .iter()
                .filter(|command| command.contains(" type veth peer "))
                .all(|command| command.contains("numrxqueues 8 numtxqueues 8"))
        );
        assert!(command_text.iter().any(|command| {
            command.contains(SHIM_INTERNET_BRIDGE)
                && command.contains("stp_state 0")
                && command.contains("mcast_snooping 0")
        }));
        assert!(command_text.iter().any(|command| {
            command.contains(SHIM_NETWORK_BRIDGE)
                && command.contains("stp_state 0")
                && command.contains("mcast_snooping 0")
        }));
        assert!(command_text.iter().any(|command| {
            command.contains(SHIM_INTERNET_LQOS) && command.ends_with("mtu 9000")
        }));
        assert!(command_text.iter().any(|command| {
            command.contains(SHIM_NETWORK_LQOS) && command.ends_with("mtu 9000")
        }));
    }

    #[test]
    fn cleanup_removes_bridges_before_veth_pairs() {
        let commands = cleanup_commands();

        assert!(commands[0].display().contains(SHIM_INTERNET_BRIDGE));
        assert!(commands[1].display().contains(SHIM_NETWORK_BRIDGE));
        assert!(commands[2].display().contains(SHIM_INTERNET_LQOS));
        assert!(commands[3].display().contains(SHIM_NETWORK_LQOS));
    }

    #[test]
    fn queue_count_honors_the_smaller_of_override_and_shaping_cpus() {
        let mut config = enabled_config();
        config.queues.override_available_queues = Some(8);
        assert_eq!(planned_queue_count(&config, 4).expect("queue count"), 4);

        config.queues.override_available_queues = Some(3);
        assert_eq!(planned_queue_count(&config, 8).expect("queue count"), 3);

        config.queues.override_available_queues = None;
        assert_eq!(planned_queue_count(&config, 6).expect("queue count"), 6);
    }

    #[test]
    fn prepare_uses_the_smaller_physical_mtu() {
        let mut commands = Vec::new();
        let active = prepare_with(
            &enabled_config(),
            |interface| match interface {
                "bond0" => Ok(9000),
                "enp2s0" => Ok(1500),
                _ => unreachable!("unexpected interface"),
            },
            |command| {
                commands.push(command.display());
                Ok(())
            },
        )
        .expect("shim preparation should succeed");

        assert!(active);
        assert_eq!(
            commands[..4],
            cleanup_commands()
                .iter()
                .map(CommandSpec::display)
                .collect::<Vec<_>>()
        );
        assert!(
            commands
                .iter()
                .filter(|command| command.contains(" link set dev ") && command.contains(" mtu "))
                .all(|command| command.ends_with("mtu 1500"))
        );
    }

    #[test]
    fn prepare_rolls_back_after_partial_setup_failure() {
        let mut commands = Vec::new();
        let error = prepare_with(
            &enabled_config(),
            |_| Ok(1500),
            |command| {
                let display = command.display();
                commands.push(display.clone());
                if display.contains("link add name v_isp_lq") {
                    return Err(CompatibilityShimError::CommandFailed {
                        command: display,
                        status: "1".to_string(),
                        stderr: "simulated failure".to_string(),
                    });
                }
                Ok(())
            },
        )
        .expect_err("partial setup should fail");

        assert!(error.to_string().contains("simulated failure"));
        let expected_cleanup = cleanup_commands()
            .iter()
            .map(CommandSpec::display)
            .collect::<Vec<_>>();
        assert_eq!(&commands[..4], expected_cleanup.as_slice());
        assert_eq!(&commands[commands.len() - 4..], expected_cleanup.as_slice());
    }

    #[test]
    fn invalid_config_is_rejected_before_host_cleanup() {
        let mut config = enabled_config();
        config.bridge.as_mut().expect("bridge").to_internet = SHIM_INTERNET_LQOS.to_string();
        let mut commands = Vec::new();

        let error = prepare_with(
            &config,
            |_| unreachable!("MTU should not be read for invalid config"),
            |command| {
                commands.push(command.display());
                Ok(())
            },
        )
        .expect_err("reserved physical name should fail");

        assert!(
            error
                .to_string()
                .contains("cannot use LibreQoS shim device names")
        );
        assert!(commands.is_empty());
    }

    #[test]
    fn mtu_read_failure_still_removes_stale_devices() {
        let mut commands = Vec::new();
        let error = prepare_with(
            &enabled_config(),
            |interface| {
                Err(CompatibilityShimError::ReadMtu {
                    path: format!("/sys/class/net/{interface}/mtu").into(),
                    source: std::io::Error::other("simulated MTU read failure"),
                })
            },
            |command| {
                commands.push(command.display());
                Ok(())
            },
        )
        .expect_err("MTU read should fail");

        assert!(error.to_string().contains("simulated MTU read failure"));
        assert_eq!(
            commands,
            cleanup_commands()
                .iter()
                .map(CommandSpec::display)
                .collect::<Vec<_>>()
        );
    }
}
