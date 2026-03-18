use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// A UISP data-link record connecting two endpoints.
#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DataLink {
    /// The UISP identifier for the data link.
    pub id: String,
    /// The source side of the link.
    pub from: DataLinkFrom,
    /// The destination side of the link.
    pub to: DataLinkTo,
    #[serde(rename = "canDelete")]
    /// Whether UISP reports that this link can be deleted.
    pub can_delete: bool,
}

/// The origin endpoint for a UISP data link.
#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DataLinkFrom {
    /// The source device when the link starts at a device.
    pub device: Option<DataLinkDevice>,
    /// The source site when the link starts at a site.
    pub site: Option<DataLinkSite>,
}

/// A device reference embedded in a UISP data-link response.
#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DataLinkDevice {
    /// The identifying fields UISP includes for the device.
    pub identification: DataLinkDeviceIdentification,
}

/// Shared identifier payload used for devices and sites in data-link records.
#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DataLinkDeviceIdentification {
    /// The UISP identifier of the referenced object.
    pub id: String,
    /// The human-readable name of the referenced object.
    pub name: String,
}

/// The destination endpoint for a UISP data link.
#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DataLinkTo {
    /// The destination device when the link ends at a device.
    pub device: Option<DataLinkDevice>,
    /// The destination site when the link ends at a site.
    pub site: Option<DataLinkSite>,
}

/// A site reference embedded in a UISP data-link response.
#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DataLinkSite {
    /// The identifying fields UISP includes for the site.
    pub identification: DataLinkDeviceIdentification,
}
