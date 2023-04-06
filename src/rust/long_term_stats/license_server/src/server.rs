use std::net::SocketAddr;
use lqos_bus::long_term_stats::{LicenseCheck, LicenseReply};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener, spawn};

pub async fn start() -> anyhow::Result<()> {
  let listener = TcpListener::bind(":::9126").await?;
  log::info!("Listening on :::9126");

  loop {
    let (mut socket, address) = listener.accept().await?;
    log::info!("Connection from {address:?}");
    spawn(async move {
      let mut buf = Vec::with_capacity(10240);
      if let Ok(bytes) = socket.read_to_end(&mut buf).await {
        log::info!("Received {bytes} bytes from {address:?}");
        match decode(&buf, address).await {
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

async fn decode(buf: &[u8], address: SocketAddr) -> anyhow::Result<LicenseReply> {
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
      let payload: Result<LicenseCheck, _> =
        serde_cbor::from_slice(&buf[start..end]);
      match payload {
        Ok(payload) => check_license(&payload, address).await,
        Err(e) => {
          log::error!(
            "Unable to deserialize request sent from {address:?}"
          );
          log::error!("{e:?}");
          Err(anyhow::Error::msg("Deserialize error"))
        }
      }
    }
    _ => {
      log::error!(
        "Unknown version of statistics: {version}, dumped {size} bytes"
      );
      Err(anyhow::Error::msg("Version error"))
    }
  }
}

async fn check_license(request: &LicenseCheck, address: SocketAddr) -> anyhow::Result<LicenseReply> {
    Ok(LicenseReply::Denied)
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
    result.extend( 1u16.to_be_bytes() );
    // Store the payload size as network order
    result.extend( (payload.len() as u64).to_be_bytes() );
    // Store the payload itself
    result.extend(payload);

    Ok(result)
}