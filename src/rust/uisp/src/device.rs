use std::collections::HashSet;

use serde::Deserialize;

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct Device {
    pub identification: DeviceIdentification,
    pub ipAddress: Option<String>,
    pub attributes: Option<DeviceAttributes>,
    pub mode: Option<String>,
    pub interfaces: Option<Vec<DeviceInterface>>,
    pub overview: Option<DeviceOverview>,
}

impl Device {
    pub fn get_name(&self) -> Option<String> {
        if let Some(hostname) = &self.identification.hostname {
            return Some(hostname.clone());
        }
        None
    }

    pub fn get_model(&self) -> Option<String> {
        if let Some(model) = &self.identification.model {
            return Some(model.clone());
        }
        None
    }

    pub fn get_model_name(&self) -> Option<String> {
        if let Some(model) = &self.identification.modelName {
            return Some(model.clone());
        }
        None
    }

    pub fn get_firmware(&self) -> Option<String> {
        if let Some(firmware) = &self.identification.firmwareVersion {
            return Some(firmware.clone());
        }
        None
    }

    pub fn get_id(&self) -> String {
        self.identification.id.clone()
    }

    pub fn get_site_id(&self) -> Option<String> {
        if let Some(site) = &self.identification.site {
            return Some(site.id.clone());
        }
        None
    }

    pub fn get_status(&self) -> Option<String> {
        if let Some(overview) = &self.overview {
            if let Some(status) = &overview.status {
                return Some(status.clone());
            }
        }
        None
    }

    pub fn get_frequency(&self) -> Option<f64> {
        if let Some(overview) = &self.overview {
            if let Some(frequency) = &overview.frequency {
                return Some(*frequency);
            }
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

    pub fn get_noise_floor(&self) -> Option<i32> {
        if let Some(interfaces) = &self.interfaces {
            for intf in interfaces.iter() {
                if let Some(w) = &intf.wireless {
                    if let Some(nf) = &w.noiseFloor {
                        return Some(*nf);
                    }
                }
            }
        }
        None
    }
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct DeviceIdentification {
    pub id: String,
    pub hostname: Option<String>,
    pub mac: Option<String>,
    pub model: Option<String>,
    pub modelName: Option<String>,
    pub role: Option<String>,
    pub site: Option<DeviceSite>,
    pub firmwareVersion: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct DeviceSite {
    pub id: String,
    pub parent: Option<DeviceParent>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct DeviceParent {
    pub id: String,
    pub name: String,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct DeviceAttributes {
    pub ssid: Option<String>,
    pub apDevice: Option<DeviceAccessPoint>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct DeviceAccessPoint {
    pub id: Option<String>,
    pub name: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct DeviceInterface {
    pub identification: Option<InterfaceIdentification>,
    pub addresses: Option<Vec<DeviceAddress>>,
    pub status: Option<InterfaceStatus>,
    pub wireless: Option<DeviceWireless>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct InterfaceIdentification {
    pub name: Option<String>,
    pub mac: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct DeviceAddress {
    pub cidr: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct InterfaceStatus {
    pub status: Option<String>,
    pub speed: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct DeviceOverview {
    pub status: Option<String>,
    pub frequency: Option<f64>,
    pub outageScore: Option<f64>,
    pub stationsCount: Option<i32>,
    pub downlinkCapacity: Option<i64>,
    pub uplinkCapacity: Option<i64>,
    pub channelWidth: Option<i32>,
    pub transmitPower: Option<i32>,
    pub signal: Option<i32>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct DeviceWireless {
    pub noiseFloor: Option<i32>,
}