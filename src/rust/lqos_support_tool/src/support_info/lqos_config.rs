use std::path::Path;
use crate::console::success;
use crate::support_info::SupportInfo;

#[derive(Default)]
pub struct LqosConfig {
    output: String,
}

impl SupportInfo for LqosConfig {
    fn get_string(&self) -> String {
        self.output.to_string()
    }

    fn get_name(&self) -> String {
        "LibreQoS Config File".to_string()
    }

    fn get_filename(&self) -> Option<String> {
        Some("/etc/lqos.conf".to_string())
    }

    fn gather(&mut self) -> anyhow::Result<()> {
        let path = Path::new("/etc/lqos.conf");
        if !path.exists() {
            anyhow::bail!("/etc/lqos.conf could not be opened");
        }
        self.output = std::fs::read_to_string(path)?;
        success("Gathered /etc/lqos.conf");
        Ok(())
    }
}

impl LqosConfig {
    pub fn boxed() -> Box<Self> {
        Box::new(Self::default())
    }
}