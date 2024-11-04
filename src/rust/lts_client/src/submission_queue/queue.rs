use lqos_config::ShapedDevice;
use once_cell::sync::Lazy;
use thiserror::Error;
use tokio::{sync::{Mutex, mpsc::Sender}, net::TcpStream, io::AsyncWriteExt};
use tracing::{error, info};
use crate::transport_data::{StatsSubmission, LtsCommand};
use super::{licensing::{LicenseState, get_license_status}, comm_channel::{SenderChannelMessage, encode_submission}};

pub(crate) async fn enqueue_if_allowed(data: StatsSubmission, comm_tx: Sender<SenderChannelMessage>) {
    let license = get_license_status().await;
    match license {
        LicenseState::Unknown => {
            info!("Temporary error finding license status. Will retry.");
        }
        LicenseState::Denied => {
            error!("Your license is invalid. Please contact support.");
        }
        LicenseState::Valid{ .. } => {
            info!("Sending data to the queue.");
            QUEUE.push(LtsCommand::Submit(Box::new(data))).await;
            if let Err(e) = comm_tx.send(SenderChannelMessage::QueueReady).await {
                error!("Unable to send queue ready message: {}", e);
            }
        }
    }
}

pub(crate) async fn enqueue_shaped_devices_if_allowed(devices: Vec<ShapedDevice>, comm_tx: Sender<SenderChannelMessage>) {
    let license = get_license_status().await;
    match license {
        LicenseState::Unknown => {
            info!("Temporary error finding license status. Will retry.");
        }
        LicenseState::Denied => {
            error!("Your license is invalid. Please contact support.");
        }
        LicenseState::Valid{ .. } => {
            QUEUE.push(LtsCommand::Devices(devices)).await;
            let _ = comm_tx.send(SenderChannelMessage::QueueReady).await;
        }
    }
}

static QUEUE: Lazy<Queue> = Lazy::new(Queue::new);

pub(crate) struct QueueSubmission {
    pub(crate) attempts: u8,
    pub(crate) body: LtsCommand,
    pub(crate) sent: bool,
}

pub(crate) struct Queue {
    queue: Mutex<Vec<QueueSubmission>>,
}

impl Queue {
    fn new() -> Self {
        Self {
            queue: Mutex::new(Vec::new()),
        }
    }

    pub async fn push(&self, data: LtsCommand) {
        {
            let mut lock = self.queue.lock().await;
            lock.push(QueueSubmission {
                attempts: 0,
                sent: false,
                body: data,
            });
        }
    }
}

pub(crate) async fn send_queue(stream: &mut TcpStream) -> Result<(), QueueError> {
    let mut lock = QUEUE.queue.lock().await;
    for message in lock.iter_mut() {
        message.attempts += 1;
        let submission_buffer = encode_submission(&message.body).await?;
        let ret = stream.write_all(&submission_buffer).await;
        info!("Sent submission: {} bytes.", submission_buffer.len());
        if ret.is_err() {
            error!("Unable to write to TCP stream.");
            error!("{:?}", ret);
            message.sent = false;
            match crate::submission_queue::comm_channel::key_exchange().await {
                true => {
                    info!("Successfully exchanged license keys.");
                }
                false => {
                    error!("Unable to talk to the licensing system to fix keys.");
                }
            }
            return Err(QueueError::SendFail);
        } else {
            message.sent = true;
        }
    }

    lock.clear();
    Ok(())
}

#[derive(Error, Debug)]
pub(crate) enum QueueError {
    #[error("No local license key")]
    NoLocalLicenseKey,
    #[error("Stats are disabled")]
    StatsDisabled,
    #[error("Unable to send")]
    SendFail,
}