use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::{anyhow, Result};
use clap::{Args, Parser, Subcommand};

use lqos_config::ShapedDevice;
use lqos_overrides::OverrideFile;

#[derive(Parser, Debug)]
#[command(name = "lqos_overrides")]
#[command(about = "Manage LibreQoS overrides", version, author)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Manage appended shaped devices
    AppendDevices {
        #[command(subcommand)]
        command: AppendDevicesCommand,
    },
}

#[derive(Subcommand, Debug)]
enum AppendDevicesCommand {
    /// Add a shaped device to append list
    Add(AddArgs),
    /// Delete all appended devices with a circuit ID
    DeleteCircuitId { #[arg(long)] circuit_id: String },
    /// Delete appended device(s) by device ID
    DeleteDeviceId { #[arg(long)] device_id: String },
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
            circuit_hash: 0,
            device_hash: 0,
            parent_hash: 0,
        })
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load overrides file at start
    let mut overrides = OverrideFile::load()?;

    match cli.command {
        Commands::AppendDevices { command: cmd } => match cmd {
            AppendDevicesCommand::Add(args) => {
                let device = args.into_device()?;
                let changed = overrides.add_append_shaped_device_return_changed(device);
                if changed {
                    overrides.save()?;
                    println!("Added device; overrides saved.");
                } else {
                    println!("No changes (device already present).");
                }
            }
            AppendDevicesCommand::DeleteCircuitId { circuit_id } => {
                let removed = overrides.remove_append_shaped_device_by_circuit_count(&circuit_id);
                if removed > 0 {
                    overrides.save()?;
                    println!("Removed {removed} device(s) by circuit_id; overrides saved.");
                } else {
                    println!("No devices matched circuit_id {circuit_id}.");
                }
            }
            AppendDevicesCommand::DeleteDeviceId { device_id } => {
                let removed = overrides.remove_append_shaped_device_by_device_count(&device_id);
                if removed > 0 {
                    overrides.save()?;
                    println!("Removed {removed} device(s) by device_id; overrides saved.");
                } else {
                    println!("No devices matched device_id {device_id}.");
                }
            }
        },
    }

    Ok(())
}
