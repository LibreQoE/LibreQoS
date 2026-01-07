use dryoc::dryocbox::KeyPair;

pub struct KeyStore {
    pub(crate) keys: KeyPair,
}

impl KeyStore {
    pub fn new() -> Self {
        Self {
            keys: KeyPair::r#gen(),
        }
    }

    pub fn public_key_as_cbor_bytes(&self) -> Vec<u8> {
        serde_cbor::to_vec(&self.keys.public_key).unwrap_or(vec![])
    }
}

#[cfg(test)]
mod test {
    use dryoc::dryocbox::*;

    #[test]
    fn test_sealed_box_roundtrip() {
        let sender_keypair = KeyPair::r#gen();
        let recipient_keypair = KeyPair::r#gen();
        let nonce = Nonce::r#gen();
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
        let keypair = KeyPair::r#gen();
        let serialized = serde_cbor::to_vec(&keypair).expect("Cannot serialize keypair");
        let deserialized: KeyPair =
            serde_cbor::from_slice(&serialized).expect("Cannot deserialize keypair");
        assert_eq!(keypair, deserialized);
    }
}
