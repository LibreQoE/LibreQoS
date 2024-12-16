use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use timerfd::{SetTimeFlags, TimerFd, TimerState};
use uuid::Uuid;
use lqos_config::load_config;
use crate::lts2_sys::lts2_client::nacl_blob;
use crate::lts2_sys::lts2_client::nacl_blob::KeyStore;

static ALLOWED_TO_SUBMIT: AtomicBool = AtomicBool::new(false);

pub(crate) fn is_allowed_to_submit() -> bool {
    ALLOWED_TO_SUBMIT.load(Ordering::Relaxed)
}

pub(crate) fn check_submit_permission() {
    let keys = Arc::new(KeyStore::new());

    let mut tfd = TimerFd::new().unwrap();
    assert_eq!(tfd.get_state(), TimerState::Disarmed);
    tfd.set_state(TimerState::Periodic{
        current: Duration::new(60 * 15, 0),
        interval: Duration::new(60 * 15, 0)}
                  , SetTimeFlags::Default
    );
    
    check_permission(keys.clone());

    // Periodically check if we're allowed to submit data
    loop {
        tfd.read();
        check_permission(keys.clone());
    }
}

fn check_permission(keys: Arc<KeyStore>) {
    println!("Checking for permission to submit");
    let config = load_config().unwrap();
    let remote_host = {
        config.long_term_stats.lts_url.clone().unwrap_or("insight.libreqos.com".to_string())
    };
    if let Ok(license_key) = Uuid::parse_str(&config.long_term_stats.license_key.clone().unwrap_or(String::default())) {
        if let Ok(mut socket) = TcpStream::connect(format!("{}:9122", remote_host)) {
            if let Err(e) = nacl_blob::transmit_hello(&keys, 0x8342, 3, &mut socket) {
                println!("Failed to send hello to license server. {e:?}");
                return;
            }

            if let Ok((server_hello, _)) = nacl_blob::receive_hello(&mut socket) {
                if let Err(e) = nacl_blob::transmit_payload(&keys, &server_hello.public_key, &license_key, &mut socket) {
                    println!("Failed to send license key to license server. {e:?}");
                    return;
                }
                if let Ok((response, _)) = nacl_blob::receive_payload::<bool>(&keys, &server_hello.public_key, &mut socket) {
                    println!("Received response from license server: {response}");
                    ALLOWED_TO_SUBMIT.store(response, Ordering::Relaxed);
                    return;
                } else {
                    println!("Failed to receive response from license server");
                }
            }
        }
    }
    ALLOWED_TO_SUBMIT.store(false, Ordering::Relaxed);
}