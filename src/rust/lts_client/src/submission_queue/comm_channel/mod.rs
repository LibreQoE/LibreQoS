use std::time::Duration;
use tokio::{sync::mpsc::Receiver, time::sleep, net::TcpStream};
use super::{licensing::{get_license_status, LicenseState}, queue::send_queue};
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
                let mut stream = connect_if_permitted().await;

                // If we're still not connected, skip - otherwise, send the
                // queued data
                if let Some(tcpstream) = &mut stream {
                    // Send the data
                    let all_good = send_queue(tcpstream).await;
                    if all_good.is_err() {
                        log::error!("Stream fail during send. Will re-send");
                    }
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
            log::error!("Unable to exchange keys with license server.");
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