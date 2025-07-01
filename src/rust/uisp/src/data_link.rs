use allocative::Allocative;
use serde::{Deserialize, Serialize};

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DataLink {
    pub id: String,
    pub from: DataLinkFrom,
    pub to: DataLinkTo,
    #[serde(rename = "canDelete")]
    pub can_delete: bool,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DataLinkFrom {
    pub device: Option<DataLinkDevice>,
    pub site: Option<DataLinkSite>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DataLinkDevice {
    pub identification: DataLinkDeviceIdentification,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DataLinkDeviceIdentification {
    pub id: String,
    pub name: String,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DataLinkTo {
    pub device: Option<DataLinkDevice>,
    pub site: Option<DataLinkSite>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug, Allocative)]
pub struct DataLinkSite {
    pub identification: DataLinkDeviceIdentification,
}
