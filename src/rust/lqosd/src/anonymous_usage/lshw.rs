use std::process::Command;

use lqos_bus::anonymous::NicV1;

#[derive(Default)]
pub(crate) struct Nic {
    pub(crate) description: String,
    pub(crate) product: String,
    pub(crate) vendor: String,
    pub(crate) clock: String,
    pub(crate) capacity: String,
}

#[allow(clippy::from_over_into)]
impl Into<NicV1> for Nic {
    fn into(self) -> NicV1 {
        NicV1 {
            description: self.description,
            product: self.product,
            vendor: self.vendor,
            clock: self.clock,
            capacity: self.capacity,
        }
    }
}

pub(crate) fn get_nic_info() -> anyhow::Result<Vec<Nic>> {
    let mut current_nic = None;
    let mut result = Vec::new();

    let output = Command::new("/bin/lshw")
        .args(["-C", "network"])
        .output()?;
    let stdout = String::from_utf8(output.stdout)?;
    let lines = stdout.split('\n');
    for line in lines {
        let trimmed = line.trim();

        // Starting a new record
        if trimmed.starts_with("*-network:") {
            if let Some(nic) = current_nic {
                result.push(nic);
            }
            current_nic = Some(Nic::default());
        }

        if let Some(nic) = current_nic.as_mut() {
            if let Some(d) = trimmed.strip_prefix("description: ") {
                nic.description = d.to_string();
            }
            if let Some(d) = trimmed.strip_prefix("product: ") {
                nic.product = d.to_string();
            }
            if let Some(d) = trimmed.strip_prefix("vendor: ") {
                nic.vendor = d.to_string();
            }
            if let Some(d) = trimmed.strip_prefix("clock: ") {
                nic.clock = d.to_string();
            }
            if let Some(d) = trimmed.strip_prefix("capacity: ") {
                nic.capacity = d.to_string();
            }
        }
    }

    if let Some(nic) = current_nic {
        result.push(nic);
    }
    Ok(result)
}
