use std::process::Command;
use crate::console::success;
use crate::support_info::SupportInfo;

#[derive(Debug, Default)]
pub struct KernelInfo {
    output: String,
}

impl SupportInfo for KernelInfo {
    fn get_string(&self) -> String {
        self.output.to_string()
    }

    fn get_name(&self) -> String {
        "Uname Kernel Info".to_string()
    }

    fn get_filename(&self) -> Option<String> {
        None
    }

    fn gather(&mut self) -> anyhow::Result<()> {
        let output = Command::new("/bin/uname")
            .arg("-a")
            .output()?;
        let out_str = String::from_utf8_lossy(output.stdout.as_slice());
        self.output = out_str.to_string();
        success("Gathered kernel info");
        Ok(())
    }
}

impl KernelInfo {
    pub fn boxed() -> Box<Self> {
        Box::new(Self::default())
    }
}