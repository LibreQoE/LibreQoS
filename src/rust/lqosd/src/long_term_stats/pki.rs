use std::sync::RwLock;
use lts_client::{pki::generate_new_keypair, dryoc::dryocbox::{KeyPair, PublicKey}};
use once_cell::sync::Lazy;

pub(crate) static KEYPAIR: Lazy<RwLock<KeyPair>> = Lazy::new(|| RwLock::new(generate_new_keypair("lts_keys.bin")));
pub(crate) static SERVER_PUBLIC_KEY: Lazy<RwLock<Option<PublicKey>>> = Lazy::new(|| RwLock::new(None));

pub(crate) fn store_server_public_key(key: &PublicKey) {
    *SERVER_PUBLIC_KEY.write().unwrap() = Some(key.clone());
}
