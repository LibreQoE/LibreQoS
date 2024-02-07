use crate::{pki::generate_new_keypair, dryoc::dryocbox::{KeyPair, PublicKey}, transport_data::{exchange_keys_with_license_server, LicenseReply}};
use lqos_config::load_config;
use once_cell::sync::Lazy;
use tokio::sync::RwLock;

pub(crate) static KEYPAIR: Lazy<RwLock<KeyPair>> = Lazy::new(|| RwLock::new(generate_new_keypair()));
pub(crate) static SERVER_PUBLIC_KEY: Lazy<RwLock<Option<PublicKey>>> = Lazy::new(|| RwLock::new(None));

pub(crate) async fn store_server_public_key(key: &PublicKey) {
    *SERVER_PUBLIC_KEY.write().await = Some(key.clone());
}

pub(crate) async fn key_exchange() -> bool {
    let cfg = load_config().unwrap();
    let node_id = cfg.node_id.clone();
    let node_name = if !cfg.node_name.is_empty() {
        cfg.node_name
    } else {
        node_id.clone()
    };
    let license_key = cfg.long_term_stats.license_key.unwrap();
    let keypair = (KEYPAIR.read().await).clone();
    match exchange_keys_with_license_server(node_id, node_name, license_key, keypair.public_key.clone()).await {
        Ok(LicenseReply::MyPublicKey { public_key }) => {
            store_server_public_key(&public_key).await;
            log::info!("Received a public key for the server");
            true
        }
        Ok(_) => {
            log::warn!("License server sent an unexpected response.");
            false
        }
        Err(e) => {
            log::warn!("Error exchanging keys with license server: {}", e);
            false
        }
    }
}