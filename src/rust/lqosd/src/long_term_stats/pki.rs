use std::sync::RwLock;

use dryoc::dryocbox::*;
use once_cell::sync::Lazy;

static KEYPAIR: Lazy<RwLock<KeyPair>> = Lazy::new(|| RwLock::new(generate_new_keypair()));

pub(crate) fn generate_new_keypair() -> KeyPair {
    KeyPair::gen()
}