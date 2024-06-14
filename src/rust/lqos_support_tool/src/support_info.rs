use colored::Colorize;
use serde::{Deserialize, Serialize};
use crate::console::error;
use crate::sanity_checks::{run_sanity_checks, SanityChecks};

mod ip_link;
mod systemctl_services;
mod systemctl_service_single;
mod lqos_config;
mod task_journal;
mod service_config;
mod ip_addr;
mod kernel_info;
mod distro_name;

pub trait SupportInfo {
    fn get_string(&self) -> String;
    fn get_name(&self) -> String;
    fn get_filename(&self) -> Option<String>;
    fn gather(&mut self) -> anyhow::Result<()>;
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SupportDump {
    pub sender: String,
    pub comment: String,
    pub lts_key: String,
    pub sanity_checks: SanityChecks,
    pub entries: Vec<DumpEntry>
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct DumpEntry {
    pub name: String,
    pub filename: Option<String>,
    pub contents: String,
}

impl SupportDump {
    pub fn serialize_and_compress(&self) -> anyhow::Result<Vec<u8>> {
        let cbor_bytes = serde_cbor::to_vec(self)?;
        let compressed_bytes = miniz_oxide::deflate::compress_to_vec(&cbor_bytes, 10);
        Ok(compressed_bytes)
    }

    pub fn from_bytes(raw_bytes: &[u8]) -> anyhow::Result<Self> {
        let decompressed_bytes = miniz_oxide::inflate::decompress_to_vec(raw_bytes).unwrap();
        let deserialized = serde_cbor::from_slice(&decompressed_bytes)?;
        Ok(deserialized)
    }
}

pub fn gather_all_support_info(sender: &str, comments: &str, lts_key: &str) -> anyhow::Result<SupportDump> {
    let sanity_checks = run_sanity_checks()?;

    let mut data_targets: Vec<Box<dyn SupportInfo>> = vec![
        lqos_config::LqosConfig::boxed(),
        ip_link::IpLink::boxed(),
        ip_addr::IpAddr::boxed(),
        kernel_info::KernelInfo::boxed(),
        distro_name::DistroName::boxed(),
        systemctl_services::SystemCtlServices::boxed(),
        systemctl_service_single::SystemCtlService::boxed("lqosd"),
        systemctl_service_single::SystemCtlService::boxed("lqos_node_manager"),
        systemctl_service_single::SystemCtlService::boxed("lqos_scheduler"),
        task_journal::TaskJournal::boxed("lqosd"),
        task_journal::TaskJournal::boxed("lqos_node_manager"),
        task_journal::TaskJournal::boxed("lqos_scheduler"),
        service_config::ServiceConfig::boxed("ShapedDevices.csv"),
        service_config::ServiceConfig::boxed("network.json"),
    ];

    for target in data_targets.iter_mut() {
        println!("{} : {}",
                 "TASK-GATHER".cyan(),
                 target.get_name().yellow()
        );
        if let Err(e) = target.gather() {
            error(&e.to_string());
        }
    }

    let mut dump = SupportDump {
        sender: sender.to_string(),
        comment: comments.to_string(),
        lts_key: lts_key.to_string(),
        sanity_checks,
        entries: Vec::new(),
    };

    for target in data_targets.iter() {
        let entry = DumpEntry {
            name: target.get_name(),
            filename: target.get_filename(),
            contents: target.get_string(),
        };
        dump.entries.push(entry);
    }
    //println!("{dump:#?}");

    Ok(dump)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let original = SupportDump {
            entries: vec![
                DumpEntry {
                    name: "Test".to_string(),
                    filename: None,
                    contents: "BLAH".to_string(),                    
                }
            ],
            ..Default::default()
        };
        let bytes = original.serialize_and_compress().unwrap();
        let restored = SupportDump::from_bytes(&bytes).unwrap();
        assert_eq!(original.entries.len(), restored.entries.len());
        assert_eq!(original.entries.len(), 1);
        assert_eq!(original.entries[0].name, "Test");
        assert!(original.entries[0].filename.is_none());
        assert_eq!(original.entries[0].contents, "BLAH");
    }
}