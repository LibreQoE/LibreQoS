use dryoc::{dryocbox::{Nonce, DryocBox}, types::{NewByteArray, ByteArray}};
use lqos_config::EtcLqos;
use crate::{transport_data::{LtsCommand, NodeIdAndLicense}, submission_queue::queue::QueueError};
use super::keys::{SERVER_PUBLIC_KEY, KEYPAIR};

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