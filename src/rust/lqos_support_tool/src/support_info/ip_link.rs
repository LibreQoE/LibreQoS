use std::process::Command;
use crate::console::success;
use crate::support_info::SupportInfo;

#[derive(Debug, Default)]
pub struct IpLink {
    output: String,
}

impl SupportInfo for IpLink {
    fn get_string(&self) -> String {
        self.output.to_string()
    }

    fn get_name(&self) -> String {
        "IP Link Information".to_string()
    }

    fn get_filename(&self) -> Option<String> {
        None
    }

    fn gather(&mut self) -> anyhow::Result<()> {
        let output = Command::new("/sbin/ip")
            .arg("link")
            .output()?;
        let out_str = String::from_utf8_lossy(output.stdout.as_slice());
        self.output = out_str.to_string();
        success("Gathered `ip link` data");
        Ok(())
    }
}

impl IpLink {
    pub fn boxed() -> Box<Self> {
        Box::new(Self::default())
    }
}