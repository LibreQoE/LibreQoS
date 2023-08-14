use std::time::Duration;
use tokio::{sync::mpsc::Receiver, time::sleep, net::TcpStream};
use self::keys::key_exchange;
use super::{licensing::{get_license_status, LicenseState}, queue::send_queue};
mod keys;
mod encode;
pub(crate) use encode::encode_submission;

pub(crate) enum SenderChannelMessage {
    QueueReady,
    Quit,
}

pub(crate) async fn start_communication_channel(mut rx: Receiver<SenderChannelMessage>) {
    let mut connected = false;
    let mut stream: Option<TcpStream> = None;
    loop {
        match rx.try_recv() {
            Ok(SenderChannelMessage::QueueReady) => {
                // If not connected, see if we are allowed to connect and get a target
                if !connected || stream.is_none() {
                    log::info!("Establishing LTS TCP channel.");
                    stream = connect_if_permitted().await;
                    if stream.is_some() {
                        connected = true;
                    }
                }

                // If we're still not connected, skip - otherwise, send the
                // queued data
                if let Some(tcpstream) = &mut stream {
                    if connected && tcpstream.writable().await.is_ok() {
                        // Send the data
                        let all_good = send_queue(tcpstream).await;
                        if all_good.is_err() {
                            log::error!("Stream fail during send. Will re-send");
                            connected = false;
                            stream = None;
                        }
                    } else {
                        stream = None;
                        connected = false;
                    }
                } else {
                    connected = false;
                    stream = None;
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

async fn connect_if_permitted() -> Option<TcpStream> {
    let license = get_license_status().await;
    if let LicenseState::Valid { stats_host, .. } = license {
        if !key_exchange().await {
            return None;
        }

        let host = format!("{stats_host}:9128");
        let stream = TcpStream::connect(&host).await;
        match stream {
            Err(e) => {
                log::error!("Unable to connect to {host}: {e}");
                return None;
            }
            Ok(stream) => {
                if stream.writable().await.is_err() {
                    log::error!("Unable to write to {host}");
                    return None;
                }
                return Some(stream);
            }
        }
    }
    None
}