//! Dynamic circuit persistence for the runtime overlay.
//!
//! The dynamic-circuits store is an overlay on top of
//! `lqos_config::ConfigShapedDevices` and never mutates ShapedDevices.csv.
//! Static `ShapedDevices.csv` data remains authoritative; dynamic state exists
//! only to persist runtime overlay entries in `dynamic_status.json`.
//!
//! This module is intentionally small until the runtime overlay loader lands.

use allocative::Allocative;
use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}};
use thiserror::Error;

/// File name used to persist dynamic circuit overlay state.
pub const DYNAMIC_STATUS_FILENAME: &str = "dynamic_status.json";
/// Current supported on-disk schema version for `dynamic_status.json`.
pub const DYNAMIC_STATUS_VERSION: u32 = 1;

/// Errors that can occur while resolving or loading `dynamic_status.json`.
#[derive(Debug, Error)]
pub enum DynamicStatusError {
    /// The main LibreQoS config could not be loaded.
    #[error("Unable to load LibreQoS config while resolving dynamic_status.json path")]
    ConfigLoadError,
    /// A `dynamic_status.json` file could not be read from disk.
    #[error("Unable to read dynamic status file at {path}: {source}")]
    CannotRead {
        /// Full path to the file that failed.
        path: String,
        /// Source IO error.
        source: std::io::Error,
    },
    /// A `dynamic_status.json` file existed but could not be parsed.
    #[error("Unable to parse dynamic status file at {path}: {details}")]
    ParseError {
        /// Full path to the file that failed.
        path: String,
        /// Parse error details.
        details: String,
    },
    /// A `dynamic_status.json` file could not be written to disk.
    #[error("Unable to write dynamic status file at {path}: {source}")]
    CannotWrite {
        /// Full path to the file that failed.
        path: String,
        /// Source IO error.
        source: std::io::Error,
    },
    /// Dynamic status data could not be serialized.
    #[error("Unable to serialize dynamic status file: {details}")]
    SerializeError {
        /// Serialization error details.
        details: String,
    },
    /// A dynamic circuit record is internally inconsistent or contains invalid data.
    #[error("Invalid dynamic circuit record: {details}")]
    InvalidRecord {
        /// Specific validation or conversion details.
        details: String,
    },
}

/// Root persisted state for the dynamic circuit runtime overlay.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Allocative)]
pub struct DynamicStatusFile {
    /// On-disk schema version for `dynamic_status.json`.
    pub version: u32,
    /// Monotonic ID allocator so `Dynamic <n>` values are never reused.
    pub next_dynamic_id: u64,
    /// Persisted dynamic circuits that can be materialized into overlay devices.
    pub circuits: Vec<DynamicCircuitRecord>,
}

impl DynamicStatusFile {
    /// Loads `dynamic_status.json`, returning an empty default state when the file is absent.
    pub fn load_or_default() -> Result<Self, DynamicStatusError> {
        let path = dynamic_status_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(&path).map_err(|source| DynamicStatusError::CannotRead {
            path: path.display().to_string(),
            source,
        })?;
        let parsed: Self = serde_json::from_str(&raw).map_err(|error| DynamicStatusError::ParseError {
            path: path.display().to_string(),
            details: error.to_string(),
        })?;
        if parsed.version != DYNAMIC_STATUS_VERSION {
            return Err(DynamicStatusError::InvalidRecord {
                details: format!(
                    "Unsupported dynamic_status.json version {} at {} (expected {})",
                    parsed.version,
                    path.display(),
                    DYNAMIC_STATUS_VERSION
                ),
            });
        }
        Ok(parsed)
    }

    /// Saves `dynamic_status.json` using a same-directory temp file and atomic rename.
    pub fn save_atomic(&self) -> Result<(), DynamicStatusError> {
        let path = dynamic_status_path()?;
        let tmp_path = path.with_file_name(format!("{DYNAMIC_STATUS_FILENAME}.tmp"));
        let serialized =
            serde_json::to_string_pretty(self).map_err(|error| DynamicStatusError::SerializeError {
                details: error.to_string(),
            })?;

        fs::write(&tmp_path, serialized).map_err(|source| DynamicStatusError::CannotWrite {
            path: tmp_path.display().to_string(),
            source,
        })?;
        fs::rename(&tmp_path, &path).map_err(|source| DynamicStatusError::CannotWrite {
            path: path.display().to_string(),
            source,
        })?;
        Ok(())
    }

    /// Allocates default circuit/device identifiers when callers omit them.
    pub fn allocate_default_ids(
        &mut self,
        circuit_id: Option<String>,
        device_id: Option<String>,
    ) -> (String, String) {
        let circuit_id = circuit_id.unwrap_or_else(|| {
            let allocated = format!("Dynamic {}", self.next_dynamic_id);
            self.next_dynamic_id += 1;
            allocated
        });
        let device_id = device_id.unwrap_or_else(|| circuit_id.clone());
        (circuit_id, device_id)
    }
}

/// Persisted attachment target for a dynamic circuit.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Allocative)]
pub struct DynamicAttachmentTarget {
    /// Stable topology node identifier when known.
    pub node_id: Option<String>,
    /// Human-readable parent node name.
    pub node_name: String,
}

/// Persisted dynamic circuit record.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Allocative)]
pub struct DynamicCircuitRecord {
    /// Stable circuit identifier.
    pub circuit_id: String,
    /// Stable device identifier.
    pub device_id: String,
    /// Optional human-readable circuit name.
    pub circuit_name: Option<String>,
    /// Optional human-readable device name.
    pub device_name: Option<String>,
    /// Attachment target used to place the circuit in the topology.
    pub attachment: DynamicAttachmentTarget,
    /// Optional MAC address associated with the device.
    pub mac: Option<String>,
    /// IP/CIDR strings assigned to the dynamic circuit.
    pub ip_cidrs: Vec<String>,
    /// Minimum guaranteed downstream rate in Mbps.
    pub download_min_mbps: f32,
    /// Minimum guaranteed upstream rate in Mbps.
    pub upload_min_mbps: f32,
    /// Maximum downstream rate in Mbps.
    pub download_max_mbps: f32,
    /// Maximum upstream rate in Mbps.
    pub upload_max_mbps: f32,
    /// Optional SQM override.
    pub sqm_override: Option<String>,
    /// TTL in seconds applied when extending expiry on activity.
    pub ttl_seconds: u64,
    /// Unix timestamp when the record was created.
    pub created: u64,
    /// Unix timestamp when the record was last observed active.
    #[serde(alias = "last_seen")]
    pub last_seen_at: u64,
    /// Unix timestamp when the record should expire if not refreshed.
    #[serde(alias = "expires")]
    pub expires_at: u64,
}

impl DynamicCircuitRecord {
    /// Converts the persisted record into a `ShapedDevice` for overlay indexing.
    pub fn to_shaped_device(&self) -> Result<crate::ShapedDevice, DynamicStatusError> {
        let mut ipv4 = Vec::new();
        let mut ipv6 = Vec::new();
        for cidr in &self.ip_cidrs {
            if let Ok(parsed) = crate::ShapedDevice::parse_cidr_v4(cidr) {
                ipv4.push(parsed);
                continue;
            }
            if let Ok(parsed) = crate::ShapedDevice::parse_cidr_v6(cidr) {
                ipv6.push(parsed);
                continue;
            }
            return Err(DynamicStatusError::InvalidRecord {
                details: format!("Unable to parse CIDR '{cidr}' for dynamic circuit '{}'", self.circuit_id),
            });
        }

        let circuit_name = self
            .circuit_name
            .clone()
            .unwrap_or_else(|| self.circuit_id.clone());
        let device_name = self
            .device_name
            .clone()
            .unwrap_or_else(|| self.device_id.clone());
        let parent_node = self.attachment.node_name.clone();

        Ok(crate::ShapedDevice {
            circuit_hash: lqos_utils::hash_to_i64(&self.circuit_id),
            device_hash: lqos_utils::hash_to_i64(&self.device_id),
            parent_hash: lqos_utils::hash_to_i64(&parent_node),
            circuit_id: self.circuit_id.clone(),
            circuit_name,
            device_id: self.device_id.clone(),
            device_name,
            parent_node,
            mac: self.mac.clone().unwrap_or_default(),
            ipv4,
            ipv6,
            download_min_mbps: self.download_min_mbps,
            upload_min_mbps: self.upload_min_mbps,
            download_max_mbps: self.download_max_mbps,
            upload_max_mbps: self.upload_max_mbps,
            comment: String::new(),
            sqm_override: self.sqm_override.clone(),
        })
    }
}

fn dynamic_status_path_for_config(config: &crate::Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(DYNAMIC_STATUS_FILENAME)
}

/// Resolves the active `dynamic_status.json` path from the LibreQoS config.
pub fn dynamic_status_path() -> Result<PathBuf, DynamicStatusError> {
    let config = crate::load_config().map_err(|_| DynamicStatusError::ConfigLoadError)?;
    Ok(dynamic_status_path_for_config(&config))
}

#[cfg(test)]
mod tests {
    use super::{
        DYNAMIC_STATUS_FILENAME, DYNAMIC_STATUS_VERSION, DynamicAttachmentTarget,
        DynamicCircuitRecord, DynamicStatusFile,
        dynamic_status_path,
    };
    use crate::{Config, clear_cached_config};
    use std::{
        fs,
        path::PathBuf,
        sync::Mutex,
        sync::atomic::{AtomicU64, Ordering},
    };

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);
    static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn unique_test_dir(prefix: &str) -> PathBuf {
        let suffix = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("libreqos-{prefix}-{}-{suffix}", std::process::id()))
    }

    #[test]
    fn dynamic_status_path_ends_with_filename() {
        let _guard = TEST_ENV_LOCK.lock().expect("lock test env");
        clear_cached_config();

        let config_dir = unique_test_dir("dynamic-status-path");
        let config_path = config_dir.join("lqos.conf");
        fs::create_dir_all(&config_dir).expect("create config dir");
        let mut config = Config::default();
        config.lqos_directory = config_dir.to_string_lossy().to_string();
        let config_toml = toml::to_string(&config).expect("serialize config");
        fs::write(&config_path, config_toml).expect("write config");

        let old_lqos_config = std::env::var_os("LQOS_CONFIG");
        let old_lqos_directory = std::env::var_os("LQOS_DIRECTORY");
        unsafe {
            std::env::set_var("LQOS_CONFIG", &config_path);
            std::env::set_var("LQOS_DIRECTORY", &config_dir);
        }

        let resolved = dynamic_status_path().expect("dynamic_status_path");
        assert!(resolved.ends_with(DYNAMIC_STATUS_FILENAME));

        match old_lqos_config {
            Some(value) => unsafe { std::env::set_var("LQOS_CONFIG", value) },
            None => unsafe { std::env::remove_var("LQOS_CONFIG") },
        }
        match old_lqos_directory {
            Some(value) => unsafe { std::env::set_var("LQOS_DIRECTORY", value) },
            None => unsafe { std::env::remove_var("LQOS_DIRECTORY") },
        }
        clear_cached_config();
        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_dir_all(&config_dir);
    }

    #[test]
    fn load_or_default_missing_file_returns_empty_status() {
        let _guard = TEST_ENV_LOCK.lock().expect("lock test env");
        clear_cached_config();

        let config_dir = unique_test_dir("dynamic-status-missing");
        let config_path = config_dir.join("lqos.conf");
        fs::create_dir_all(&config_dir).expect("create config dir");
        let mut config = Config::default();
        config.lqos_directory = config_dir.to_string_lossy().to_string();
        let config_toml = toml::to_string(&config).expect("serialize config");
        fs::write(&config_path, config_toml).expect("write config");

        let old_lqos_config = std::env::var_os("LQOS_CONFIG");
        let old_lqos_directory = std::env::var_os("LQOS_DIRECTORY");
        unsafe {
            std::env::set_var("LQOS_CONFIG", &config_path);
            std::env::set_var("LQOS_DIRECTORY", &config_dir);
        }

        let loaded = DynamicStatusFile::load_or_default().expect("load_or_default");
        assert_eq!(loaded.circuits.len(), 0);

        match old_lqos_config {
            Some(value) => unsafe { std::env::set_var("LQOS_CONFIG", value) },
            None => unsafe { std::env::remove_var("LQOS_CONFIG") },
        }
        match old_lqos_directory {
            Some(value) => unsafe { std::env::set_var("LQOS_DIRECTORY", value) },
            None => unsafe { std::env::remove_var("LQOS_DIRECTORY") },
        }
        clear_cached_config();
        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_dir_all(&config_dir);
    }

    #[test]
    fn save_atomic_then_load_round_trip() {
        let _guard = TEST_ENV_LOCK.lock().expect("lock test env");
        clear_cached_config();

        let config_dir = unique_test_dir("dynamic-status-save");
        let config_path = config_dir.join("lqos.conf");
        fs::create_dir_all(&config_dir).expect("create config dir");
        let mut config = Config::default();
        config.lqos_directory = config_dir.to_string_lossy().to_string();
        let config_toml = toml::to_string(&config).expect("serialize config");
        fs::write(&config_path, config_toml).expect("write config");

        let old_lqos_config = std::env::var_os("LQOS_CONFIG");
        let old_lqos_directory = std::env::var_os("LQOS_DIRECTORY");
        unsafe {
            std::env::set_var("LQOS_CONFIG", &config_path);
            std::env::set_var("LQOS_DIRECTORY", &config_dir);
        }

        let status = DynamicStatusFile {
            version: DYNAMIC_STATUS_VERSION,
            next_dynamic_id: 43,
            circuits: vec![super::DynamicCircuitRecord {
                circuit_id: "Dynamic 42".to_string(),
                device_id: "Dynamic 42".to_string(),
                circuit_name: Some("Transient Subscriber".to_string()),
                device_name: Some("Transient CPE".to_string()),
                attachment: super::DynamicAttachmentTarget {
                    node_id: Some("tower-a-sector-3".to_string()),
                    node_name: "Tower A / Sector 3".to_string(),
                },
                mac: Some("aa:bb:cc:dd:ee:ff".to_string()),
                ip_cidrs: vec!["192.0.2.10/32".to_string(), "2001:db8::1234/128".to_string()],
                download_min_mbps: 25.0,
                upload_min_mbps: 5.0,
                download_max_mbps: 150.0,
                upload_max_mbps: 30.0,
                sqm_override: Some("cake".to_string()),
                ttl_seconds: 3600,
                created: 1_775_606_400,
                last_seen_at: 1_775_608_200,
                expires_at: 1_775_611_800,
            }],
        };
        status.save_atomic().expect("save_atomic");
        let loaded = DynamicStatusFile::load_or_default().expect("load_or_default");
        assert_eq!(loaded, status);

        match old_lqos_config {
            Some(value) => unsafe { std::env::set_var("LQOS_CONFIG", value) },
            None => unsafe { std::env::remove_var("LQOS_CONFIG") },
        }
        match old_lqos_directory {
            Some(value) => unsafe { std::env::set_var("LQOS_DIRECTORY", value) },
            None => unsafe { std::env::remove_var("LQOS_DIRECTORY") },
        }
        clear_cached_config();
        let _ = fs::remove_file(config_dir.join(DYNAMIC_STATUS_FILENAME));
        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_dir_all(&config_dir);
    }

    #[test]
    fn allocate_default_ids_increments_next_dynamic_id() {
        let mut status = DynamicStatusFile {
            version: DYNAMIC_STATUS_VERSION,
            next_dynamic_id: 42,
            circuits: Vec::new(),
        };

        let first = status.allocate_default_ids(None, None);
        let second = status.allocate_default_ids(None, None);

        assert_eq!(first.0, "Dynamic 42");
        assert_eq!(first.1, "Dynamic 42");
        assert_eq!(second.0, "Dynamic 43");
        assert_eq!(second.1, "Dynamic 43");
        assert_eq!(status.next_dynamic_id, 44);
    }

    #[test]
    fn to_shaped_device_sets_hashes_from_ids() {
        let record = DynamicCircuitRecord {
            circuit_id: "Dynamic 42".to_string(),
            device_id: "Dynamic 42".to_string(),
            circuit_name: Some("Transient Subscriber".to_string()),
            device_name: Some("Transient CPE".to_string()),
            attachment: DynamicAttachmentTarget {
                node_id: Some("tower-a-sector-3".to_string()),
                node_name: "Tower A / Sector 3".to_string(),
            },
            mac: Some("aa:bb:cc:dd:ee:ff".to_string()),
            ip_cidrs: vec!["192.0.2.10/32".to_string(), "2001:db8::1234/128".to_string()],
            download_min_mbps: 25.0,
            upload_min_mbps: 5.0,
            download_max_mbps: 150.0,
            upload_max_mbps: 30.0,
            sqm_override: Some("cake".to_string()),
            ttl_seconds: 3600,
            created: 1_775_606_400,
            last_seen_at: 1_775_608_200,
            expires_at: 1_775_611_800,
        };

        let device = record.to_shaped_device().expect("to_shaped_device");
        assert_eq!(device.circuit_hash, lqos_utils::hash_to_i64(&device.circuit_id));
    }

    #[test]
    fn to_shaped_device_ipv4_cidr_uses_ipv6_mapped_prefix() {
        let record = DynamicCircuitRecord {
            circuit_id: "Dynamic 7".to_string(),
            device_id: "Dynamic 7".to_string(),
            circuit_name: None,
            device_name: None,
            attachment: DynamicAttachmentTarget {
                node_id: None,
                node_name: "Orphans".to_string(),
            },
            mac: None,
            ip_cidrs: vec!["192.0.2.10/24".to_string()],
            download_min_mbps: 10.0,
            upload_min_mbps: 10.0,
            download_max_mbps: 20.0,
            upload_max_mbps: 20.0,
            sqm_override: None,
            ttl_seconds: 3600,
            created: 1,
            last_seen_at: 1,
            expires_at: 2,
        };

        let device = record.to_shaped_device().expect("to_shaped_device");
        let ipv6_list = device.to_ipv6_list();
        assert_eq!(ipv6_list.len(), 1);
        let (mapped, prefix) = ipv6_list[0];
        assert_eq!(mapped, std::net::Ipv4Addr::new(192, 0, 2, 10).to_ipv6_mapped());
        assert_eq!(prefix, 120);
    }

    #[test]
    fn to_shaped_device_invalid_ip_reports_offending_string() {
        let record = DynamicCircuitRecord {
            circuit_id: "Dynamic 99".to_string(),
            device_id: "Dynamic 99".to_string(),
            circuit_name: None,
            device_name: None,
            attachment: DynamicAttachmentTarget {
                node_id: None,
                node_name: "Orphans".to_string(),
            },
            mac: None,
            ip_cidrs: vec!["not-an-ip".to_string()],
            download_min_mbps: 10.0,
            upload_min_mbps: 10.0,
            download_max_mbps: 20.0,
            upload_max_mbps: 20.0,
            sqm_override: None,
            ttl_seconds: 3600,
            created: 1,
            last_seen_at: 1,
            expires_at: 2,
        };

        let error = record.to_shaped_device().expect_err("invalid CIDR should fail");
        let message = error.to_string();
        assert!(message.contains("not-an-ip"));
    }

    #[test]
    fn example_fixture_parses_ttl_fields() {
        let parsed: DynamicStatusFile =
            serde_json::from_str(include_str!("testdata/dynamic_status.example.json"))
                .expect("parse example fixture");
        assert_eq!(parsed.version, DYNAMIC_STATUS_VERSION);
        assert_eq!(parsed.circuits.len(), 1);
        assert_eq!(parsed.circuits[0].ttl_seconds, 3600);
        assert_eq!(parsed.circuits[0].last_seen_at, 1_775_608_200);
        assert_eq!(parsed.circuits[0].expires_at, 1_775_611_800);
    }

    #[test]
    fn legacy_ttl_field_names_still_parse() {
        let parsed: DynamicStatusFile = serde_json::from_str(
            r#"{
  "version": 1,
  "next_dynamic_id": 2,
  "circuits": [
    {
      "circuit_id": "Dynamic 1",
      "device_id": "Dynamic 1",
      "attachment": { "node_id": null, "node_name": "AP-1" },
      "mac": null,
      "ip_cidrs": ["100.64.0.10/32"],
      "download_min_mbps": 25.0,
      "upload_min_mbps": 10.0,
      "download_max_mbps": 50.0,
      "upload_max_mbps": 20.0,
      "sqm_override": null,
      "ttl_seconds": 3600,
      "created": 1,
      "last_seen": 2,
      "expires": 3602
    }
  ]
}"#,
        )
        .expect("parse legacy ttl field names");
        assert_eq!(parsed.circuits[0].last_seen_at, 2);
        assert_eq!(parsed.circuits[0].expires_at, 3602);
    }

    #[test]
    fn load_or_default_rejects_unsupported_version() {
        let _guard = TEST_ENV_LOCK.lock().expect("lock test env");
        clear_cached_config();

        let config_dir = unique_test_dir("dynamic-status-bad-version");
        let config_path = config_dir.join("lqos.conf");
        fs::create_dir_all(&config_dir).expect("create config dir");
        let mut config = Config::default();
        config.lqos_directory = config_dir.to_string_lossy().to_string();
        let config_toml = toml::to_string(&config).expect("serialize config");
        fs::write(&config_path, config_toml).expect("write config");

        let old_lqos_config = std::env::var_os("LQOS_CONFIG");
        let old_lqos_directory = std::env::var_os("LQOS_DIRECTORY");
        unsafe {
            std::env::set_var("LQOS_CONFIG", &config_path);
            std::env::set_var("LQOS_DIRECTORY", &config_dir);
        }

        let status_path = config_dir.join(DYNAMIC_STATUS_FILENAME);
        fs::write(
            &status_path,
            r#"{"version":99,"next_dynamic_id":1,"circuits":[]}"#,
        )
        .expect("write dynamic status");

        let error = DynamicStatusFile::load_or_default().expect_err("unsupported version");
        assert!(matches!(error, super::DynamicStatusError::InvalidRecord { .. }));

        match old_lqos_config {
            Some(value) => unsafe { std::env::set_var("LQOS_CONFIG", value) },
            None => unsafe { std::env::remove_var("LQOS_CONFIG") },
        }
        match old_lqos_directory {
            Some(value) => unsafe { std::env::set_var("LQOS_DIRECTORY", value) },
            None => unsafe { std::env::remove_var("LQOS_DIRECTORY") },
        }
        clear_cached_config();
        let _ = fs::remove_file(&status_path);
        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_dir_all(&config_dir);
    }
}
