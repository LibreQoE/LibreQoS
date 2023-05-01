use std::sync::RwLock;
use lts_client::{dryoc::dryocbox::*, pki::generate_new_keypair};
use once_cell::sync::Lazy;

pub(crate) static LIBREQOS_KEYPAIR: Lazy<RwLock<KeyPair>> = Lazy::new(|| RwLock::new(generate_new_keypair(KEY_PATH)));
const KEY_PATH: &str = "lqkeys.bin"; // Store in the working directory
