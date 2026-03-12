use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// A UISP site record with identification, location, QoS, and CRM metadata.
#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct Site {
    /// The UISP identifier for the site.
    pub id: String,
    /// Identification and status information reported for the site.
    pub identification: Option<SiteId>,
    /// Address, coordinates, and endpoint details for the site.
    pub description: Option<Description>,
    /// Traffic-shaping settings attached to the site.
    pub qos: Option<Qos>,
    /// CRM client and service associations, when available.
    pub ucrm: Option<Ucrm>,
}

impl Site {
    /// Returns the site name from the identification block.
    pub fn name(&self) -> Option<String> {
        if let Some(id) = &self.identification
            && let Some(name) = &id.name
        {
            return Some(name.clone());
        }
        None
    }

    /// Returns the site name or an empty string when UISP did not provide one.
    pub fn name_or_blank(&self) -> String {
        if let Some(name) = self.name() {
            name
        } else {
            "".to_string()
        }
    }

    /// Returns the postal address string from the description block.
    pub fn address(&self) -> Option<String> {
        if let Some(desc) = &self.description
            && let Some(address) = &desc.address
        {
            return Some(address.to_string());
        }
        None
    }

    /// Returns `true` when UISP classifies this site as a tower or parent site.
    pub fn is_tower(&self) -> bool {
        if let Some(id) = &self.identification
            && let Some(site_type) = &id.site_type
            && site_type == "site"
        {
            return true;
        }
        false
    }

    /// Returns `true` when UISP classifies this site as a client endpoint.
    pub fn is_client_site(&self) -> bool {
        if let Some(id) = &self.identification
            && let Some(site_type) = &id.site_type
            && site_type == "endpoint"
        {
            return true;
        }
        false
    }

    /// Returns `true` when the site reports the supplied parent site identifier.
    pub fn is_child_of(&self, parent_id: &str) -> bool {
        if let Some(id) = &self.identification
            && let Some(parent) = &id.parent
            && let Some(pid) = &parent.id
            && pid == parent_id
        {
            return true;
        }
        false
    }

    /// Returns site download and upload rates in Mbps, falling back to supplied defaults.
    pub fn qos(&self, default_download_mbps: u64, default_upload_mbps: u64) -> (u64, u64) {
        let mut down = default_download_mbps;
        let mut up = default_upload_mbps;
        if let Some(qos) = &self.qos {
            if let Some(d) = &qos.downloadSpeed {
                down = *d / 1_000_000;
            }
            if let Some(u) = &qos.uploadSpeed {
                up = *u / 1_000_000;
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

    /// Returns whether UISP marks the site as suspended.
    pub fn is_suspended(&self) -> bool {
        if let Some(site_id) = &self.identification {
            site_id.suspended
        } else {
            false
        }
    }
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct SiteParent {
    pub id: Option<String>,
}

/// UISP identification and status metadata for a site.
#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct SiteId {
    /// The human-readable site name.
    pub name: Option<String>,
    #[serde(rename = "type")]
    /// The UISP site type string such as `site` or `endpoint`.
    pub site_type: Option<String>,
    /// The parent site reference when this site is nested under another.
    pub parent: Option<SiteParent>,
    /// The UISP status string for the site.
    pub status: Option<String>,
    /// Whether UISP reports the site as suspended.
    pub suspended: bool,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Clone, Allocative)]
pub struct Endpoint {
    pub id: Option<String>,
    pub name: Option<String>,
    pub parentId: Option<String>,
}

/// Descriptive and geographic metadata for a UISP site.
#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct Description {
    /// The postal or street address for the site.
    pub address: Option<String>,
    /// Geographic coordinates for the site.
    pub location: Option<Location>,
    /// The site height reported by UISP.
    pub height: Option<f64>,
    /// Endpoint references associated with the site.
    pub endpoints: Option<Vec<Endpoint>>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct Location {
    pub longitude: f64,
    pub latitude: f64,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct Qos {
    pub enabled: bool,
    pub downloadSpeed: Option<u64>,
    pub uploadSpeed: Option<u64>,
    // Optional burst sizes reported by UISP in kilobytes per second (kB/s).
    // Example: 12500 kB/s represents 100 Mbps burst.
    pub downloadBurstSize: Option<u64>,
    pub uploadBurstSize: Option<u64>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct Ucrm {
    pub client: Option<UcrmClient>,
    pub service: Option<UcrmService>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct UcrmClient {
    pub id: String,
    pub name: String,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct UcrmService {
    pub id: String,
    pub name: String,
    pub status: i32,
    pub tariffId: String,
    pub trafficShapingOverrideEnabled: bool,
}
