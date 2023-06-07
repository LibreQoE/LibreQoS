use serde::{Deserialize, Serialize};

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct Site {
    pub id: String,
    pub identification: Option<SiteId>,
    pub description: Option<Description>,
    pub qos: Option<Qos>,
    pub ucrm: Option<Ucrm>,
}

impl Site {
    pub fn name(&self) -> Option<String> {
        if let Some(id) = &self.identification {
            if let Some(name) = &id.name {
                return Some(name.clone());
            }
        }
        None
    }

    pub fn address(&self) -> Option<String> {
        if let Some(desc) = &self.description {
            if let Some(address) = &desc.address {
                return Some(address.to_string());
            }
        }
        None
    }

    pub fn is_tower(&self) -> bool {
        if let Some(id) = &self.identification {
            if let Some(site_type) = &id.site_type {
                if site_type == "site" {
                    return true;
                }
            }
        }
        false
    }

    pub fn is_client_site(&self) -> bool {
        if let Some(id) = &self.identification {
            if let Some(site_type) = &id.site_type {
                if site_type == "endpoint" {
                    return true;
                }
            }
        }
        false
    }

    pub fn is_child_of(&self, parent_id: &str) -> bool {
        if let Some(id) = &self.identification {
            if let Some(parent) = &id.parent {
                if let Some(pid) = &parent.id {
                    if pid == parent_id {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn qos(&self, default_download_mbps: u32, default_upload_mbps: u32) -> (u32, u32) {
        let mut down = default_download_mbps;
        let mut up = default_upload_mbps;
        if let Some(qos) = &self.qos {
            if let Some(d) = &qos.downloadSpeed {
                down = *d as u32 / 1_000_000;
            }
            if let Some(u) = &qos.uploadSpeed {
                up = *u as u32 / 1_000_000;
            }
        }
        if down == 0 {
            down = default_download_mbps;
        }
        if up == 0 {
            up = default_upload_mbps;
        }
        (down, up)
    }
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct SiteParent {
    pub id: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct SiteId {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub site_type: Option<String>,
    pub parent: Option<SiteParent>,
    pub status: Option<String>,
    pub suspended: bool,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Endpoint {
    pub id: Option<String>,
    pub name: Option<String>,
    pub parentId: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct Description {
    pub address: Option<String>,
    pub location: Option<Location>,
    pub height: Option<f64>,
    pub endpoints: Option<Vec<Endpoint>>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct Location {
    pub longitude: f64,
    pub latitude: f64,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct Qos {
    pub enabled: bool,
    pub downloadSpeed: Option<usize>,
    pub uploadSpeed: Option<usize>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct Ucrm {
    pub client: Option<UcrmClient>,
    pub service: Option<UcrmService>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct UcrmClient {
    pub id: String,
    pub name: String,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct UcrmService {
    pub id: String,
    pub name: String,
    pub status: i32,
    pub tariffId: String,
    pub trafficShapingOverrideEnabled: bool,
}