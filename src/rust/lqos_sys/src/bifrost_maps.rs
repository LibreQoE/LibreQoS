use crate::{bpf_map::BpfMap, lqos_kernel::interface_name_to_index};
use anyhow::Result;
use log::info;
use lqos_config::{BridgeInterface, BridgeVlan};

#[repr(C)]
#[derive(Default, Clone, Debug)]
struct BifrostInterface {
  redirect_to: u32,
  scan_vlans: u32,
}

#[repr(C)]
#[derive(Default, Clone, Debug)]
struct BifrostVlan {
  redirect_to: u32,
}

const INTERFACE_PATH: &str = "/sys/fs/bpf/bifrost_interface_map";
const VLAN_PATH: &str = "/sys/fs/bpf/bifrost_vlan_map";

pub(crate) fn clear_bifrost() -> Result<()> {
  info!("Clearing bifrost maps");
  let mut interface_map =
    BpfMap::<u32, BifrostInterface>::from_path(INTERFACE_PATH)?;
  let mut vlan_map = BpfMap::<u32, BifrostVlan>::from_path(VLAN_PATH)?;
  info!("Clearing VLANs");
  vlan_map.clear_no_repeat()?;
  info!("Clearing Interfaces");
  interface_map.clear_no_repeat()?;
  Ok(())
}

pub(crate) fn map_interfaces(mappings: &[BridgeInterface]) -> Result<()> {
  info!("Interface maps");
  let mut interface_map =
    BpfMap::<u32, BifrostInterface>::from_path(INTERFACE_PATH)?;
  for mapping in mappings.iter() {
    // Key is the parent interface
    let mut from = interface_name_to_index(&mapping.name)?;
    let redirect_to = interface_name_to_index(&mapping.redirect_to)?;
    let mut mapping = BifrostInterface {
      redirect_to,
      scan_vlans: match mapping.scan_vlans {
        true => 1,
        false => 0,
      },
    };
    interface_map.insert(&mut from, &mut mapping)?;
    info!("Mapped bifrost interface {}->{}", from, redirect_to);
  }
  Ok(())
}

pub(crate) fn map_vlans(mappings: &[BridgeVlan]) -> Result<()> {
  info!("VLAN maps");
  let mut vlan_map = BpfMap::<u32, BifrostVlan>::from_path(VLAN_PATH)?;
  for mapping in mappings.iter() {
    let mut key: u32 =
      (interface_name_to_index(&mapping.parent)? << 16) | mapping.tag;
    let mut val = BifrostVlan { redirect_to: mapping.redirect_to };
    vlan_map.insert(&mut key, &mut val)?;
    info!(
      "Mapped bifrost VLAN: {}:{} => {}",
      mapping.parent, mapping.tag, mapping.redirect_to
    );
    info!("{key}");
  }
  Ok(())
}
