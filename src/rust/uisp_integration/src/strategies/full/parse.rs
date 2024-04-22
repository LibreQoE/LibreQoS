use crate::ip_ranges::IpRanges;
use crate::strategies::full::bandwidth_overrides::BandwidthOverrides;
use crate::uisp_types::{UispDataLink, UispDevice, UispSite};
use lqos_config::Config;
use tracing::info;
use uisp::{DataLink, Device, Site};

pub fn parse_uisp_datasets(
    sites_raw: &[Site],
    data_links_raw: &[DataLink],
    devices_raw: &[Device],
    config: &Config,
    bandwidth_overrides: &BandwidthOverrides,
    ip_ranges: &IpRanges,
) -> (Vec<UispSite>, Vec<UispDataLink>, Vec<UispDevice>) {
    let (mut sites, data_links, devices) = (
        parse_sites(sites_raw, config, bandwidth_overrides),
        parse_data_links(data_links_raw, devices_raw),
        parse_devices(devices_raw, config, ip_ranges),
    );

    // Assign devices to sites
    for site in sites.iter_mut() {
        devices
            .iter()
            .enumerate()
            .filter(|(_, device)| device.site_id == site.id)
            .for_each(|(idx, _)| {
                site.device_indices.push(idx);
            });
    }

    (sites, data_links, devices)
}

fn parse_sites(
    sites_raw: &[Site],
    config: &Config,
    bandwidth_overrides: &BandwidthOverrides,
) -> Vec<UispSite> {
    let mut sites: Vec<UispSite> = sites_raw
        .iter()
        .map(|s| UispSite::from_uisp(s, &config, bandwidth_overrides))
        .collect();
    info!("{} sites have been successfully parsed", sites.len());
    sites
}

fn parse_data_links(data_links_raw: &[DataLink], devices_raw: &[Device]) -> Vec<UispDataLink> {
    let mut data_links: Vec<UispDataLink> = data_links_raw
        .iter()
        .map(|l| UispDataLink::from_uisp(l, &devices_raw))
        .flatten()
        .collect();
    info!(
        "{} data-links have been successfully parsed",
        data_links.len()
    );
    data_links
}

fn parse_devices(devices_raw: &[Device], config: &Config, ip_ranges: &IpRanges) -> Vec<UispDevice> {
    let mut devices: Vec<UispDevice> = devices_raw
        .iter()
        .map(|d| UispDevice::from_uisp(d, config, ip_ranges))
        .collect();
    info!("{} devices have been sucessfully parsed", devices.len());
    devices
}
