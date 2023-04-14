use std::sync::atomic::AtomicBool;
use dryoc::dryocbox::*;
use lqos_bus::long_term_stats::{StatsSubmission, exchange_keys_with_license_server, NodeIdAndLicense};
use lqos_config::EtcLqos;
use once_cell::sync::Lazy;
use tokio::{sync::Mutex, net::TcpStream, io::AsyncWriteExt};
use crate::long_term_stats::pki::store_server_public_key;
use super::pki::{KEYPAIR, SERVER_PUBLIC_KEY};

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

    let mut lock = QUEUE.queue.lock().await;
    if lock.is_empty() {
        return;
    }

    for s in lock.iter_mut() {
        let submission_buffer = encode_submission(s);
        log::info!("Encoded {} bytes", submission_buffer.len());
        let host = format!("{host}:9128");
        log::info!("Sending stats to {host}");
        let stream = TcpStream::connect(&host).await;
        if let Err(e) = &stream {
            if e.kind() == std::io::ErrorKind::NotFound {
                log::error!("Unable to access {host}. Check that lqosd is running and you have appropriate permissions.");
            }
        }
        let mut stream = stream.unwrap(); // This unwrap is safe, we checked that it exists previously
        let ret = stream.write(&submission_buffer).await;
        if ret.is_err() {
            log::error!("Unable to write to {host} stream.");
            log::error!("{:?}", ret);
        } else {
            s.sent = true;
        }
    }

    lock.retain(|s| !s.sent);
    lock.retain(|s| s.attempts < 200);
}

fn get_license_key_and_node_id(nonce: &Nonce) -> NodeIdAndLicense {
    if let Ok(cfg) = EtcLqos::load() {
        if let Some(node_id) = cfg.node_id {
            if let Some(license_key) = cfg.long_term_stats.unwrap().license_key {
                return NodeIdAndLicense {
                    node_id,
                    license_key,
                    nonce: *nonce.as_array(),
                };
            }
        }
    }
    NodeIdAndLicense { node_id: String::new(), license_key: String::new(), nonce: [0; 24] }
}

fn encode_submission(submission: &QueueSubmission) -> Vec<u8> {
    let nonce = Nonce::gen();
    let mut result = Vec::new();

    // Store the version as network order
    result.extend(1u16.to_be_bytes());

    // Pack the license key and node id into a header
    let header = get_license_key_and_node_id(&nonce);
    let header_bytes = lqos_bus::cbor::to_vec(&header).unwrap();

    // Store the size of the header and the header
    result.extend((header_bytes.len() as u64).to_be_bytes());
    result.extend(header_bytes);

    // Pack the submission body into bytes
    let payload_bytes = lqos_bus::cbor::to_vec(&submission.body).unwrap();
    
    // Encrypt it
    let remote_public = SERVER_PUBLIC_KEY.read().unwrap().clone().unwrap();
    let my_private = KEYPAIR.read().unwrap().secret_key.clone();
    let dryocbox = DryocBox::encrypt_to_vecbox(
        &payload_bytes,
        &nonce,
        &remote_public,
        &my_private,
    )
    .expect("unable to encrypt");
    let encrypted_bytes = dryocbox.to_vec();

    // Store the size of the submission
    result.extend((encrypted_bytes.len() as u64).to_be_bytes());
    result.extend(encrypted_bytes);

    // Store the encrypted, zipped submission itself
    result
}