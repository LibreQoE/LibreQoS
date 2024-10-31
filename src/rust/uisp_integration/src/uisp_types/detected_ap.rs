/// Detected Access Point
#[derive(Debug)]
pub struct DetectedAccessPoint {
    pub site_id: String,
    pub device_id: String,
    pub device_name: String,
    pub child_sites: Vec<String>,
}
