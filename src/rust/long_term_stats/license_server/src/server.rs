use crate::pki::LIBREQOS_KEYPAIR;
use lts_client::transport_data::{LicenseReply, LicenseRequest};
use pgdb::sqlx::{Pool, Postgres};
use std::net::SocketAddr;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    spawn,
};

/// Entry point for the main license server system.
/// Starts listening on port 9126 for license requests.
pub async fn listen_accept() -> anyhow::Result<()> {
    let listener = TcpListener::bind(":::9126").await?;
    tracing::info!("Listening on :::9126");

    let pool = pgdb::get_connection_pool(10).await;
    if pool.is_err() {
        tracing::error!("Unable to connect to the database");
        tracing::error!("{pool:?}");
        return Err(anyhow::Error::msg("Unable to connect to the database"));
    }
    let pool = pool.unwrap();

    loop {
        let (socket, address) = listener.accept().await?;
        tracing::info!("Connection from {address:?}");
        let pool = pool.clone();
        spawn(async move {
            handle_connection(socket, address, pool)
        });
    }
}

#[tracing::instrument(skip(socket, pool))]
async fn handle_connection(mut socket: TcpStream, address: SocketAddr, pool: Pool<Postgres>) {
    let mut buf = vec![0u8; 10240];
    if let Ok(bytes) = socket.read(&mut buf).await {
        // Graceful shutdown handler
        if bytes == 0 {
            tracing::info!("Connection closed by peer");
            return;
        }

        tracing::info!("Received {bytes} bytes from {address:?}");
        match decode(&buf, address, pool).await {
            Err(e) => tracing::error!("{e:?}"),
            Ok(reply) => {
                let bytes = build_reply(&reply);
                match bytes {
                    Ok(bytes) => {
                        tracing::info!("Submitting {} bytes to network", bytes.len());
                        if let Err(e) = socket.write_all(&bytes).await {
                            tracing::error!("Write error: {e:?}");
                        }
                    }
                    Err(e) => {
                        tracing::error!("{e:?}");
                    }
                }
            }
        }
    }
}

#[tracing::instrument(skip(buf, pool))]
async fn decode(
    buf: &[u8],
    address: SocketAddr,
    pool: Pool<Postgres>,
) -> anyhow::Result<LicenseReply> {
    const U64SIZE: usize = std::mem::size_of::<u64>();
    let version_buf = &buf[0..2].try_into()?;
    let version = u16::from_be_bytes(*version_buf);
    let size_buf = &buf[2..2 + U64SIZE].try_into()?;
    let size = u64::from_be_bytes(*size_buf);
    tracing::info!(
        "Received a version {version} payload of serialized size {size} from {address:?}"
    );

    match version {
        1 => {
            let start = 2 + U64SIZE;
            let end = start + size as usize;
            let payload: LicenseRequest = lts_client::cbor::from_slice(&buf[start..end])?;
            let license = check_license(&payload, address, pool).await?;
            Ok(license)
        }
        _ => {
            tracing::error!("Unknown version of statistics: {version}, dumped {size} bytes");
            Err(anyhow::Error::msg("Version error"))
        }
    }
}

#[tracing::instrument(skip(request, pool))]
async fn check_license(
    request: &LicenseRequest,
    address: SocketAddr,
    pool: Pool<Postgres>,
) -> anyhow::Result<LicenseReply> {
    match request {
        LicenseRequest::LicenseCheck { key } => {
            tracing::info!("Checking license from {address:?}, key: {key}");
            if key == "test" {
                tracing::info!("License is valid");
                Ok(LicenseReply::Valid {
                    expiry: 0,                                // Temporary value
                    stats_host: "127.0.0.1:9127".to_string(), // Also temporary
                })
            } else {
                match pgdb::get_stats_host_for_key(pool, key).await {
                    Ok(host) => {
                        tracing::info!("License is valid");
                        return Ok(LicenseReply::Valid {
                            expiry: 0, // Temporary value
                            stats_host: host,
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Unable to get stats host for key: {e:?}");
                    }
                }

                tracing::info!("License is denied");
                Ok(LicenseReply::Denied)
            }
        }
        LicenseRequest::KeyExchange {
            node_id,
            node_name,
            license_key,
            public_key,
        } => {
            tracing::info!("Public key exchange requested by {node_id}");

            // Check if the node_id / license key combination exists
            // If it does, update it to the current last-seen and the new public key
            // If it doesn't, insert it
            let public_key = lts_client::cbor::to_vec(&public_key).unwrap();
            let result = pgdb::insert_or_update_node_public_key(
                pool,
                node_id,
                node_name,
                license_key,
                &public_key,
            )
            .await;
            if result.is_err() {
                tracing::warn!("Unable to insert or update node public key: {result:?}");
                return Err(anyhow::Error::msg(
                    "Unable to insert or update node public key",
                ));
            }

            let public_key = LIBREQOS_KEYPAIR.read().await.public_key.clone();
            Ok(LicenseReply::MyPublicKey { public_key })
        }
    }
}

fn build_reply(reply: &LicenseReply) -> anyhow::Result<Vec<u8>> {
    let mut result = Vec::new();
    let payload = lts_client::cbor::to_vec(reply);
    if let Err(e) = payload {
        tracing::warn!("Unable to serialize statistics. Not sending them.");
        tracing::warn!("{e:?}");
        return Err(anyhow::Error::msg("Unable to serialize"));
    }
    let payload = payload.unwrap();

    // Store the version as network order
    result.extend(1u16.to_be_bytes());
    // Store the payload size as network order
    result.extend((payload.len() as u64).to_be_bytes());
    // Store the payload itself
    result.extend(payload);

    Ok(result)
}
