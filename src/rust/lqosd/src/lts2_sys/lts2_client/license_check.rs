use std::sync::Arc;
use serde::Deserialize;
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::Duration;
use timerfd::{SetTimeFlags, TimerFd, TimerState};
use uuid::Uuid;
use lqos_config::load_config;
use crate::lts2_sys::lts2_client::{get_remote_host, nacl_blob};
use crate::lts2_sys::lts2_client::nacl_blob::KeyStore;

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

#[derive(Deserialize, Debug)]
pub(crate) struct LicenseResponse {
    pub license_state: i32,
    pub expiration_date: i64,
}

pub(crate) fn license_check_loop(
    license_status: Arc<Mutex<LicenseStatus>>,
) {
    let keys = KeyStore::new();

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
                remote_license_check(remote_host, &keys, lic, license_status.clone());
            } else {
                println!("Invalid license key: {}", license_key);
            }
        }
        tfd.read();
    }
}

fn remote_license_check(
    remote_host: String,
    keys: &KeyStore,
    lic: Uuid,
    license_status: Arc<Mutex<LicenseStatus>>,
) {
    println!("Checking license key with remote host: {}", remote_host);
    if let Ok(mut socket) = TcpStream::connect(format!("{}:9122", remote_host)) {
        if let Err(e) = nacl_blob::transmit_hello(&keys, 0x8342, 1, &mut socket) {
            println!("Failed to send hello to license server. {e:?}");
            return;
        }

        if let Ok((server_hello, _)) = nacl_blob::receive_hello(&mut socket) {
            if let Err(e) = nacl_blob::transmit_payload(&keys, &server_hello.public_key, &lic, &mut socket) {
                println!("Failed to send license key to license server. {e:?}");
                return;
            }

            if let Ok((response, _)) = nacl_blob::receive_payload::<LicenseResponse>(&keys, &server_hello.public_key, &mut socket) {
                println!("Received license response from license server: {response:?}");
                let mut license_lock = license_status.lock().unwrap();
                license_lock.valid = response.license_state != 0;
                license_lock.license_type = response.license_state;
                license_lock.trial_expires = response.expiration_date as i32;

            } else {
                println!("Failed to receive license response from license server.");
                return;
            }
        } else {
            println!("Failed to receive hello from license server.");
            return;
        }
    } else {
        println!("Failed to connect to license server. This is not fatal - we'll try again.");
        return;
    }
}