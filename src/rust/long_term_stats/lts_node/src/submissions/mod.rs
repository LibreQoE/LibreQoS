use std::net::SocketAddr;
use dryoc::dryocbox::*;
use lqos_bus::long_term_stats::{NodeIdAndLicense, StatsSubmission};
use pgdb::sqlx::{Pool, Postgres};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    spawn,
};

use crate::pki::LIBREQOS_KEYPAIR;

pub async fn submissions_server() -> anyhow::Result<()> {
    let listener = TcpListener::bind(":::9128").await?;
    log::info!("Listening for stats submissions on :::9128");

    let pool = pgdb::get_connection_pool(5).await;
    if pool.is_err() {
        log::error!("Unable to connect to the database");
        log::error!("{pool:?}");
        return Err(anyhow::Error::msg("Unable to connect to the database"));
    }
    let pool = pool.unwrap();

    loop {
        let (mut socket, address) = listener.accept().await?;
        log::info!("Connection from {address:?}");
        let pool = pool.clone();
        spawn(async move {
            let mut buffer = Vec::new();
            if let Ok(bytes) = socket.read_to_end(&mut buffer).await {
                log::info!("Received {bytes} bytes from {address:?}");
                match decode(&buffer, address, pool).await {
                    Ok(stats) => {
                        println!("{stats:?}");
                    }
                    Err(e) => log::error!("{e:?}"),
                }
            }
        });
    }
}

async fn decode(
    buf: &[u8],
    address: SocketAddr,
    pool: Pool<Postgres>,
) -> anyhow::Result<StatsSubmission> {
    const U64SIZE: usize = std::mem::size_of::<u64>();
    let version_buf = &buf[0..2].try_into()?;
    let version = u16::from_be_bytes(*version_buf);
    let size_buf = &buf[2..2 + U64SIZE].try_into()?;
    let size = u64::from_be_bytes(*size_buf);

    // Check the version
    log::info!("Received a version {version} header of serialized size {size} from {address:?}");
    if version != 1 {
        log::warn!("Received a version {version} header from {address:?}");
        return Err(anyhow::Error::msg("Received an unknown version header"));
    }

    // Read the header
    let start = 2 + U64SIZE;
    let end = start + size as usize;
    let header: NodeIdAndLicense = lqos_bus::cbor::from_slice(&buf[start..end])?;

    // Check the header against the database and retrieve the current
    // public key
    let public_key = pgdb::fetch_public_key(pool, &header.license_key, &header.node_id).await?;
    let public_key: PublicKey = lqos_bus::cbor::from_slice(&public_key)?;
    let private_key = LIBREQOS_KEYPAIR.read().unwrap().secret_key.clone();

    // Retrieve the payload size
    let size_buf = &buf[end .. end + U64SIZE].try_into()?;
    let size = u64::from_be_bytes(*size_buf);
    let payload_encrypted = &buf[end + U64SIZE .. end + U64SIZE + size as usize];
    
    // Decrypt
    let dryocbox = DryocBox::from_bytes(&payload_encrypted).expect("failed to read box");
    let decrypted = dryocbox
        .decrypt_to_vec(
            &header.nonce.into(),
            &public_key,
            &private_key,
        )
        .expect("unable to decrypt");

    // Try to deserialize
    let payload: StatsSubmission = lqos_bus::cbor::from_slice(&decrypted)?;

    Ok(payload)
}