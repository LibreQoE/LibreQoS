use sysinfo::{System, SystemExt};

use super::Task;

pub struct Ram {
    used: u64,
    total: u64,
}

impl Ram {
    async fn get(mut sys: System) -> Self {
        sys.refresh_memory();
        Ram {
            total: sys.total_memory(),
            used: sys.used_memory(),
        }
    }
}

impl Task for Ram {
    fn execute(&self) -> TaskResult {
        self.get()
    }

    fn key(&self) -> String {
        String::from("RAM")
    }

    fn cacheable(&self) -> bool { true }
}