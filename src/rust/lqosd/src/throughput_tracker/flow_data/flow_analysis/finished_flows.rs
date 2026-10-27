use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AsnListEntry {
    pub count: usize,
    pub asn: u32,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct AsnCountryListEntry {
    pub count: usize,
    pub name: String,
    pub iso_code: String,
}

#[derive(Debug, Serialize)]
pub struct AsnProtocolListEntry {
    pub count: usize,
    pub protocol: String,
}
