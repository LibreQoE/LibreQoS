use std::sync::RwLock;
use dryoc::dryocbox::*;
use once_cell::sync::Lazy;

pub(crate) static KEYPAIR: Lazy<RwLock<KeyPair>> = Lazy::new(|| RwLock::new(generate_new_keypair()));
pub(crate) static SERVER_PUBLIC_KEY: Lazy<RwLock<Option<PublicKey>>> = Lazy::new(|| RwLock::new(None));

pub(crate) fn generate_new_keypair() -> KeyPair {
    KeyPair::gen()
}

pub(crate) fn store_server_public_key(key: &PublicKey) {
    *SERVER_PUBLIC_KEY.write().unwrap() = Some(key.clone());
}