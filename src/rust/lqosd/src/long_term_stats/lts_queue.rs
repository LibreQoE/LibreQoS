use std::sync::atomic::AtomicBool;
use lqos_bus::long_term_stats::{StatsSubmission, exchange_keys_with_license_server};
use lqos_config::EtcLqos;
use once_cell::sync::Lazy;
use tokio::sync::Mutex;
use crate::long_term_stats::pki::store_server_public_key;
use super::pki::KEYPAIR;

struct QueueSubmission {
    attempts: u8,
    body: StatsSubmission,
    sent: bool,
}

pub(crate) struct Queue {
    queue: Mutex<Vec<QueueSubmission>>,
}

impl Queue {
    fn new() -> Self {
        Self {
            queue: Mutex::new(Vec::new()),
        }
    }

    pub async fn push(&self, data: lqos_bus::long_term_stats::StatsSubmission, host: &str) {
        {
            let mut lock = self.queue.lock().await;
            lock.push(QueueSubmission {
                attempts: 0,
                sent: false,
                body: data,
            });
        }
        tokio::spawn(send_queue(host.to_string()));
    }
}

pub(crate) static QUEUE: Lazy<Queue> = Lazy::new(Queue::new);
static DONE_KEY_EXCHANGE: AtomicBool = AtomicBool::new(false);

async fn send_queue(host: String) {
    if !DONE_KEY_EXCHANGE.load(std::sync::atomic::Ordering::Relaxed) {
        let cfg = EtcLqos::load().unwrap();
        let node_id = cfg.node_id.unwrap();
        let license_key = cfg.long_term_stats.unwrap().license_key.unwrap();
        let keypair = (KEYPAIR.read().unwrap()).clone();
        match exchange_keys_with_license_server(node_id, license_key, keypair.public_key.clone()).await {
            Ok(lqos_bus::long_term_stats::LicenseReply::MyPublicKey { public_key }) => {
                store_server_public_key(&public_key);
                log::info!("Received a public key for the server");
            }
            Ok(_) => {
                log::warn!("License server sent an unexpected response.");
                return;
            }
            Err(e) => {
                log::warn!("Error exchanging keys with license server: {}", e);
                return;
            }
        }

        DONE_KEY_EXCHANGE.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    if !DONE_KEY_EXCHANGE.load(std::sync::atomic::Ordering::Relaxed) {
        log::warn!("Not sending stats because key exchange failed.");
        return;
    }

    let url = format!("http://{host}:9127/submit");

    let mut lock = QUEUE.queue.lock().await;
    if lock.is_empty() {
        return;
    }

    for s in lock.iter_mut() {
        let client = reqwest::Client::new();
        let res = client.post(&url)
            .json(&s.body)
            .send();

        match res.await {
            Ok(_) => {
                s.sent = true;
            }
            Err(e) => {
                log::warn!("Error sending stats: {}", e);
                s.attempts += 1;
            }
        }
    }

    lock.retain(|s| !s.sent);
    lock.retain(|s| s.attempts < 200);
}