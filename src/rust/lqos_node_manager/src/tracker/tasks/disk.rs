use sysinfo::{System, SystemExt};

use super::Task;

pub struct Disk {
    used: u64,
    total: u64,
}

impl Disk {
    async fn get(mut sys: System) -> Vec<Self> {
        sys.refresh_disks();
        let disks = Vec::new();
        for disk in sys.disks() {
            disks.push(Disk {
                available: disk.available_space(),
                total: disk.total_space(),
            });
        }
        disks
    }
}

impl Task for Disk {
    fn execute(&self) -> TaskResult {
        self.get()
    }

    fn key(&self) -> String {
        String::from("DISK")
    }

    fn cacheable(&self) -> bool { false }
}