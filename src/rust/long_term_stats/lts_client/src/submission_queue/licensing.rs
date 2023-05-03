use crate::transport_data::{ask_license_server, LicenseReply};
use lqos_config::EtcLqos;
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
    if let Ok(cfg) = EtcLqos::load() {
        if let Some(cfg) = cfg.long_term_stats {
            if let Some(key) = cfg.license_key {
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
        }
    }
    LicenseState::Unknown
}
