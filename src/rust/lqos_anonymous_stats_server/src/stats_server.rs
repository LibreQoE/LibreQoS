use lqos_bus::anonymous::AnonymousUsageV1;
use std::net::SocketAddr;
use tokio::{io::AsyncReadExt, net::TcpListener, spawn};

pub async fn gather_stats() -> anyhow::Result<()> {
  let listener = TcpListener::bind(":::9125").await?;
  log::info!("Listening on :::9125");

  loop {
    let (mut socket, address) = listener.accept().await?;
    log::info!("Connection from {address:?}");
    spawn(async move {
      let mut buf = vec![0; 10240];
      if let Ok(n) = socket.read(&mut buf).await {
        log::info!("Received {n} bytes from {address:?}");
        if let Err(e) = decode(&buf, address).await {
          log::error!("Decode error from {address:?}");
          log::error!("{e:?}");
        }
      }
    });
  }
}

async fn decode(buf: &[u8], address: SocketAddr) -> anyhow::Result<()> {
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
      let payload: Result<AnonymousUsageV1, _> =
        serde_cbor::from_slice(&buf[start..end]);
      match payload {
        Ok(payload) => store_stats_v1(&payload, address).await,
        Err(e) => {
          log::error!(
            "Unable to deserialize statistics sent from {address:?}"
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

async fn store_stats_v1(
  payload: &AnonymousUsageV1,
  address: SocketAddr,
) -> anyhow::Result<()> {
  println!("{payload:?} {address:?}");
  Ok(())
}
