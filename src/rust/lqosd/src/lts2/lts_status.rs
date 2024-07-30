use std::sync::atomic::AtomicU32;
use lqos_config::load_config;
use crate::lts2::lts_client::get_lts_client;
use crate::lts2::lts_client::lts2_grpc::LicenseCheckRequest;

pub enum LtsStatus {
    NotChecked,
    AlwaysFree,
    Trial,
    SelfHosted,
    ApiOnly,
    Full,
    Invalid,
}

static LTS_STATUS: AtomicU32 = AtomicU32::new(LtsStatus::NotChecked as u32);
static LTS_TRIAL_EXPIRATION_DAYS: AtomicU32 = AtomicU32::new(0);

pub fn get_lts_status() -> LtsStatus {
    match LTS_STATUS.load(std::sync::atomic::Ordering::Relaxed) {
        0 => LtsStatus::NotChecked,
        1 => LtsStatus::AlwaysFree,
        2 => LtsStatus::Trial,
        3 => LtsStatus::SelfHosted,
        4 => LtsStatus::ApiOnly,
        5 => LtsStatus::Full,
        _ => LtsStatus::Invalid,
    }
}

pub fn get_lts_trial_days_remaining() -> u32 {
    LTS_TRIAL_EXPIRATION_DAYS.load(std::sync::atomic::Ordering::Relaxed)
}

pub async fn poll_lts_status() {
    do_license_poll().await;
    let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(60 * 15));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        do_license_poll().await;
    }
}

async fn do_license_poll() {
    log::info!("Polling LTS status");
    if let Ok(cfg) = load_config() {
        if let Some(key) = cfg.long_term_stats.license_key {
            if let Some(mut client) = get_lts_client().await {
                log::warn!("Checking license key: {}", key);
                let response = client.license_check(
                    tonic::Request::new(LicenseCheckRequest {
                    license_key: key.clone(),
                })).await.unwrap();
                //println!("RESPONSE={:?}", response);

                let r = response.into_inner();
                if !r.valid {
                    LTS_STATUS.store(LtsStatus::Invalid as u32, std::sync::atomic::Ordering::Relaxed);
                    log::info!("LTS2: License key is invalid");
                } else {
                    match r.license_type {
                        0 => LTS_STATUS.store(LtsStatus::AlwaysFree as u32, std::sync::atomic::Ordering::Relaxed),
                        1 => {
                            LTS_STATUS.store(LtsStatus::Trial as u32, std::sync::atomic::Ordering::Relaxed);
                            let expires: u32 = r.expires.parse().unwrap();
                            LTS_TRIAL_EXPIRATION_DAYS.store(expires, std::sync::atomic::Ordering::Relaxed);
                        },
                        2 => LTS_STATUS.store(LtsStatus::SelfHosted as u32, std::sync::atomic::Ordering::Relaxed),
                        3 => LTS_STATUS.store(LtsStatus::ApiOnly as u32, std::sync::atomic::Ordering::Relaxed),
                        4 => LTS_STATUS.store(LtsStatus::Full as u32, std::sync::atomic::Ordering::Relaxed),
                        _ => LTS_STATUS.store(LtsStatus::Invalid as u32, std::sync::atomic::Ordering::Relaxed),
                    }
                }
            }
        } else {
            log::warn!("No license key found, not polling.");
        }
    } else {
        log::debug!("No configuration found, not polling.");
    }
}