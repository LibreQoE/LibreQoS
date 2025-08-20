use crate::lts2_sys::lts2_client::license_check::{LicenseRequest, LicenseResponse};
use lqos_config::load_config;
use native_tls::TlsConnector;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use timerfd::{SetTimeFlags, TimerFd, TimerState};
use tracing::{error, info, warn};
use uuid::Uuid;

static ALLOWED_TO_SUBMIT: AtomicBool = AtomicBool::new(false);

pub(crate) fn is_allowed_to_submit() -> bool {
    ALLOWED_TO_SUBMIT.load(Ordering::Relaxed)
}

pub(crate) fn check_submit_permission() {
    let mut tfd = TimerFd::new().unwrap();
    assert_eq!(tfd.get_state(), TimerState::Disarmed);
    tfd.set_state(
        TimerState::Periodic {
            current: Duration::new(60 * 15, 0),
            interval: Duration::new(60 * 15, 0),
        },
        SetTimeFlags::Default,
    );

    check_permission();

    // Periodically check if we're allowed to submit data
    loop {
        tfd.read();
        check_permission();
    }
}

fn check_permission() {
    //println!("Checking for permission to submit");
    let Ok(config) = load_config() else {
        error!("Failed to load config");
        return;
    };
    if config.long_term_stats.gather_stats == false {
        info!("Long term stats are disabled. Not checking license.");
        ALLOWED_TO_SUBMIT.store(false, Ordering::Relaxed);
        return;
    }
    let remote_host = {
        config
            .long_term_stats
            .lts_url
            .clone()
            .unwrap_or("insight.libreqos.com".to_string())
    };
    let license_key = config
        .long_term_stats
        .license_key
        .clone()
        .unwrap_or_default();
    let Ok(license_key) = Uuid::parse_str(&license_key.replace("-", "")) else {
        warn!("Invalid license key format: [{license_key}]");
        return;
    };
    info!("Checking license key with remote host: {}", remote_host);
    let url = format!("https://{}/license/license_check", remote_host);
    info!("License Check URL: {}", url);
    // Make a ureq request to the remote host. POST a LicenseRequest with the license key.

    let Ok(tls) = TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
    else {
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
        .send_json(serde_json::json!(&LicenseRequest {
            license: license_key
        }));
    if result.is_err() {
        warn!(
            "Failed to connect to license server. This is not fatal - we'll try again. {result:?}"
        );
        return;
    }
    let Ok(response) = result else {
        warn!("Failed to receive license response from license server.");
        return;
    };
    let response = response.into_json::<LicenseResponse>();
    let Ok(response) = response else {
        warn!("Failed to parse license response from license server.");
        return;
    };
    info!("Received license response from license server: {response:?}");
    if response.valid {
        ALLOWED_TO_SUBMIT.store(true, Ordering::Relaxed);
    } else {
        ALLOWED_TO_SUBMIT.store(false, Ordering::Relaxed);
    }
}
