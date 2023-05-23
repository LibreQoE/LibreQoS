use std::sync::RwLock;
use once_cell::sync::Lazy;
use lts_client::{pki::generate_new_keypair, dryoc::dryocbox::KeyPair};

pub(crate) static LIBREQOS_KEYPAIR: Lazy<RwLock<KeyPair>> = Lazy::new(|| RwLock::new(generate_new_keypair(KEY_PATH)));
const KEY_PATH: &str = "lqkeys.bin"; // Store in the working directory
