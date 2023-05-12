use sysinfo::System;

use super::Task;

pub struct Cpu {
    cores: u16,
    usage: Vec<u32>,
}

impl Cpu {
    async fn get(mut sys: System) -> Self {
        sys.refresh_cpu();
        Cpu {
            cores: sys.cpus().len(),
            usage: sys.cpus().iter().map(|cpu| cpu.cpu_usage() as u32).collect()
        }
    }
}

impl Task for Cpu {
    fn execute(&self) -> TaskResult {
        self.get()
    }

    fn key(&self) -> String {
        String::from("CPU")
    }

    fn cacheable(&self) -> bool { true }
}