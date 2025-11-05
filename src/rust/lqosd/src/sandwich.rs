use lqos_config::{Config, SandwichMode, SandwichRateLimiter, BRIDGE_TO_INTERNET, BRIDGE_TO_NETWORK, SANDWICH_TO_INTERNET, SANDWICH_TO_INTERNET2, SANDWICH_TO_NETWORK, SANDWICH_TO_NETWORK2};
use tracing::{info, warn};
use std::process::Stdio;

/// Set up sandwich mode, if configured.
pub fn make_me_a_sandwich(config: &Config) -> anyhow::Result<bool> {
    // https://www.explainxkcd.com/wiki/index.php/149:_Sandwich
    let Some(bridge_config) = &config.bridge else {
        return Err(anyhow::anyhow!("Bridge mode required"));
    };
    let Some(SandwichMode::Full { with_rate_limiter, rate_override_mbps_down, rate_override_mbps_up, queue_override, use_fq_codel }) = &bridge_config.sandwich else {
        return Ok(false); // No sandwich mode, not an error
    };
    if !bridge_config.use_xdp_bridge {
        warn!("Sandwich mode requires XDP bridge. Running without sandwich mode.");
        return Ok(false);
    }
    info!("Enabling sandwich mode.");

    // Tear down all existing bridges and veth pairs
    cleanup_my_sandwich(config)?;

    // Create the veth pair
    let num_queues = queue_override.unwrap_or(lqos_sys::num_possible_cpus()? as usize);
    create_veth(SANDWICH_TO_INTERNET, SANDWICH_TO_INTERNET2, num_queues)?;
    create_veth(SANDWICH_TO_NETWORK, SANDWICH_TO_NETWORK2, num_queues)?;
    info!("Created veth pair {} <-> {}", SANDWICH_TO_INTERNET, SANDWICH_TO_INTERNET2);
    info!("Created veth pair {} <-> {}", SANDWICH_TO_NETWORK, SANDWICH_TO_NETWORK2);
    veth_up(SANDWICH_TO_INTERNET)?;
    veth_up(SANDWICH_TO_INTERNET2)?;
    veth_up(SANDWICH_TO_NETWORK)?;
    veth_up(SANDWICH_TO_NETWORK2)?;

    // Create the bridges on each end
    // Attach the veth ends to the bridges
    // Attach the physical interfaces to the bridges
    // Bring up all interfaces and bridges
    let to_internet = vec![SANDWICH_TO_INTERNET2, &bridge_config.to_internet];
    let to_network = vec![SANDWICH_TO_NETWORK2, &bridge_config.to_network];
    create_bridge(BRIDGE_TO_INTERNET, &to_internet)?;
    create_bridge(BRIDGE_TO_NETWORK, &to_network)?;

    // Attach rate limiters if requested
    match with_rate_limiter {
        SandwichRateLimiter::None => {}
        SandwichRateLimiter::Download => {
            let rate_mbps = rate_override_mbps_down.unwrap_or(config.queues.downlink_bandwidth_mbps);
            info!("Applying download rate limit of {} Mbps (HTB)", rate_mbps);
            create_egress_htb_limit(&bridge_config.to_network, rate_mbps, *use_fq_codel)?;
        }
        SandwichRateLimiter::Upload => {
            let rate_mbps = rate_override_mbps_up.unwrap_or(config.queues.uplink_bandwidth_mbps);
            info!("Applying upload rate limit of {} Mbps (HTB)", rate_mbps);
            create_egress_htb_limit(&bridge_config.to_internet, rate_mbps, *use_fq_codel)?;
        }
        SandwichRateLimiter::Both => {
            let down_rate_mbps = rate_override_mbps_down.unwrap_or(config.queues.downlink_bandwidth_mbps);
            let up_rate_mbps = rate_override_mbps_up.unwrap_or(config.queues.uplink_bandwidth_mbps);
            info!("Applying download rate limit of {} Mbps (HTB)", down_rate_mbps);
            info!("Applying upload rate limit of {} Mbps (HTB)", up_rate_mbps);
            create_egress_htb_limit(&bridge_config.to_network, down_rate_mbps, *use_fq_codel)?;
            create_egress_htb_limit(&bridge_config.to_internet, up_rate_mbps, *use_fq_codel)?;
        }
    }

    Ok(true)
}

/// Tear down sandwich mode, if configured.
pub fn cleanup_my_sandwich(config: &Config) -> anyhow::Result<()> {
    let Some(bridge) = &config.bridge else {
        return Err(anyhow::anyhow!("Bridge mode required"));
    };
    let Some(SandwichMode::Full { .. }) = &bridge.sandwich else {
        return Ok(()); // No sandwich mode, not an error
    };

    // Tear down all existing bridges and veth pairs
    // (ignore errors, they might not exist)
    if let Err(e) = delete_bridge(BRIDGE_TO_INTERNET) {
        info!("Ignoring error deleting existing bridge {}: {}", BRIDGE_TO_INTERNET, e);
    }
    if let Err(e) = delete_bridge(BRIDGE_TO_NETWORK) {
        info!("Ignoring error deleting existing bridge {}: {}", BRIDGE_TO_NETWORK, e);
    }
    if let Err(e) = delete_veth(SANDWICH_TO_INTERNET) {
        info!("Ignoring error deleting existing veth {}: {}", SANDWICH_TO_INTERNET, e);
    }
    if let Err(e) = delete_veth(SANDWICH_TO_NETWORK) {
        info!("Ignoring error deleting existing veth {}: {}", SANDWICH_TO_NETWORK, e);
    }
    Ok(())
}

fn delete_bridge(bridge_name: &str) -> anyhow::Result<()> {
    // ip link set dev $bridge_name down
    // ip link delete $bridge_name type bridge
    std::process::Command::new("ip")
        .args(&["link", "set", "dev", bridge_name, "down"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    std::process::Command::new("ip")
        .args(&["link", "delete", bridge_name, "type", "bridge"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(())
}

fn delete_veth(veth_name: &str) -> anyhow::Result<()> {
    // ip link set dev $veth_name down
    // ip link delete $veth_name type veth
    std::process::Command::new("ip")
        .args(&["link", "set", "dev", veth_name, "down"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    std::process::Command::new("ip")
        .args(&["link", "delete", veth_name, "type", "veth"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(())
}

fn create_veth(
    veth_name: &str,
    peer_name: &str,
    num_queues: usize,
) -> anyhow::Result<()> {
    // ip link add $VETH_NAME numrxqueues $NUM_QUEUES numtxqueues $NUM_QUEUES index 123 type veth peer name veth_toexternal numrxqueues $NUM_QUEUES numtxqueues $NUM_QUEUES index $INDEX
    let output = std::process::Command::new("ip")
        .args(&[
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
        ])
        .output()?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Failed to create veth {} <-> {} (exit={:?}): {}",
            veth_name,
            peer_name,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn veth_up(veth_name: &str) -> anyhow::Result<()> {
    // ip link set dev $veth_name up
    std::process::Command::new("ip")
        .args(&["link", "set", "dev", veth_name, "up"])
        .status()?;
    Ok(())
}

fn create_bridge(name: &str, members: &[&str]) -> anyhow::Result<()> {
    // ip link add name br0 type bridge
    std::process::Command::new("ip")
        .args(&["link", "add", "name", name, "type", "bridge"])
        .status()?;
    for member in members {
        std::process::Command::new("ip")
            .args(&["link", "set", member, "master", name])
            .status()?;
    }
    std::process::Command::new("ip")
        .args(&["link", "set", name, "up"])
        .status()?;
    Ok(())
}

fn create_egress_htb_limit(interface: &str, rate_mbps: u64, use_fq_codel: bool) -> anyhow::Result<()> {
    // Clean slate: delete existing root (ignore errors/noise)
    let _ = std::process::Command::new("tc")
        .args(&["qdisc", "del", "dev", interface, "root"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // Root HTB qdisc
    std::process::Command::new("tc")
        .args(&["qdisc", "add", "dev", interface, "root", "handle", "1:", "htb", "default", "1"])
        .status()?;

    // Single HTB class at the target rate with an explicit quantum around MTU
    let add_status = std::process::Command::new("tc")
        .args(&["class", "add", "dev", interface, "parent", "1:", "classid", "1:1", "htb",
                "rate", &format!("{}mbit", rate_mbps),
                "ceil", &format!("{}mbit", rate_mbps)])
        .status()?;
    if !add_status.success() {
        // Fallback to change if class already exists
        std::process::Command::new("tc")
            .args(&["class", "change", "dev", interface, "parent", "1:", "classid", "1:1", "htb",
                    "rate", &format!("{}mbit", rate_mbps),
                    "ceil", &format!("{}mbit", rate_mbps),
                    "quantum", "1514"])
            .status()?;
    }

    if use_fq_codel {
        // fq_codel as child of class 1:1
        std::process::Command::new("tc")
            .args(&["qdisc", "replace", "dev", interface, "parent", "1:1", "handle", "10:", "fq_codel"])
            .status()?;
    }
    Ok(())
}
