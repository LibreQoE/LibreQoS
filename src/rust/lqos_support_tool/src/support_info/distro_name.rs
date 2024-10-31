use std::process::Command;
use crate::console::success;
use crate::support_info::SupportInfo;

#[derive(Debug, Default)]
pub struct DistroName {
    output: String,
}

impl SupportInfo for DistroName {
    fn get_string(&self) -> String {
        self.output.to_string()
    }

    fn get_name(&self) -> String {
        "LSB Distro Info".to_string()
    }

    fn get_filename(&self) -> Option<String> {
        None
    }

    fn gather(&mut self) -> anyhow::Result<()> {
        let output = Command::new("/bin/lsb_release")
            .arg("-a")
            .output()?;
        let out_str = String::from_utf8_lossy(output.stdout.as_slice());
        self.output = out_str.to_string();
        success("Gathered distro info");
        Ok(())
    }
}

impl DistroName {
    pub fn boxed() -> Box<Self> {
        Box::new(Self::default())
    }
}