use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::{anyhow, Result};
use clap::{Args, Parser, Subcommand};

use lqos_config::ShapedDevice;
use lqos_overrides::{CircuitAdjustment, NetworkAdjustment, OverrideFile};

#[derive(Parser, Debug)]
#[command(name = "lqos_overrides")]
#[command(about = "Manage LibreQoS overrides", version, author)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Manage persistent shaped devices
    PersistentDevices {
        #[command(subcommand)]
        command: PersistentDevicesCommand,
    },
    /// Manage circuit/device adjustments
    Adjustments {
        #[command(subcommand)]
        command: AdjustmentsCommand,
    },
    /// Manage network (network.json) adjustments
    NetworkAdjustments {
        #[command(subcommand)]
        command: NetworkAdjustmentsCommand,
    },
    /// Manage UISP integration overrides (bandwidth, routes)
    Uisp {
        #[command(subcommand)]
        command: UispCommand,
    },
}

#[derive(Subcommand, Debug)]
enum PersistentDevicesCommand {
    /// Add a persistent shaped device
    Add(AddArgs),
    /// Delete all persistent devices with a circuit ID
    DeleteCircuitId { #[arg(long)] circuit_id: String },
    /// Delete persistent device(s) by device ID
    DeleteDeviceId { #[arg(long)] device_id: String },
    /// List current persistent devices
    List,
}

#[derive(Subcommand, Debug)]
enum AdjustmentsCommand {
    /// Add a circuit speed adjustment
    AddCircuitSpeed(AddCircuitSpeedArgs),
    /// Add a device speed adjustment
    AddDeviceSpeed(AddDeviceSpeedArgs),
    /// Add a circuit removal adjustment
    AddRemoveCircuit { #[arg(long)] circuit_id: String },
    /// Add a device removal adjustment
    AddRemoveDevice { #[arg(long)] device_id: String },
    /// Add a circuit reparent adjustment
    AddReparentCircuit { #[arg(long)] circuit_id: String, #[arg(long)] parent_node: String },
    /// Remove an adjustment by index (see list)
    DeleteIndex { #[arg(long)] index: usize },
    /// List current adjustments
    List,
}

#[derive(Subcommand, Debug)]
enum NetworkAdjustmentsCommand {
    /// Add site speed adjustment
    AddSiteSpeed(AddSiteSpeedArgs),
    /// Set whether a node is virtual (logical-only) in network.json
    SetVirtual {
        node_name: String,
        /// `true` marks the node as logical-only (omitted from the physical HTB tree).
        /// If omitted, defaults to `true`.
        #[arg(value_name = "VIRTUAL", default_value_t = true, action = clap::ArgAction::Set)]
        virtual_node: bool,
    },
    /// Remove any virtual-node override for a specific node name
    DeleteVirtual { node_name: String },
    /// Remove a network adjustment by index (see list)
    DeleteIndex { #[arg(long)] index: usize },
    /// List current network adjustments
    List,
}

#[derive(Subcommand, Debug)]
enum UispCommand {
    /// Set per-site bandwidth override
    BandwidthSet { #[arg(long)] site_name: String, #[arg(long)] down: f32, #[arg(long)] up: f32 },
    /// Remove a per-site bandwidth override
    BandwidthRemove { #[arg(long)] site_name: String },
    /// List bandwidth overrides
    BandwidthList,
    /// Add a route override
    RouteAdd { #[arg(long)] from_site: String, #[arg(long)] to_site: String, #[arg(long)] cost: u32 },
    /// Remove a route override by index
    RouteRemoveIndex { #[arg(long)] index: usize },
    /// List route overrides
    RouteList,
}

#[derive(Args, Debug)]
struct AddArgs {
    #[arg(long)]
    circuit_id: String,
    #[arg(long)]
    circuit_name: String,
    #[arg(long)]
    device_id: String,
    #[arg(long)]
    device_name: String,
    #[arg(long)]
    parent_node: String,
    #[arg(long)]
    mac: String,
    /// Repeat for multiple IPv4 entries (CIDR supported, default /32)
    #[arg(long = "ipv4")]
    ipv4_list: Vec<String>,
    /// Repeat for multiple IPv6 entries (CIDR supported, default /128)
    #[arg(long = "ipv6")]
    ipv6_list: Vec<String>,
    #[arg(long)]
    download_min_mbps: f32,
    #[arg(long)]
    upload_min_mbps: f32,
    #[arg(long)]
    download_max_mbps: f32,
    #[arg(long)]
    upload_max_mbps: f32,
    #[arg(long, default_value = "")]
    comment: String,
    /// Optional per-circuit SQM override token ("cake", "fq_codel", "none", or "down_sqm/up_sqm").
    /// A single token applies to both directions; empty means use defaults.
    #[arg(long, default_value = "")]
    sqm_override: String,
}

fn parse_ipv4(s: &str) -> Result<(Ipv4Addr, u32)> {
    if let Some((ip, prefix)) = s.split_once('/') {
        Ok((ip.parse()?, prefix.parse()?))
    } else {
        Ok((s.parse()?, 32))
    }
}

fn parse_ipv6(s: &str) -> Result<(Ipv6Addr, u32)> {
    if let Some((ip, prefix)) = s.split_once('/') {
        Ok((ip.parse()?, prefix.parse()?))
    } else {
        Ok((s.parse()?, 128))
    }
}

impl AddArgs {
    fn into_device(self) -> Result<ShapedDevice> {
        let ipv4 = self
            .ipv4_list
            .iter()
            .map(|s| parse_ipv4(s))
            .collect::<Result<Vec<_>>>()?;
        let ipv6 = self
            .ipv6_list
            .iter()
            .map(|s| parse_ipv6(s))
            .collect::<Result<Vec<_>>>()?;

        if self.download_min_mbps <= 0.0
            || self.upload_min_mbps <= 0.0
            || self.download_max_mbps <= 0.0
            || self.upload_max_mbps <= 0.0
        {
            return Err(anyhow!("Bandwidth values must be positive"));
        }

        let sqm_override = normalize_sqm_override(&self.sqm_override)?;

        Ok(ShapedDevice {
            circuit_id: self.circuit_id,
            circuit_name: self.circuit_name,
            device_id: self.device_id,
            device_name: self.device_name,
            parent_node: self.parent_node,
            mac: self.mac,
            ipv4,
            ipv6,
            download_min_mbps: self.download_min_mbps,
            upload_min_mbps: self.upload_min_mbps,
            download_max_mbps: self.download_max_mbps,
            upload_max_mbps: self.upload_max_mbps,
            comment: self.comment,
            sqm_override,
            circuit_hash: 0,
            device_hash: 0,
            parent_hash: 0,
        })
    }
}

fn normalize_sqm_override(raw: &str) -> Result<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let token = trimmed.to_ascii_lowercase();
    if token.contains('/') {
        let mut parts = token.splitn(2, '/');
        let down = parts.next().unwrap_or("").trim();
        let up = parts.next().unwrap_or("").trim();
        let valid = |s: &str| matches!(s, "" | "cake" | "fq_codel" | "none");
        if !valid(down) || !valid(up) {
            return Err(anyhow!(
                "invalid directional sqm override '{token}'. Allowed: 'cake', 'fq_codel', 'none', or 'down_sqm/up_sqm' (e.g. 'cake/fq_codel', '/none')"
            ));
        }
        return Ok(Some(format!("{down}/{up}")));
    }

    match token.as_str() {
        "cake" | "fq_codel" | "none" => Ok(Some(token)),
        _ => Err(anyhow!(
            "invalid sqm override '{token}'. Allowed values: 'cake', 'fq_codel', 'none', or 'down_sqm/up_sqm' (e.g. 'cake/fq_codel', '/none')"
        )),
    }
}

#[derive(Args, Debug, Default)]
struct AddCircuitSpeedArgs {
    #[arg(long)]
    circuit_id: String,
    #[arg(long)]
    min_download_bandwidth: Option<f32>,
    #[arg(long)]
    max_download_bandwidth: Option<f32>,
    #[arg(long)]
    min_upload_bandwidth: Option<f32>,
    #[arg(long)]
    max_upload_bandwidth: Option<f32>,
}

#[derive(Args, Debug, Default)]
struct AddDeviceSpeedArgs {
    #[arg(long)]
    device_id: String,
    #[arg(long)]
    min_download_bandwidth: Option<f32>,
    #[arg(long)]
    max_download_bandwidth: Option<f32>,
    #[arg(long)]
    min_upload_bandwidth: Option<f32>,
    #[arg(long)]
    max_upload_bandwidth: Option<f32>,
}

#[derive(Args, Debug, Default)]
struct AddSiteSpeedArgs {
    #[arg(long)]
    site_name: String,
    #[arg(long)]
    download_bandwidth_mbps: Option<u32>,
    #[arg(long)]
    upload_bandwidth_mbps: Option<u32>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load overrides file at start
    let mut overrides = OverrideFile::load()?;

    match cli.command {
        Commands::PersistentDevices { command: cmd } => match cmd {
            PersistentDevicesCommand::Add(args) => {
                let device = args.into_device()?;
                let changed = overrides.add_persistent_shaped_device_return_changed(device);
                if changed {
                    overrides.save()?;
                    println!("Added device; overrides saved.");
                } else {
                    println!("No changes (device already present).");
                }
            }
            PersistentDevicesCommand::DeleteCircuitId { circuit_id } => {
                let removed = overrides.remove_persistent_shaped_device_by_circuit_count(&circuit_id);
                if removed > 0 {
                    overrides.save()?;
                    println!("Removed {removed} device(s) by circuit_id; overrides saved.");
                } else {
                    println!("No devices matched circuit_id {circuit_id}.");
                }
            }
            PersistentDevicesCommand::DeleteDeviceId { device_id } => {
                let removed = overrides.remove_persistent_shaped_device_by_device_count(&device_id);
                if removed > 0 {
                    overrides.save()?;
                    println!("Removed {removed} device(s) by device_id; overrides saved.");
                } else {
                    println!("No devices matched device_id {device_id}.");
                }
            }
            PersistentDevicesCommand::List => {
                let list = overrides.persistent_devices();
                println!("{}", serde_json::to_string_pretty(&list)?);
            }
        },
        Commands::Adjustments { command: cmd } => match cmd {
            AdjustmentsCommand::AddCircuitSpeed(args) => {
                let adj = CircuitAdjustment::CircuitAdjustSpeed {
                    circuit_id: args.circuit_id,
                    min_download_bandwidth: args.min_download_bandwidth,
                    max_download_bandwidth: args.max_download_bandwidth,
                    min_upload_bandwidth: args.min_upload_bandwidth,
                    max_upload_bandwidth: args.max_upload_bandwidth,
                };
                overrides.add_circuit_adjustment(adj);
                overrides.save()?;
                println!("Added circuit speed adjustment; overrides saved.");
            }
            AdjustmentsCommand::AddDeviceSpeed(args) => {
                let adj = CircuitAdjustment::DeviceAdjustSpeed {
                    device_id: args.device_id,
                    min_download_bandwidth: args.min_download_bandwidth,
                    max_download_bandwidth: args.max_download_bandwidth,
                    min_upload_bandwidth: args.min_upload_bandwidth,
                    max_upload_bandwidth: args.max_upload_bandwidth,
                };
                overrides.add_circuit_adjustment(adj);
                overrides.save()?;
                println!("Added device speed adjustment; overrides saved.");
            }
            AdjustmentsCommand::AddRemoveCircuit { circuit_id } => {
                let adj = CircuitAdjustment::RemoveCircuit { circuit_id };
                overrides.add_circuit_adjustment(adj);
                overrides.save()?;
                println!("Added remove-circuit adjustment; overrides saved.");
            }
            AdjustmentsCommand::AddRemoveDevice { device_id } => {
                let adj = CircuitAdjustment::RemoveDevice { device_id };
                overrides.add_circuit_adjustment(adj);
                overrides.save()?;
                println!("Added remove-device adjustment; overrides saved.");
            }
            AdjustmentsCommand::AddReparentCircuit { circuit_id, parent_node } => {
                let adj = CircuitAdjustment::ReparentCircuit { circuit_id, parent_node };
                overrides.add_circuit_adjustment(adj);
                overrides.save()?;
                println!("Added reparent-circuit adjustment; overrides saved.");
            }
            AdjustmentsCommand::DeleteIndex { index } => {
                let ok = overrides.remove_circuit_adjustment_by_index(index);
                if ok {
                    overrides.save()?;
                    println!("Removed adjustment at index {index}; overrides saved.");
                } else {
                    println!("No adjustment at index {index}.");
                }
            }
            AdjustmentsCommand::List => {
                let list = overrides.circuit_adjustments();
                println!("{}", serde_json::to_string_pretty(&list)?);
            }
        },
        Commands::NetworkAdjustments { command: cmd } => match cmd {
            NetworkAdjustmentsCommand::AddSiteSpeed(args) => {
                let adj = NetworkAdjustment::AdjustSiteSpeed {
                    site_name: args.site_name,
                    download_bandwidth_mbps: args.download_bandwidth_mbps,
                    upload_bandwidth_mbps: args.upload_bandwidth_mbps,
                };
                overrides.add_network_adjustment(adj);
                overrides.save()?;
                println!("Added site speed adjustment; overrides saved.");
            }
            NetworkAdjustmentsCommand::SetVirtual { node_name, virtual_node } => {
                overrides.set_network_node_virtual(node_name, virtual_node);
                overrides.save()?;
                println!("Set node virtual flag; overrides saved.");
            }
            NetworkAdjustmentsCommand::DeleteVirtual { node_name } => {
                let removed = overrides.remove_network_node_virtual_by_name_count(&node_name);
                if removed > 0 {
                    overrides.save()?;
                    println!("Removed {removed} virtual override(s) for node '{node_name}'; overrides saved.");
                } else {
                    println!("No virtual override found for node '{node_name}'.");
                }
            }
            NetworkAdjustmentsCommand::DeleteIndex { index } => {
                let ok = overrides.remove_network_adjustment_by_index(index);
                if ok {
                    overrides.save()?;
                    println!("Removed network adjustment at index {index}; overrides saved.");
                } else {
                    println!("No network adjustment at index {index}.");
                }
            }
            NetworkAdjustmentsCommand::List => {
                let list = overrides.network_adjustments();
                println!("{}", serde_json::to_string_pretty(&list)?);
            }
        },
        Commands::Uisp { command: cmd } => match cmd {
            UispCommand::BandwidthSet { site_name, down, up } => {
                overrides.set_uisp_bandwidth_override(site_name, down, up);
                overrides.save()?;
                println!("Set UISP bandwidth override; overrides saved.");
            }
            UispCommand::BandwidthRemove { site_name } => {
                let removed = overrides.remove_uisp_bandwidth_override(&site_name);
                if removed {
                    overrides.save()?;
                    println!("Removed UISP bandwidth override for {site_name}; overrides saved.");
                } else {
                    println!("No UISP bandwidth override found for {site_name}.");
                }
            }
            UispCommand::BandwidthList => {
                if let Some(uisp) = overrides.uisp() {
                    println!("{}", serde_json::to_string_pretty(&uisp.bandwidth_overrides)?);
                } else {
                    println!("{}", "{}");
                }
            }
            UispCommand::RouteAdd { from_site, to_site, cost } => {
                overrides.add_uisp_route_override(from_site, to_site, cost);
                overrides.save()?;
                println!("Added UISP route override; overrides saved.");
            }
            UispCommand::RouteRemoveIndex { index } => {
                let removed = overrides.remove_uisp_route_by_index(index);
                if removed {
                    overrides.save()?;
                    println!("Removed UISP route override at index {index}; overrides saved.");
                } else {
                    println!("No UISP route override at index {index}.");
                }
            }
            UispCommand::RouteList => {
                if let Some(uisp) = overrides.uisp() {
                    println!("{}", serde_json::to_string_pretty(&uisp.route_overrides)?);
                } else {
                    println!("[]");
                }
            }
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_virtual_defaults_to_true() {
        let cli = Cli::try_parse_from([
            "lqos_overrides",
            "network-adjustments",
            "set-virtual",
            "CALVIN",
        ])
        .expect("CLI parse must succeed");

        match cli.command {
            Commands::NetworkAdjustments { command } => match command {
                NetworkAdjustmentsCommand::SetVirtual {
                    node_name,
                    virtual_node,
                } => {
                    assert_eq!(node_name, "CALVIN");
                    assert!(virtual_node);
                }
                other => panic!("unexpected network-adjustments command: {other:?}"),
            },
            other => panic!("unexpected top-level command: {other:?}"),
        }
    }

    #[test]
    fn set_virtual_accepts_explicit_false() {
        let cli = Cli::try_parse_from([
            "lqos_overrides",
            "network-adjustments",
            "set-virtual",
            "CALVIN",
            "false",
        ])
        .expect("CLI parse must succeed");

        match cli.command {
            Commands::NetworkAdjustments { command } => match command {
                NetworkAdjustmentsCommand::SetVirtual {
                    node_name,
                    virtual_node,
                } => {
                    assert_eq!(node_name, "CALVIN");
                    assert!(!virtual_node);
                }
                other => panic!("unexpected network-adjustments command: {other:?}"),
            },
            other => panic!("unexpected top-level command: {other:?}"),
        }
    }
}
