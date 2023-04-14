use std::{sync::RwLock, path::Path};
use dryoc::dryocbox::*;
use lqos_bus::bincode;
use lqos_config::EtcLqos;
use once_cell::sync::Lazy;

pub(crate) static KEYPAIR: Lazy<RwLock<KeyPair>> = Lazy::new(|| RwLock::new(generate_new_keypair()));
pub(crate) static SERVER_PUBLIC_KEY: Lazy<RwLock<Option<PublicKey>>> = Lazy::new(|| RwLock::new(None));

pub(crate) fn generate_new_keypair() -> KeyPair {
    if let Ok(cfg) = EtcLqos::load() {
        let path = Path::new(&cfg.lqos_directory).join("lts_keys.bin");
        if path.exists() {
            if let Ok(bytes) = std::fs::read(&path) {
                if let Ok(keypair) = bincode::deserialize(&bytes) {
                    log::info!("Loaded keypair from {}", path.display());
                    return keypair;
                }
            }
        }
        let keypair = KeyPair::gen();
        let bytes = bincode::serialize(&keypair).unwrap();
        std::fs::write(&path, bytes).unwrap();
        log::info!("Generated new keypair and stored it at {}", path.display());
        return keypair;
    }
    panic!("Cannot continue without a valid configuration file");
}

pub(crate) fn store_server_public_key(key: &PublicKey) {
    *SERVER_PUBLIC_KEY.write().unwrap() = Some(key.clone());
}

#[cfg(test)]
mod test {
    use dryoc::dryocbox::*;
    use lqos_bus::bincode;

    #[test]
    fn test_sealed_box_roundtrip() {
        let sender_keypair = KeyPair::gen();
        let recipient_keypair = KeyPair::gen();
        let nonce = Nonce::gen();
        let message = b"Once upon a time, there was a man with a dream.";
        let dryocbox = DryocBox::encrypt_to_vecbox(
            message,
            &nonce,
            &recipient_keypair.public_key,
            &sender_keypair.secret_key,
        )
        .expect("unable to encrypt");
        
        let sodium_box = dryocbox.to_vec();
        let dryocbox = DryocBox::from_bytes(&sodium_box).expect("failed to read box");
        let decrypted = dryocbox
            .decrypt_to_vec(
                &nonce,
                &sender_keypair.public_key,
                &recipient_keypair.secret_key,
            )
            .expect("unable to decrypt");
        
        assert_eq!(message, decrypted.as_slice());
    }

    #[test]
    fn test_serialize_keypair() {
        let keypair = KeyPair::gen();
        let serialized = bincode::serialize(&keypair).unwrap();
        let deserialized : KeyPair = bincode::deserialize(&serialized).unwrap();
        assert_eq!(keypair, deserialized);
    }
}