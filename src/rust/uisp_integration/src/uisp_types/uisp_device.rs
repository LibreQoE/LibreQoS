use uisp::Device;

/// Trimmed UISP device for easy use
pub struct UispDevice {
    pub id: String,
    pub mac: String,
    pub site_id: String,
}

impl UispDevice {
    pub fn from_uisp(device: &Device) -> Self {
        let mac = if let Some(id) = &device.identification.mac {
            id.clone()
        } else {
            "".to_string()
        };

        Self {
            id: device.get_id(),
            mac,
            site_id: device.get_site_id().unwrap_or("".to_string())
        }
    }
}
