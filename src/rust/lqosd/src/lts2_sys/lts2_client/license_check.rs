use std::sync::Arc;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;
use native_tls::TlsConnector;
use timerfd::{SetTimeFlags, TimerFd, TimerState};
use tracing::{error, info, warn};
use uuid::Uuid;
use lqos_config::load_config;
use crate::lts2_sys::lts2_client::get_remote_host;

pub(crate) struct LicenseStatus {
    pub(crate) valid: bool,
    pub(crate) license_type: i32,
    pub(crate) trial_expires: i32,
}

impl Default for LicenseStatus {
    fn default() -> Self {
        LicenseStatus {
            valid: false,
            license_type: -1,
            trial_expires: -1,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct LicenseRequest {
    pub license: Uuid,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LicenseResponse {
    pub valid: bool,
    pub license_state: i32,
    pub expiration_date: i64,
}

pub(crate) fn license_check_loop(
    license_status: Arc<Mutex<LicenseStatus>>,
) {
    let mut tfd = TimerFd::new().unwrap();
    assert_eq!(tfd.get_state(), TimerState::Disarmed);
    tfd.set_state(TimerState::Periodic{
        current: Duration::new(60 * 15, 0),
        interval: Duration::new(60 * 15, 0)}
                  , SetTimeFlags::Default
    );
    
    loop {
        let license_key = load_config().unwrap().long_term_stats.license_key.clone().unwrap_or_default();
        
        if !license_key.is_empty() {
            if let Ok(lic) = Uuid::parse_str(&license_key) {
                let remote_host = get_remote_host();
                remote_license_check(remote_host, lic, license_status.clone());
            } else {
                println!("Invalid license key: {}", license_key);
            }
        }
        tfd.read();
    }
}

fn remote_license_check(
    remote_host: String,
    lic: Uuid,
    license_status: Arc<Mutex<LicenseStatus>>,
) {
    info!("Checking license key with remote host: {}", remote_host);
    let url = format!("https://{}/license/license_check", remote_host);
    info!("License Check URL: {}", url);
    // Make a ureq request to the remote host. POST a LicenseRequest with the license key.

    let Ok(tls) = TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build() else {
            error!("Failed to build TLS connector.");
            return;
    };
    let tls = Arc::new(tls);

    let client = ureq::builder()
        .timeout_connect(Duration::from_secs(20))
        .tls_connector(tls.clone())
        .build();

    let result = client
        .post(&url)
        .send_json(serde_json::json!(&LicenseRequest { license: lic }));
    if result.is_err() {
        warn!("Failed to connect to license server. This is not fatal - we'll try again. {result:?}");
        return;
    }
    let Ok(response) = result else {
        warn!("Failed to receive license response from license server.");
        return;
    };
    let response = response.into_json::<LicenseResponse>();
    if response.is_err() {
        warn!("Failed to receive license response from license server.");
        return;
    }
    let response = response.unwrap();
    info!("Received license response from license server: {response:?}");
    let mut license_lock = license_status.lock().unwrap();
    license_lock.valid = response.valid;
    license_lock.license_type = response.license_state;
    license_lock.trial_expires = response.expiration_date as i32;
}