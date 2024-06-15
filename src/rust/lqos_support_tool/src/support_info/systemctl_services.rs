use std::process::Command;
use crate::console::success;
use crate::support_info::SupportInfo;

#[derive(Default)]
pub struct SystemCtlServices {
    output: String,
}

impl SupportInfo for SystemCtlServices {
    fn get_string(&self) -> String {
        self.output.to_string()
    }

    fn get_name(&self) -> String {
        "SystemCtl Status".to_string()
    }

    fn get_filename(&self) -> Option<String> {
        None
    }

    fn gather(&mut self) -> anyhow::Result<()> {
        let out = Command::new("/bin/systemctl")
            .args(&["--no-pager", "status"])
            .output()?;

        self.output = String::from_utf8_lossy(&out.stdout).to_string();
        success("Gathered global `systemctl status`");

        Ok(())
    }
}

impl SystemCtlServices {
    pub fn boxed() -> Box<Self> {
        Box::new(Self::default())
    }
}