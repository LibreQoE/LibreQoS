use anyhow::{Context, Result, anyhow, bail};
use lqos_config::{
    BRIDGE_TO_INTERNET, BRIDGE_TO_NETWORK, Config, SANDWICH_TO_INTERNET, SANDWICH_TO_INTERNET2,
    SANDWICH_TO_NETWORK, SANDWICH_TO_NETWORK2, SandwichMode, SandwichRateLimiter,
};
use std::{
    path::Path,
    process::{Command, Stdio},
};
use tracing::info;

#[derive(Clone, Debug)]
pub struct SandwichTopology {
    physical_to_internet: String,
    physical_to_network: String,
    with_rate_limiter: SandwichRateLimiter,
    rate_override_mbps_down: Option<u64>,
    rate_override_mbps_up: Option<u64>,
    queue_override: Option<usize>,
    use_fq_codel: bool,
}

pub fn topology_from_config(config: &Config) -> Option<SandwichTopology> {
    let bridge = config.bridge.as_ref()?;
    let SandwichMode::Full {
        with_rate_limiter,
        rate_override_mbps_down,
        rate_override_mbps_up,
        queue_override,
        use_fq_codel,
    } = bridge.sandwich_mode()?
    else {
        return None;
    };

    Some(SandwichTopology {
        physical_to_internet: bridge.to_internet.clone(),
        physical_to_network: bridge.to_network.clone(),
        with_rate_limiter: with_rate_limiter.clone(),
        rate_override_mbps_down: *rate_override_mbps_down,
        rate_override_mbps_up: *rate_override_mbps_up,
        queue_override: *queue_override,
        use_fq_codel: *use_fq_codel,
    })
}

impl SandwichTopology {
    pub fn queue_count(&self) -> Result<usize> {
        Ok(self
            .queue_override
            .unwrap_or(lqos_sys::num_possible_cpus()? as usize))
    }

    pub fn lq_interfaces(&self) -> [&'static str; 2] {
        [SANDWICH_TO_INTERNET, SANDWICH_TO_NETWORK]
    }

    pub fn all_veth_interfaces(&self) -> [&'static str; 4] {
        [
            SANDWICH_TO_INTERNET,
            SANDWICH_TO_INTERNET2,
            SANDWICH_TO_NETWORK,
            SANDWICH_TO_NETWORK2,
        ]
    }

    pub fn bridges(&self) -> [&'static str; 2] {
        [BRIDGE_TO_INTERNET, BRIDGE_TO_NETWORK]
    }

    pub fn tuning_interfaces(&self) -> Vec<&str> {
        vec![
            SANDWICH_TO_INTERNET,
            SANDWICH_TO_INTERNET2,
            SANDWICH_TO_NETWORK,
            SANDWICH_TO_NETWORK2,
            self.physical_to_internet.as_str(),
            self.physical_to_network.as_str(),
        ]
    }

    fn download_rate_mbps(&self, config: &Config) -> u64 {
        self.rate_override_mbps_down
            .unwrap_or(config.queues.downlink_bandwidth_mbps)
    }

    fn upload_rate_mbps(&self, config: &Config) -> u64 {
        self.rate_override_mbps_up
            .unwrap_or(config.queues.uplink_bandwidth_mbps)
    }
}

/// Set up sandwich mode, if configured.
pub fn make_me_a_sandwich(config: &Config) -> Result<bool> {
    let Some(topology) = topology_from_config(config) else {
        return Ok(false);
    };
    let Some(bridge_config) = &config.bridge else {
        return Err(anyhow!("Bridge mode required"));
    };
    if !bridge_config.use_xdp_bridge {
        bail!("Sandwich mode requires bridge.use_xdp_bridge = true");
    }

    info!("Enabling sandwich mode.");
    cleanup_my_sandwich(config)?;

    let num_queues = topology.queue_count()?;
    create_veth(SANDWICH_TO_INTERNET, SANDWICH_TO_INTERNET2, num_queues)?;
    create_veth(SANDWICH_TO_NETWORK, SANDWICH_TO_NETWORK2, num_queues)?;
    for interface in topology.all_veth_interfaces() {
        set_link_up(interface)?;
    }

    create_bridge(
        BRIDGE_TO_INTERNET,
        &[
            SANDWICH_TO_INTERNET2,
            topology.physical_to_internet.as_str(),
        ],
    )?;
    create_bridge(
        BRIDGE_TO_NETWORK,
        &[SANDWICH_TO_NETWORK2, topology.physical_to_network.as_str()],
    )?;

    validate_veth_queue_counts(&topology, num_queues)?;

    match topology.with_rate_limiter {
        SandwichRateLimiter::None => {}
        SandwichRateLimiter::Download => {
            let rate_mbps = topology.download_rate_mbps(config);
            info!("Applying download rate limit of {} Mbps (HTB)", rate_mbps);
            create_egress_htb_limit(
                topology.physical_to_network.as_str(),
                rate_mbps,
                topology.use_fq_codel,
            )?;
        }
        SandwichRateLimiter::Upload => {
            let rate_mbps = topology.upload_rate_mbps(config);
            info!("Applying upload rate limit of {} Mbps (HTB)", rate_mbps);
            create_egress_htb_limit(
                topology.physical_to_internet.as_str(),
                rate_mbps,
                topology.use_fq_codel,
            )?;
        }
        SandwichRateLimiter::Both => {
            let down_rate_mbps = topology.download_rate_mbps(config);
            let up_rate_mbps = topology.upload_rate_mbps(config);
            info!(
                "Applying download rate limit of {} Mbps (HTB)",
                down_rate_mbps
            );
            info!("Applying upload rate limit of {} Mbps (HTB)", up_rate_mbps);
            create_egress_htb_limit(
                topology.physical_to_network.as_str(),
                down_rate_mbps,
                topology.use_fq_codel,
            )?;
            create_egress_htb_limit(
                topology.physical_to_internet.as_str(),
                up_rate_mbps,
                topology.use_fq_codel,
            )?;
        }
    }

    Ok(true)
}

/// Tear down sandwich mode, if configured.
pub fn cleanup_my_sandwich(config: &Config) -> Result<()> {
    let Some(topology) = topology_from_config(config) else {
        return Ok(());
    };

    for bridge in topology.bridges() {
        ignore_command_failure("ip", &["link", "set", "dev", bridge, "down"]);
        ignore_command_failure("ip", &["link", "delete", bridge, "type", "bridge"]);
    }
    ignore_command_failure(
        "ip",
        &["link", "delete", SANDWICH_TO_INTERNET, "type", "veth"],
    );
    ignore_command_failure(
        "ip",
        &["link", "delete", SANDWICH_TO_NETWORK, "type", "veth"],
    );

    Ok(())
}

fn validate_veth_queue_counts(topology: &SandwichTopology, expected: usize) -> Result<()> {
    for interface in topology.lq_interfaces() {
        let (rx, tx) = queue_counts(interface)?;
        if rx != expected || tx != expected {
            bail!("Sandwich interface {interface} has rx={rx} tx={tx}; expected {expected} queues");
        }
    }
    Ok(())
}

fn queue_counts(interface: &str) -> Result<(usize, usize)> {
    let path = format!("/sys/class/net/{interface}/queues");
    let sys_path = Path::new(&path);
    if !sys_path.exists() {
        bail!("Queue path {path} does not exist");
    }

    let mut counts = (0, 0);
    for entry in std::fs::read_dir(sys_path)? {
        let entry = entry?;
        if !entry.path().is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.starts_with("rx-") {
            counts.0 += 1;
        } else if name.starts_with("tx-") {
            counts.1 += 1;
        }
    }

    Ok(counts)
}

fn create_veth(veth_name: &str, peer_name: &str, num_queues: usize) -> Result<()> {
    run_command(
        "ip",
        &[
            "link",
            "add",
            veth_name,
            "numrxqueues",
            &num_queues.to_string(),
            "numtxqueues",
            &num_queues.to_string(),
            "type",
            "veth",
            "peer",
            "name",
            peer_name,
            "numrxqueues",
            &num_queues.to_string(),
            "numtxqueues",
            &num_queues.to_string(),
        ],
    )
}

fn set_link_up(interface: &str) -> Result<()> {
    run_command("ip", &["link", "set", "dev", interface, "up"])
}

fn create_bridge(name: &str, members: &[&str]) -> Result<()> {
    run_command("ip", &["link", "add", "name", name, "type", "bridge"])?;
    for member in members {
        run_command("ip", &["link", "set", member, "master", name])?;
        run_command("ip", &["link", "set", "dev", member, "up"])?;
    }
    run_command("ip", &["link", "set", "dev", name, "up"])
}

fn create_egress_htb_limit(interface: &str, rate_mbps: u64, use_fq_codel: bool) -> Result<()> {
    ignore_command_failure("tc", &["qdisc", "del", "dev", interface, "root"]);

    run_command(
        "tc",
        &[
            "qdisc", "add", "dev", interface, "root", "handle", "1:", "htb", "default", "1",
        ],
    )?;

    run_command(
        "tc",
        &[
            "class",
            "replace",
            "dev",
            interface,
            "parent",
            "1:",
            "classid",
            "1:1",
            "htb",
            "rate",
            &format!("{rate_mbps}mbit"),
            "ceil",
            &format!("{rate_mbps}mbit"),
            "quantum",
            "1514",
        ],
    )?;

    if use_fq_codel {
        run_command(
            "tc",
            &[
                "qdisc", "replace", "dev", interface, "parent", "1:1", "handle", "10:", "fq_codel",
            ],
        )?;
    }
    Ok(())
}

fn run_command(command: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(command)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("Failed to launch {command} {}", args.join(" ")))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(anyhow!(
            "{command} {} failed with exit {:?}: {}",
            args.join(" "),
            output.status.code(),
            stderr
        ))
    }
}

fn ignore_command_failure(command: &str, args: &[&str]) {
    if let Err(err) = run_command(command, args) {
        info!(
            "Ignoring cleanup error for {command} {}: {err}",
            args.join(" ")
        );
    }
}
