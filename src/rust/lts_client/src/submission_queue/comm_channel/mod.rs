use std::time::Duration;
use lqos_config::EtcLqos;
use tokio::{sync::mpsc::Receiver, time::sleep, net::TcpStream, io::{AsyncWriteExt, AsyncReadExt}};
use crate::submission_queue::comm_channel::keys::store_server_public_key;
use self::encode::encode_submission_hello;
use super::queue::{send_queue, QueueError};
mod keys;
pub(crate) use keys::key_exchange;
mod encode;
pub(crate) use encode::encode_submission;

pub(crate) enum SenderChannelMessage {
    QueueReady,
    Quit,
}

pub(crate) async fn start_communication_channel(mut rx: Receiver<SenderChannelMessage>) {
//    let mut connected = false;
//    let mut stream: Option<TcpStream> = None;
    loop {
        match rx.try_recv() {
            Ok(SenderChannelMessage::QueueReady) => {
                log::info!("Trying to connect to stats.libreqos.io");
                let mut stream = connect_if_permitted().await;
                log::info!("Connection to stats.libreqos.io established");

                // If we're still not connected, skip - otherwise, send the
                // queued data
                if let Ok(tcpstream) = &mut stream {
                    // Send the data
                    let all_good = send_queue(tcpstream).await;
                    if all_good.is_err() {
                        log::error!("Stream fail during send. Will re-send");
                    }
                } else {
                    log::error!("Unable to submit data to stats.libreqos.io: {stream:?}");
                }
            }
            Ok(SenderChannelMessage::Quit) => {
                break;
            }
            _ => {}
        }

        sleep(Duration::from_secs(10)).await;
    }
}

async fn connect_if_permitted() -> Result<TcpStream, QueueError> {
    log::info!("Connecting to stats.libreqos.io");
    // Check that we have a local license key and are enabled
    let cfg = EtcLqos::load().map_err(|_| {
        log::error!("Unable to load config file.");
        QueueError::NoLocalLicenseKey
    })?;
    let node_id = cfg.node_id.ok_or_else(|| {
        log::warn!("No node ID configured.");
        QueueError::NoLocalLicenseKey
    })?;
    let node_name = cfg.node_name.unwrap_or(node_id.clone());
    let usage_cfg = cfg.long_term_stats.ok_or_else(|| {
        log::warn!("Long-term stats are not configured.");
        QueueError::NoLocalLicenseKey
    })?;
    if !usage_cfg.gather_stats {
        log::warn!("Gathering long-term stats is disabled.");
        return Err(QueueError::StatsDisabled);
    }
    let license_key = usage_cfg.license_key.ok_or_else(|| {
        log::warn!("No license key configured.");
        QueueError::NoLocalLicenseKey
    })?;
    
    // Connect
    let host = "stats.libreqos.io:9128";
    let mut stream = TcpStream::connect(&host).await
        .map_err(|e| {
            log::error!("Unable to connect to {host}: {e:?}");
            QueueError::SendFail
        })?;

    // Send Hello
    let bytes = encode_submission_hello(&license_key, &node_id, &node_name).await?;
    stream.write_all(&bytes).await
        .map_err(|e| {
            log::error!("Unable to write to {host}: {e:?}");
            QueueError::SendFail
        })?;

    // Receive Server Public Key or Denied
    let result = stream.read_u16().await
        .map_err(|e| {
            log::error!("Unable to read reply from {host}, {e:?}");
            QueueError::SendFail
        })?;
    match result {
        0 => {
            log::error!("License validation failure.");
            return Err(QueueError::SendFail);
        }
        1 => {
            // We received validation. Now to decode the public key.
            let key_size = stream.read_u64().await
                .map_err(|e| {
                    log::error!("Unable to read reply from {host}, {e:?}");
                    QueueError::SendFail
                })?;
            let mut key_buffer = vec![0u8; key_size as usize];
            stream.read_exact(&mut key_buffer).await
                .map_err(|e| {
                    log::error!("Unable to read reply from {host}, {e:?}");
                    QueueError::SendFail
                })?;
            let server_public_key = serde_cbor::from_slice(&key_buffer)
                .map_err(|e| {
                    log::error!("Unable to decode key from {host}, {e:?}");
                    QueueError::SendFail
                })?;
                store_server_public_key(&server_public_key).await;
            log::info!("Received server public key.");
        }
        _ => {
            log::error!("Unexpected reply from server.");
            return Err(QueueError::SendFail);
        }
    }

    // Proceed
    Ok(stream)
}