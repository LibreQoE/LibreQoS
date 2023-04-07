use lqos_bus::long_term_stats::{ask_license_server, LicenseReply};
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

#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub(crate) enum LicenseState {
    #[default]
    Unknown,
    Denied,
    Valid,
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

async fn check_license(unix_time: u64) -> LicenseState {
    if let Ok(cfg) = EtcLqos::load() {
        if let Some(cfg) = cfg.long_term_stats {
            if let Some(key) = cfg.license_key {
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
                            LicenseReply::Valid => {
                                log::info!("License is in state: VALID.");
                                lock.state = LicenseState::Valid;
                            }
                        }
                        return lock.state;
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
