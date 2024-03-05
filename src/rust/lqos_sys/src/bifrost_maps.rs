use crate::{bpf_map::BpfMap, lqos_kernel::interface_name_to_index};
use anyhow::Result;
use log::info;

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

pub(crate) fn map_multi_interface_mode(
  to_internet: &str,
  to_lan: &str,  
) -> Result<()> {
  info!("Interface maps (multi-interface)");
  let mut interface_map =
    BpfMap::<u32, BifrostInterface>::from_path(INTERFACE_PATH)?;

  // Internet
  let mut from = interface_name_to_index(to_internet)?;
  let redirect_to = interface_name_to_index(to_lan)?;
  let mut mapping = BifrostInterface {
    redirect_to,
    scan_vlans: 0,
  };
  interface_map.insert(&mut from, &mut mapping)?;
  info!("Mapped bifrost interface {}->{}", from, redirect_to);

  // LAN
  let mut from = interface_name_to_index(to_lan)?;
  let redirect_to = interface_name_to_index(to_internet)?;
  let mut mapping = BifrostInterface {
    redirect_to,
    scan_vlans: 0,
  };
  interface_map.insert(&mut from, &mut mapping)?;
  info!("Mapped bifrost interface {}->{}", from, redirect_to);

  Ok(())
}

pub(crate) fn map_single_interface_mode(
  interface: &str,
  internet_vlan: u32,
  lan_vlan: u32,
) -> Result<()> {
  info!("Interface maps (single interface)");
  let mut interface_map =
    BpfMap::<u32, BifrostInterface>::from_path(INTERFACE_PATH)?;

  let mut vlan_map = BpfMap::<u32, BifrostVlan>::from_path(VLAN_PATH)?;

  // Internet
  let mut from = interface_name_to_index(interface)?;
  let redirect_to = interface_name_to_index(interface)?;
  let mut mapping = BifrostInterface {
    redirect_to,
    scan_vlans: 1,
  };
  interface_map.insert(&mut from, &mut mapping)?;
  info!("Mapped bifrost interface {}->{}", from, redirect_to);

  // VLANs - Internet
  let mut key: u32 = (interface_name_to_index(&interface)? << 16) | internet_vlan;
  let mut val = BifrostVlan { redirect_to: lan_vlan };
  vlan_map.insert(&mut key, &mut val)?;
  info!(
    "Mapped bifrost VLAN: {}:{} => {}",
    interface, internet_vlan, lan_vlan
  );
  info!("{key}");

  // VLANs - LAN
  let mut key: u32 = (interface_name_to_index(&interface)? << 16) | lan_vlan;
  let mut val = BifrostVlan { redirect_to: internet_vlan };
  vlan_map.insert(&mut key, &mut val)?;
  info!(
    "Mapped bifrost VLAN: {}:{} => {}",
    interface, lan_vlan, internet_vlan
  );
  info!("{key}");

  Ok(())
}

/*pub(crate) fn map_interfaces(mappings: &[&str]) -> Result<()> {
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
}*/
