use allocative::Allocative;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// A UISP device record, optionally including interface and radio overview data.
#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct Device {
    /// Device identity fields returned by UISP.
    pub identification: DeviceIdentification,
    /// The primary management IP address, typically in CIDR notation.
    pub ipAddress: Option<String>,
    /// Additional UISP attributes such as SSID and linked access point.
    pub attributes: Option<DeviceAttributes>,
    /// The UISP mode string for the device.
    pub mode: Option<String>,
    /// Interface definitions when `withInterfaces=true` is requested.
    pub interfaces: Option<Vec<DeviceInterface>>,
    /// Summary radio, capacity, and health data reported by UISP.
    pub overview: Option<DeviceOverview>,
}

impl Device {
    /// Returns the device hostname when UISP provides one.
    pub fn get_name(&self) -> Option<String> {
        if let Some(hostname) = &self.identification.hostname {
            return Some(hostname.clone());
        }
        None
    }

    /// Returns the device model identifier when available.
    pub fn get_model(&self) -> Option<String> {
        if let Some(model) = &self.identification.model {
            return Some(model.clone());
        }
        None
    }

    /// Returns the human-readable model name when available.
    pub fn get_model_name(&self) -> Option<String> {
        if let Some(model) = &self.identification.modelName {
            return Some(model.clone());
        }
        None
    }

    /// Returns the firmware version reported by UISP.
    pub fn get_firmware(&self) -> Option<String> {
        if let Some(firmware) = &self.identification.firmwareVersion {
            return Some(firmware.clone());
        }
        None
    }

    /// Returns the UISP device identifier.
    pub fn get_id(&self) -> String {
        self.identification.id.clone()
    }

    /// Returns the containing site identifier when the device is assigned to a site.
    pub fn get_site_id(&self) -> Option<String> {
        if let Some(site) = &self.identification.site {
            return Some(site.id.clone());
        }
        None
    }

    /// Returns the current UISP status string from the overview block.
    pub fn get_status(&self) -> Option<String> {
        if let Some(overview) = &self.overview
            && let Some(status) = &overview.status
        {
            return Some(status.clone());
        }
        None
    }

    /// Returns the operating frequency from the overview block.
    pub fn get_frequency(&self) -> Option<f64> {
        if let Some(overview) = &self.overview
            && let Some(frequency) = &overview.frequency
        {
            return Some(*frequency);
        }
        None
    }

    fn strip_ip(ip: &str) -> String {
        if !ip.contains('/') {
            ip.to_string()
        } else {
            ip[0..ip.find('/').unwrap()].to_string()
        }
    }

    /// Collects the device management and interface IP addresses without CIDR suffixes.
    pub fn get_addresses(&self) -> HashSet<String> {
        let mut result = HashSet::new();
        if let Some(ip) = &self.ipAddress {
            result.insert(Device::strip_ip(ip));
        }
        if let Some(interfaces) = &self.interfaces {
            for interface in interfaces {
                if let Some(addresses) = &interface.addresses {
                    for addy in addresses {
                        if let Some(cidr) = &addy.cidr {
                            result.insert(Device::strip_ip(cidr));
                        }
                    }
                }
            }
        }
        result
    }

    /// Returns the first wireless noise-floor value found on the device interfaces.
    pub fn get_noise_floor(&self) -> Option<i64> {
        if let Some(interfaces) = &self.interfaces {
            for intf in interfaces.iter() {
                if let Some(w) = &intf.wireless
                    && let Some(nf) = &w.noiseFloor
                {
                    return Some(*nf);
                }
            }
        }
        None
    }
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DeviceIdentification {
    pub id: String,
    pub hostname: Option<String>,
    pub mac: Option<String>,
    pub model: Option<String>,
    pub modelName: Option<String>,
    pub r#type: Option<String>,
    pub role: Option<String>,
    pub site: Option<DeviceSite>,
    pub firmwareVersion: Option<String>,
    pub vendor: Option<String>,
    pub vendorName: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DeviceSite {
    pub id: String,
    pub parent: Option<DeviceParent>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DeviceParent {
    pub id: String,
    pub name: String,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DeviceAttributes {
    pub ssid: Option<String>,
    pub apDevice: Option<DeviceAccessPoint>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DeviceAccessPoint {
    pub id: Option<String>,
    pub name: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DeviceInterface {
    pub identification: Option<InterfaceIdentification>,
    pub addresses: Option<Vec<DeviceAddress>>,
    pub status: Option<InterfaceStatus>,
    pub wireless: Option<DeviceWireless>,
    pub stations: Option<Vec<DeviceStation>>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct InterfaceIdentification {
    pub r#type: Option<String>,
    pub name: Option<String>,
    pub mac: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DeviceAddress {
    pub cidr: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct InterfaceStatus {
    pub status: Option<String>,
    pub speed: Option<String>,
    pub currentSpeed: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DeviceOverview {
    pub status: Option<String>,
    pub frequency: Option<f64>,
    pub outageScore: Option<f64>,
    pub stationsCount: Option<i64>,
    pub totalCapacity: Option<i64>,
    pub downlinkCapacity: Option<i64>,
    pub uplinkCapacity: Option<i64>,
    pub theoreticalTotalCapacity: Option<i64>,
    pub theoreticalDownlinkCapacity: Option<i64>,
    pub theoreticalUplinkCapacity: Option<i64>,
    pub channelWidth: Option<i64>,
    pub transmitPower: Option<i64>,
    pub signal: Option<i64>,

    pub cpu: Option<i64>,
    pub createdAt: Option<String>,
    pub distance: Option<i64>,
    pub downlinkUtilization: Option<f64>,
    pub uplinkUtilization: Option<f64>,
    pub ram: Option<i64>,
    pub temperature: Option<i64>,
    pub uptime: Option<i64>,
    pub wirelessMode: Option<String>,
    pub wirelessActiveInterfaceIds: Option<Vec<String>>,
    pub linkScore: Option<DeviceLinkScore>,
    pub antenna: Option<DeviceAntenna>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DeviceLinkScore {
    pub score: Option<f64>,
    pub scoreMax: Option<f64>,
    pub airTimeScore: Option<f64>,
    pub linkScore: Option<f64>,
    pub linkScoreHint: String,
    pub theoreticalDownlinkCapacity: Option<i64>,
    pub theoreticalUplinkCapacity: Option<i64>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DeviceAntenna {
    pub id: Option<String>,
    pub gain: Option<i64>,
    pub name: Option<String>,
    pub builtIn: Option<bool>,
    pub cableLoss: Option<i64>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DeviceWireless {
    pub noiseFloor: Option<i64>,
    pub dlRatio: Option<f64>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DeviceStation {
    connected: Option<bool>,
    connectedTime: Option<i64>,
    device_identification: Option<DeviceIdentification>,
    latency: Option<i64>,
    mac: Option<String>,
    name: Option<String>,
    model: Option<String>,
    rxModulation: Option<String>,
    rxSignal: Option<i64>,
    txSignal: Option<i64>,
    downlinkAirTime: Option<f64>,
    uplinkAirTime: Option<f64>,
}
