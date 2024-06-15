use std::path::Path;
use lqos_config::load_config;
use crate::console::{error, success};
use crate::support_info::SupportInfo;

#[derive(Default)]
pub struct ServiceConfig {
    target: String,
    output: String,
}

impl SupportInfo for ServiceConfig {
    fn get_string(&self) -> String {
        self.output.to_string()
    }

    fn get_name(&self) -> String {
        format!("Config File: {}", self.target)
    }

    fn get_filename(&self) -> Option<String> {
        let cfg = load_config();
        if let Ok(cfg) = cfg {
            Some(format!("{}{}", cfg.lqos_directory, self.target))
        } else {
            error("Unable to read configuration!");
            None
        }
    }

    fn gather(&mut self) -> anyhow::Result<()> {
        let cfg = load_config()?;
        let path = Path::new(&cfg.lqos_directory).join(&self.target);
        if !path.exists() {
            anyhow::bail!("Could not read from {:?}", path);
        }
        self.output = std::fs::read_to_string(path)?;
        success(&format!("Gathered {}", self.target));
        Ok(())
    }
}

impl ServiceConfig {
    pub fn boxed<S: ToString>(target: S) -> Box<Self> {
        Box::new(Self {
            target: target.to_string(),
            ..Default::default()
        })
    }
}