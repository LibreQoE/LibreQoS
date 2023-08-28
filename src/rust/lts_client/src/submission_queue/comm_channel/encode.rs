use dryoc::{dryocbox::{Nonce, DryocBox}, types::{NewByteArray, ByteArray}};
use lqos_config::EtcLqos;
use thiserror::Error;
use crate::{transport_data::{LtsCommand, NodeIdAndLicense, HelloVersion2}, submission_queue::queue::QueueError};
use super::keys::{SERVER_PUBLIC_KEY, KEYPAIR};

pub(crate) async fn encode_submission_hello(license_key: &str, node_id: &str, node_name: &str) -> Result<Vec<u8>, QueueError> {
    let mut result = Vec::new();

    // Build the body
    let hello_message = HelloVersion2 {
        license_key: license_key.to_string(),
        node_id: node_id.to_string(),
        node_name: node_name.to_string(),
        client_public_key: KEYPAIR.read().await.public_key.clone().to_vec(),
    };

    // Add the version
    result.extend(2u16.to_be_bytes());

    // Pad to 32-bit boundary
    result.extend(3u16.to_be_bytes());

    // Serialize the body
    let hello_bytes = serde_cbor::to_vec(&hello_message).map_err(|_| QueueError::SendFail)?;

    // Add the length
    result.extend((hello_bytes.len() as u64).to_be_bytes());

    // Add the body
    result.extend(hello_bytes);

    Ok(result)
}

#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum SubmissionDecodeError {
    #[error("Invalid version")]
    InvalidVersion,
    #[error("Invalid padding")]
    InvalidPadding,
    #[error("Failed to deserialize")]
    Deserialize,
}

#[allow(dead_code)]
pub(crate) fn decode_submission_hello(bytes: &[u8]) -> Result<HelloVersion2, SubmissionDecodeError> {
    let version = u16::from_be_bytes([bytes[0], bytes[1]]);
    if version != 2 {
        log::error!("Received an invalid version from the server: {}", version);
        return Err(SubmissionDecodeError::InvalidVersion);
    }
    let padding = u16::from_be_bytes([bytes[2], bytes[3]]);
    if padding != 3 {
        log::error!("Received an invalid padding from the server: {}", padding);
        return Err(SubmissionDecodeError::InvalidPadding);
    }
    let size = u64::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11]]);
    let hello_bytes = &bytes[12..12 + size as usize];
    let hello: HelloVersion2 = serde_cbor::from_slice(hello_bytes).map_err(|_| SubmissionDecodeError::Deserialize)?;

    Ok(hello)
}

pub(crate) async fn encode_submission(submission: &LtsCommand) -> Result<Vec<u8>, QueueError> {
    let nonce = Nonce::gen();
    let mut result = Vec::new();

    // Store the version as network order
    result.extend(1u16.to_be_bytes());

    // Pack the license key and node id into a header
    let header = get_license_key_and_node_id(&nonce)?;
    let header_bytes = serde_cbor::to_vec(&header).map_err(|_| QueueError::SendFail)?;

    // Store the size of the header and the header
    result.extend((header_bytes.len() as u64).to_be_bytes());
    result.extend(header_bytes);

    // Pack the submission body into bytes
    let payload_bytes = serde_cbor::to_vec(&submission).map_err(|_| QueueError::SendFail)?;

    // TODO: Compress it?
    let payload_bytes = miniz_oxide::deflate::compress_to_vec(&payload_bytes, 8);
    
    // Encrypt it
    let remote_public = SERVER_PUBLIC_KEY.read().await.clone().unwrap();
    let my_private = KEYPAIR.read().await.secret_key.clone();
    let dryocbox = DryocBox::encrypt_to_vecbox(
        &payload_bytes,
        &nonce,
        &remote_public,
        &my_private,
    ).map_err(|_| QueueError::SendFail)?;
    let encrypted_bytes = dryocbox.to_vec();

    // Store the size of the submission
    result.extend((encrypted_bytes.len() as u64).to_be_bytes());
    result.extend(encrypted_bytes);

    // Store the encrypted, zipped submission itself
    Ok(result)
}

fn get_license_key_and_node_id(nonce: &Nonce) -> Result<NodeIdAndLicense, QueueError> {
    let cfg = EtcLqos::load().map_err(|_| QueueError::SendFail)?;
    if let Some(node_id) = cfg.node_id {
        if let Some(lts) = &cfg.long_term_stats {
            if let Some(license_key) = &lts.license_key {
                return Ok(NodeIdAndLicense {
                    node_id,
                    license_key: license_key.clone(),
                    nonce: *nonce.as_array(),
                });
            }
        }
    }
    Err(QueueError::SendFail)
}

#[cfg(test)]
mod test {
    #[tokio::test]
    async fn hello_submission_roundtrip() {
        let license_key = "1234567890";
        let node_id = "node_id";
        let node_name = "node_name";
        let hello = super::encode_submission_hello(license_key, node_id, node_name).await.unwrap();
        let hello = super::decode_submission_hello(&hello).unwrap();
        assert_eq!(hello.license_key, license_key);
        assert_eq!(hello.node_id, node_id);
        assert_eq!(hello.node_name, node_name);
    }
}