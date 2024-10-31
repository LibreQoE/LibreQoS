use dryoc::dryocbox::*;
use tracing::info;

/// Genereate a new keypair and store it in a file. If the file exists,
/// it will be loaded rather than re-generated.
/// 
/// # Arguments
/// 
/// * `key_path` - The path to the file to store the keypair in
/// 
/// # Returns
/// 
/// The generated or loaded keypair
pub fn generate_new_keypair() -> KeyPair {
    let keypair = KeyPair::gen();
    info!("Generated new keypair");
    keypair
}

#[cfg(test)]
mod test {
    use dryoc::dryocbox::*;

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