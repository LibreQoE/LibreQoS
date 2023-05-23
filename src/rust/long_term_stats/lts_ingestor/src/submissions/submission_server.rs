//! Provides a TCP handler server, listening on port 9128. Connections
//! are expected in the encrypted LTS format (see the `lq_bus` crate).
//! If everything checks out, they are sent to the submission queue
//! for storage.

use super::submission_queue::SubmissionType;
use crate::pki::LIBREQOS_KEYPAIR;
use lts_client::{
    dryoc::dryocbox::{DryocBox, PublicKey},
    transport_data::{LtsCommand, NodeIdAndLicense},
};
use pgdb::sqlx::{Pool, Postgres};
use tokio::{io::AsyncReadExt, net::{TcpListener, TcpStream}, spawn, sync::mpsc::Sender};
use tracing::{info, error, warn};

/// Starts the submission server, listening on port 9128.
/// The server runs in the background.
pub async fn submissions_server(
    cnn: Pool<Postgres>,
    sender: Sender<SubmissionType>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(":::9128").await?;
    info!("Listening for stats submissions on :::9128");

    loop {
        let (mut socket, address) = listener.accept().await?;
        info!("Connection from {address:?}");
        let pool = cnn.clone();
        let my_sender = sender.clone();
        spawn(async move {
            loop {
                if let Ok(message) = read_message(&mut socket, pool.clone()).await {
                    my_sender.send(message).await.unwrap();
                } else {
                    error!("Read failed. Dropping socket.");
                    std::mem::drop(socket);
                    break;
                }
            }
        });
    }
}

#[tracing::instrument]
async fn read_message(socket: &mut TcpStream, pool: Pool<Postgres>) -> anyhow::Result<SubmissionType> {
    read_version(socket).await?;
    let header_size = read_size(socket).await?;
    let header = read_header(socket, header_size as usize).await?;
    let body_size = read_size(socket).await?;
    let message = read_body(socket, pool.clone(), body_size as usize, &header).await?;
    Ok((header, message))
}

async fn read_version(stream: &mut TcpStream) -> anyhow::Result<()> {
    let version = stream.read_u16().await?;
    if version != 1 {
        warn!("Received a version {version} header.");
        return Err(anyhow::Error::msg("Received an unknown version header"));
    }
    Ok(())
}

async fn read_size(stream: &mut TcpStream) -> anyhow::Result<u64> {
    let size = stream.read_u64().await?;
    Ok(size)
}

async fn read_header(stream: &mut TcpStream, size: usize) -> anyhow::Result<NodeIdAndLicense> {
    let mut buffer = vec![0u8; size];
    let _bytes_read = stream.read(&mut buffer).await?;
    let header: NodeIdAndLicense = lts_client::cbor::from_slice(&buffer)?;
    Ok(header)
}

async fn read_body(stream: &mut TcpStream, pool: Pool<Postgres>, size: usize, header: &NodeIdAndLicense) -> anyhow::Result<LtsCommand> {
    info!("Reading body of size {size}");
    info!("{header:?}");

    let mut buffer = vec![0u8; size];
    let bytes_read = stream.read_exact(&mut buffer).await?;
    if bytes_read != size {
        warn!("Received a body of size {bytes_read}, expected {size}");
        return Err(anyhow::Error::msg("Received a body of unexpected size"));
    }

    // Check the header against the database and retrieve the current
    // public key
    let public_key = pgdb::fetch_public_key(pool, &header.license_key, &header.node_id).await?;
    let public_key: PublicKey = lts_client::cbor::from_slice(&public_key)?;
    let private_key = LIBREQOS_KEYPAIR.read().unwrap().secret_key.clone();

    // Decrypt
    let dryocbox = DryocBox::from_bytes(&buffer).expect("failed to read box");
    let decrypted = dryocbox
        .decrypt_to_vec(&header.nonce.into(), &public_key, &private_key)?;

    let decrypted = miniz_oxide::inflate::decompress_to_vec(&decrypted).expect("failed to decompress");

    // Try to deserialize
    let payload = lts_client::cbor::from_slice(&decrypted)?;
    Ok(payload)
}
