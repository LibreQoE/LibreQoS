use lqos_config::Config;
use uisp::Device;

/// Trimmed UISP device for easy use
pub struct UispDevice {
    pub id: String,
    pub mac: String,
    pub site_id: String,
    pub download: u32,
    pub upload: u32,
}

impl UispDevice {
    pub fn from_uisp(device: &Device, config: &Config) -> Self {
        let mac = if let Some(id) = &device.identification.mac {
            id.clone()
        } else {
            "".to_string()
        };

        let mut download = config.queues.generated_pn_download_mbps;
        let mut upload = config.queues.generated_pn_upload_mbps;
        if let Some(overview) = &device.overview {
            println!("{:?}", overview);
            if let Some(dl) = overview.downlinkCapacity {
                download = dl as u32 / 1000000;
            }
            if let Some(ul) = overview.uplinkCapacity {
                upload = ul as u32 / 1000000;
            }
        }

        Self {
            id: device.get_id(),
            mac,
            site_id: device.get_site_id().unwrap_or("".to_string()),
            upload,
            download,
        }
    }
}
