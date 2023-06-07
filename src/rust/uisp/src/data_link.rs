use serde::Deserialize;

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct DataLink {
    pub id: String,
    pub from: DataLinkFrom,
    pub to: DataLinkTo,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct DataLinkFrom {
    pub device: Option<DataLinkDevice>,
    pub site: Option<DataLinkSite>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct DataLinkDevice {
    pub identification: DataLinkDeviceIdentification,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct DataLinkDeviceIdentification {
    pub id: String,
    pub name: String,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct DataLinkTo {
    pub device: Option<DataLinkDevice>,
    pub site: Option<DataLinkSite>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct DataLinkSite {
    pub identification: DataLinkDeviceIdentification,
}