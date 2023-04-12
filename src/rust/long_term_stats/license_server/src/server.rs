<<<<<<< Updated upstream
use lqos_bus::long_term_stats::{LicenseCheck, LicenseReply};
use pgdb::sqlx::{Pool, Postgres};
=======
use lqos_bus::long_term_stats::{LicenseReply, LicenseRequest};
>>>>>>> Stashed changes
use std::net::SocketAddr;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    spawn,
};

pub async fn start() -> anyhow::Result<()> {
    let listener = TcpListener::bind(":::9126").await?;
    log::info!("Listening on :::9126");

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
            let mut buf = vec![0u8; 10240];
            if let Ok(bytes) = socket.read(&mut buf).await {
                log::info!("Received {bytes} bytes from {address:?}");
                match decode(&buf, address, pool).await {
                    Err(e) => log::error!("{e:?}"),
                    Ok(reply) => {
                        let bytes = build_reply(&reply);
                        match bytes {
                            Ok(bytes) => {
                                if let Err(e) = socket.write_all(&bytes).await {
                                    log::error!("Write error: {e:?}");
                                }
                            }
                            Err(e) => {
                                log::error!("{e:?}");
                            }
                        }
                    }
                }
            }
        });
    }
}

async fn decode(buf: &[u8], address: SocketAddr, pool: Pool<Postgres>) -> anyhow::Result<LicenseReply> {
    const U64SIZE: usize = std::mem::size_of::<u64>();
    let version_buf = &buf[0..2].try_into()?;
    let version = u16::from_be_bytes(*version_buf);
    let size_buf = &buf[2..2 + U64SIZE].try_into()?;
    let size = u64::from_be_bytes(*size_buf);
    log::info!("Received a version {version} payload of serialized size {size} from {address:?}");

    match version {
        1 => {
            let start = 2 + U64SIZE;
            let end = start + size as usize;
            let payload: LicenseCheck = serde_cbor::from_slice(&buf[start..end])?;
            let license = check_license(&payload, address, pool).await?;
            Ok(license)
        }
        _ => {
            log::error!("Unknown version of statistics: {version}, dumped {size} bytes");
            Err(anyhow::Error::msg("Version error"))
        }
    }
}

async fn check_license(
    request: &LicenseRequest,
    address: SocketAddr,
    pool: Pool<Postgres>,
) -> anyhow::Result<LicenseReply> {
    log::info!("Checking license from {address:?}, key: {}", request.key);
    if request.key == "test" {
        log::info!("License is valid");
        Ok(LicenseReply::Valid {
            expiry: 0, // Temporary value
            stats_host: "127.0.0.1:9127".to_string(), // Also temporary
        })
    } else {
        match pgdb::get_stats_host_for_key(pool, &request.key).await {
            Ok(host) => {
                log::info!("License is valid");
                return Ok(LicenseReply::Valid {
                    expiry: 0, // Temporary value
                    stats_host: host,
                });
            }
            Err(e) => {
                log::warn!("Unable to get stats host for key: {e:?}");
            }
        }        

        log::info!("License is denied");
        Ok(LicenseReply::Denied)
    }
}

fn build_reply(reply: &LicenseReply) -> anyhow::Result<Vec<u8>> {
    let mut result = Vec::new();
    let payload = serde_cbor::to_vec(reply);
    if let Err(e) = payload {
        log::warn!("Unable to serialize statistics. Not sending them.");
        log::warn!("{e:?}");
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
