use crate::transport_data::{ask_license_server, LicenseReply, ask_license_server_for_new_account};
use lqos_config::load_config;
use lqos_utils::unix_time::unix_now;
use once_cell::sync::Lazy;
use tokio::sync::RwLock;

#[derive(Default, Clone)]
struct LicenseStatus {
    key: String,
    state: LicenseState,
    last_check: u64,
}

#[derive(Default, Clone, PartialEq, Debug)]
pub(crate) enum LicenseState {
    #[default]
    Unknown,
    Denied,
    Valid {
        /// When does the license expire?
        expiry: u64,
        /// Host to which to send stats
        stats_host: String,
    },
}

static LICENSE_STATUS: Lazy<RwLock<LicenseStatus>> =
    Lazy::new(|| RwLock::new(LicenseStatus::default()));

pub(crate) async fn get_license_status() -> LicenseState {
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    if let Ok(unix_time) = unix_now() {
        let license_status = {
            LICENSE_STATUS.read().await.clone()
        };
        if license_status.state == LicenseState::Unknown || license_status.last_check < unix_time - (60 * 60) {
            return check_license(unix_time).await;
        }
        return license_status.state;
    }
    LicenseState::Unknown
}

const MISERLY_NO_KEY: &str = "IDontSupportDevelopersAndShouldFeelBad";

async fn check_license(unix_time: u64) -> LicenseState {
    log::info!("Checking LTS stats license");
    if let Ok(cfg) = load_config() {
        // The config file is good. Is LTS enabled?
        // If it isn't, we need to try very gently to see if a pending
        // request has been submitted.
        if cfg.long_term_stats.gather_stats && cfg.long_term_stats.license_key.is_some() {
            if let Some(key) = cfg.long_term_stats.license_key {
                if key == MISERLY_NO_KEY {
                    log::warn!("You are using the self-hosting license key. We'd be happy to sell you a real one.");
                    return LicenseState::Valid { expiry: 0, stats_host: "192.168.100.11:9127".to_string() }
                }

                let mut lock = LICENSE_STATUS.write().await;
                lock.last_check = unix_time;
                lock.key = key.clone();
                match ask_license_server(key.clone()).await {
                    Ok(state) => {
                        match state {
                            LicenseReply::Denied => {
                                log::warn!("License is in state: DENIED.");
                                lock.state = LicenseState::Denied;                                
                            }
                            LicenseReply::Valid{expiry, stats_host} => {
                                log::info!("License is in state: VALID.");
                                lock.state = LicenseState::Valid{
                                    expiry, stats_host
                                };
                            }
                            _ => {
                                log::warn!("Unexpected type of data received. Denying to be safe.");
                                lock.state = LicenseState::Denied; 
                            }
                        }
                        return lock.state.clone();
                    }
                    Err(e) => {
                        log::error!("Error checking licensing server");
                        log::error!("{e:?}");
                    }
                }
            }
        } else {
            // LTS is unconfigured - but not explicitly disabled.
            // So we need to check if we have a pending request.
            // If a license key has been assigned, then we'll setup
            // LTS. If it hasn't, we'll just return Unknown.
            if let Ok(result) = ask_license_server_for_new_account(cfg.node_id.to_string()).await {
                if let LicenseReply::NewActivation { license_key } = result {
                    // We have a new license!
                    let _ = lqos_config::enable_long_term_stats(license_key);
                    // Note that we're not doing anything beyond this - the next cycle
                    // will pick up on there actually being a license
                } else {
                    log::info!("No pending LTS license found");
                }
            }
        }
    } else {
        log::error!("Unable to load lqosd configuration. Not going to try.");
    }
    LicenseState::Unknown
}
