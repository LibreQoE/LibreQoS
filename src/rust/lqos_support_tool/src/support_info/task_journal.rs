use std::process::Command;
use crate::console::success;
use crate::support_info::SupportInfo;

#[derive(Default)]
pub struct TaskJournal {
    target: String,
    output: String,
}

impl SupportInfo for TaskJournal {
    fn get_string(&self) -> String {
        self.output.to_string()
    }

    fn get_name(&self) -> String {
        format!("Journal ({})", self.target)
    }

    fn get_filename(&self) -> Option<String> {
        None
    }

    fn gather(&mut self) -> anyhow::Result<()> {
        let out = Command::new("/bin/journalctl")
            .args(&["--no-pager", "-u", &self.target])
            .output()?;

        self.output = String::from_utf8_lossy(&out.stdout).to_string();
        success(&format!("Gathered journalctl status for {}", self.target));

        Ok(())
    }
}

impl TaskJournal {
    pub fn boxed<S: ToString>(target: S) -> Box<Self> {
        Box::new(Self {
            target: target.to_string(),
            ..Default::default()
        })
    }
}