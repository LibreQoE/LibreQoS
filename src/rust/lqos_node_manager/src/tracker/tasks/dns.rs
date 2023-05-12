use std::net::IpAddr;
use dns_lookup::lookup_addr;

use super::Task;

pub struct Dns {
    used: u64,
    total: u64,
}

impl Dns {
    async fn get(ip: IpAddr) -> Vec<Self> {
        lookup_addr(&ip).unwrap_or(ip.to_string())
    }
}

impl Task for Dns {
    fn execute(&self) -> TaskResult {
        self.get()
    }

    fn key(&self) -> String {
        String::from("DISK")
    }

    fn cacheable(&self) -> bool { false }
}