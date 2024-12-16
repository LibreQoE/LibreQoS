use dryoc::dryocbox::KeyPair;

pub struct KeyStore {
    pub(crate) keys: KeyPair,
}

impl KeyStore {
    pub fn new() -> Self {
        Self {
            keys: KeyPair::gen(),
        }
    }
    
    pub fn public_key_as_cbor_bytes(&self) -> Vec<u8> {
        serde_cbor::to_vec(&self.keys.public_key).unwrap()
    }
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
        let serialized = serde_cbor::to_vec(&keypair).unwrap();
        let deserialized : KeyPair = serde_cbor::from_slice(&serialized).unwrap();
        assert_eq!(keypair, deserialized);
    }
}