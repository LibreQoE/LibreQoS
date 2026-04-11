use crate::Config;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{info, warn};

const LEGACY_MIKROTIK_CSV_FILENAME: &str = "mikrotikDHCPRouterList.csv";

fn default_mikrotik_credentials_version() -> u32 {
    1
}

fn default_mikrotik_router_port() -> u16 {
    8728
}

fn default_use_ssl() -> bool {
    false
}

fn default_plaintext_login() -> bool {
    true
}

/// One Mikrotik router credential entry used for IPv6 enrichment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MikrotikIpv6RouterCredential {
    /// Operator-facing name for the router.
    pub name: String,
    /// Hostname or IP address of the Mikrotik API endpoint.
    pub host: String,
    /// API port.
    #[serde(default = "default_mikrotik_router_port")]
    pub port: u16,
    /// API username.
    pub username: String,
    /// API password.
    pub password: String,
    /// Whether to use SSL for the API connection.
    #[serde(default = "default_use_ssl")]
    pub use_ssl: bool,
    /// Whether to use legacy plaintext login mode for older RouterOS releases.
    #[serde(default = "default_plaintext_login")]
    pub plaintext_login: bool,
}

/// On-disk Mikrotik IPv6 credential file.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MikrotikIpv6CredentialsFile {
    /// Schema version for this file.
    #[serde(default = "default_mikrotik_credentials_version")]
    pub version: u32,
    /// Router credential entries.
    #[serde(default)]
    pub router: Vec<MikrotikIpv6RouterCredential>,
}

/// Errors returned while loading or migrating Mikrotik IPv6 credential data.
#[derive(Debug, Error)]
pub enum MikrotikIpv6CredentialError {
    /// The preferred Mikrotik IPv6 credential file was not found.
    #[error("Mikrotik IPv6 credential file was not found at {path}")]
    NotFound {
        /// Missing credential file path.
        path: String,
    },
    /// The credential file could not be read.
    #[error("Unable to read Mikrotik IPv6 credential file at {path}: {source}")]
    Read {
        /// Path that failed to read.
        path: String,
        /// Source I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The TOML credential file could not be parsed.
    #[error("Unable to parse Mikrotik IPv6 credential TOML at {path}: {details}")]
    ParseToml {
        /// Path that failed to parse.
        path: String,
        /// Parse failure details.
        details: String,
    },
    /// The legacy CSV credential file could not be parsed.
    #[error("Unable to parse legacy Mikrotik CSV at {path}: {details}")]
    ParseCsv {
        /// Path that failed to parse.
        path: String,
        /// Parse failure details.
        details: String,
    },
    /// A parent directory for the credential file could not be created.
    #[error("Unable to create directory {path}: {source}")]
    CreateDirectory {
        /// Directory path that failed to create.
        path: String,
        /// Source I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The TOML credential document could not be serialized.
    #[error("Unable to serialize Mikrotik IPv6 credentials: {details}")]
    Serialize {
        /// Serialization failure details.
        details: String,
    },
    /// The TOML credential file could not be written.
    #[error("Unable to write Mikrotik IPv6 credentials to {path}: {source}")]
    Write {
        /// Path that failed to write.
        path: String,
        /// Source I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The legacy CSV file could not be removed after a successful migration.
    #[error("Unable to remove legacy Mikrotik CSV at {path}: {source}")]
    DeleteLegacyCsv {
        /// Legacy CSV path that failed to delete.
        path: String,
        /// Source I/O error.
        #[source]
        source: std::io::Error,
    },
}

/// Loads Mikrotik IPv6 router credentials, auto-migrating the legacy CSV file if necessary.
pub fn load_mikrotik_ipv6_router_credentials(
    config: &Config,
) -> Result<Vec<MikrotikIpv6RouterCredential>, MikrotikIpv6CredentialError> {
    migrate_legacy_mikrotik_ipv6_credentials(config)?;

    let config_path = config.resolved_mikrotik_ipv6_config_path();
    if config_path.exists() {
        let credentials = read_credentials_toml(&config_path)?;
        let legacy_csv = config.legacy_runtime_file_path(LEGACY_MIKROTIK_CSV_FILENAME);
        if legacy_csv.exists() {
            warn!(
                "Legacy Mikrotik IPv6 CSV remains at {}; using {} instead.",
                legacy_csv.display(),
                config_path.display()
            );
        }
        return Ok(credentials.router);
    }

    let legacy_csv = config.legacy_runtime_file_path(LEGACY_MIKROTIK_CSV_FILENAME);
    if legacy_csv.exists() {
        return migrate_legacy_csv_to_toml(&legacy_csv, &config_path);
    }

    Err(MikrotikIpv6CredentialError::NotFound {
        path: config_path.display().to_string(),
    })
}

/// Migrates the legacy Mikrotik IPv6 CSV into TOML if needed.
///
/// This function has side effects: it may create the configured TOML file under `/etc/libreqos`
/// and delete the legacy CSV from `lqos_directory` after successful validation.
pub fn migrate_legacy_mikrotik_ipv6_credentials(
    config: &Config,
) -> Result<(), MikrotikIpv6CredentialError> {
    let config_path = config.resolved_mikrotik_ipv6_config_path();
    if config_path.exists() {
        return Ok(());
    }

    let legacy_csv = config.legacy_runtime_file_path(LEGACY_MIKROTIK_CSV_FILENAME);
    if legacy_csv.exists() {
        let _ = migrate_legacy_csv_to_toml(&legacy_csv, &config_path)?;
    }

    Ok(())
}

fn migrate_legacy_csv_to_toml(
    csv_path: &Path,
    config_path: &Path,
) -> Result<Vec<MikrotikIpv6RouterCredential>, MikrotikIpv6CredentialError> {
    let routers = read_credentials_csv(csv_path)?;
    let document = MikrotikIpv6CredentialsFile {
        version: default_mikrotik_credentials_version(),
        router: routers.clone(),
    };

    write_credentials_toml(config_path, &document)?;
    let validated = read_credentials_toml(config_path)?;
    fs::remove_file(csv_path).map_err(|source| MikrotikIpv6CredentialError::DeleteLegacyCsv {
        path: csv_path.display().to_string(),
        source,
    })?;

    info!(
        "Migrated legacy Mikrotik IPv6 CSV {} to {} and deleted the old CSV.",
        csv_path.display(),
        config_path.display()
    );

    Ok(validated.router)
}

fn read_credentials_toml(
    path: &Path,
) -> Result<MikrotikIpv6CredentialsFile, MikrotikIpv6CredentialError> {
    let raw = fs::read_to_string(path).map_err(|source| MikrotikIpv6CredentialError::Read {
        path: path.display().to_string(),
        source,
    })?;
    toml::from_str(&raw).map_err(|e| MikrotikIpv6CredentialError::ParseToml {
        path: path.display().to_string(),
        details: e.to_string(),
    })
}

fn read_credentials_csv(
    path: &Path,
) -> Result<Vec<MikrotikIpv6RouterCredential>, MikrotikIpv6CredentialError> {
    let file = File::open(path).map_err(|source| MikrotikIpv6CredentialError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(file);
    let mut routers = Vec::new();

    for result in reader.records() {
        let record = result.map_err(|e| MikrotikIpv6CredentialError::ParseCsv {
            path: path.display().to_string(),
            details: e.to_string(),
        })?;
        if record.len() != 5 {
            return Err(MikrotikIpv6CredentialError::ParseCsv {
                path: path.display().to_string(),
                details: format!("expected 5 columns, found {}", record.len()),
            });
        }
        let port =
            record[4]
                .trim()
                .parse::<u16>()
                .map_err(|e| MikrotikIpv6CredentialError::ParseCsv {
                    path: path.display().to_string(),
                    details: format!("invalid API port '{}': {e}", record[4].trim()),
                })?;
        routers.push(MikrotikIpv6RouterCredential {
            name: record[0].trim().to_string(),
            host: record[1].trim().to_string(),
            username: record[2].trim().to_string(),
            password: record[3].trim().to_string(),
            port,
            use_ssl: default_use_ssl(),
            plaintext_login: default_plaintext_login(),
        });
    }

    Ok(routers)
}

fn write_credentials_toml(
    path: &Path,
    document: &MikrotikIpv6CredentialsFile,
) -> Result<(), MikrotikIpv6CredentialError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| {
            MikrotikIpv6CredentialError::CreateDirectory {
                path: parent.display().to_string(),
                source,
            }
        })?;
    }

    let raw =
        toml::to_string_pretty(document).map_err(|e| MikrotikIpv6CredentialError::Serialize {
            details: e.to_string(),
        })?;
    let temp_path = temp_config_path(path);
    {
        let mut file = create_secure_file(&temp_path)?;
        file.write_all(raw.as_bytes())
            .map_err(|source| MikrotikIpv6CredentialError::Write {
                path: temp_path.display().to_string(),
                source,
            })?;
    }
    fs::rename(&temp_path, path).map_err(|source| MikrotikIpv6CredentialError::Write {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}

fn create_secure_file(path: &Path) -> Result<File, MikrotikIpv6CredentialError> {
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::os::unix::fs::OpenOptionsExt;

        OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)
            .map_err(|source| MikrotikIpv6CredentialError::Write {
                path: path.display().to_string(),
                source,
            })
    }

    #[cfg(not(unix))]
    {
        File::create(path).map_err(|source| MikrotikIpv6CredentialError::Write {
            path: path.display().to_string(),
            source,
        })
    }
}

fn temp_config_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("mikrotik_ipv6.toml");
    path.with_file_name(format!("{file_name}.tmp"))
}

#[cfg(test)]
mod tests {
    use super::{
        LEGACY_MIKROTIK_CSV_FILENAME, load_mikrotik_ipv6_router_credentials, read_credentials_toml,
    };
    use crate::Config;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("libreqos-mikrotik-{label}-{stamp}"))
    }

    fn test_config(root: &PathBuf) -> Config {
        let mut config = Config::default();
        config.lqos_directory = root.join("src").display().to_string();
        config.mikrotik_ipv6.config_path = root
            .join("etc/libreqos/mikrotik_ipv6.toml")
            .display()
            .to_string();
        config
    }

    #[test]
    fn migrates_legacy_csv_and_deletes_it() {
        let root = temp_path("migrate");
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join(LEGACY_MIKROTIK_CSV_FILENAME),
            "Router Name / ID,IP,API Username,API Password, API Port\nmain,100.64.0.1,admin,password,8728\n",
        )
        .unwrap();

        let config = test_config(&root);
        let routers = load_mikrotik_ipv6_router_credentials(&config).unwrap();

        assert_eq!(routers.len(), 1);
        assert_eq!(routers[0].name, "main");
        assert_eq!(routers[0].host, "100.64.0.1");
        assert!(root.join("etc/libreqos/mikrotik_ipv6.toml").exists());
        assert!(!src.join(LEGACY_MIKROTIK_CSV_FILENAME).exists());

        let saved = read_credentials_toml(&root.join("etc/libreqos/mikrotik_ipv6.toml")).unwrap();
        assert_eq!(saved.router, routers);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prefers_existing_toml_file() {
        let root = temp_path("prefer-toml");
        let src = root.join("src");
        let etc = root.join("etc/libreqos");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&etc).unwrap();
        fs::write(
            etc.join("mikrotik_ipv6.toml"),
            "version = 1\n\n[[router]]\nname = \"main\"\nhost = \"100.64.0.1\"\nport = 8728\nusername = \"admin\"\npassword = \"secret\"\nuse_ssl = false\nplaintext_login = true\n",
        )
        .unwrap();
        fs::write(
            src.join(LEGACY_MIKROTIK_CSV_FILENAME),
            "Router Name / ID,IP,API Username,API Password, API Port\nlegacy,100.64.0.2,user,pass,8728\n",
        )
        .unwrap();

        let config = test_config(&root);
        let routers = load_mikrotik_ipv6_router_credentials(&config).unwrap();

        assert_eq!(routers.len(), 1);
        assert_eq!(routers[0].name, "main");
        assert!(src.join(LEGACY_MIKROTIK_CSV_FILENAME).exists());

        let _ = fs::remove_dir_all(root);
    }
}
