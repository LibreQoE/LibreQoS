use super::{
    comm_channel::{SenderChannelMessage, encode_submission},
    licensing::{LicenseState, get_license_status},
};
use crate::transport_data::{LtsCommand, StatsSubmission};
use lqos_config::ShapedDevice;
use once_cell::sync::Lazy;
use thiserror::Error;
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    sync::{Mutex, mpsc::Sender},
};
use tracing::{debug, error, info};

pub(crate) async fn enqueue_if_allowed(
    data: StatsSubmission,
    comm_tx: Sender<SenderChannelMessage>,
) {
    let license = get_license_status().await;
    match license {
        LicenseState::Unknown => {
            info!("Temporary error finding license status. Will retry.");
        }
        LicenseState::Denied => {
            info!("Your LTS1 license is invalid. Please contact support if you are using LTS1.");
        }
        LicenseState::Valid { .. } => {
            debug!("Sending data to the queue.");
            let queue_length = QUEUE.queue.lock().await.len();
            if queue_length > 50 {
                // If there are more than 50 items in the queue, remove the oldest one.
                // This prevents the queue from growing indefinitely if the server is unreachable.
                let mut lock = QUEUE.queue.lock().await;
                lock.remove(0);
                debug!("Queue length exceeded 50 items. Dropping oldest item.");
            }
            QUEUE.push(LtsCommand::Submit(Box::new(data))).await;
            if let Err(e) = comm_tx.send(SenderChannelMessage::QueueReady).await {
                error!("Unable to send queue ready message: {}", e);
            }
        }
    }
}

pub(crate) async fn enqueue_shaped_devices_if_allowed(
    devices: Vec<ShapedDevice>,
    comm_tx: Sender<SenderChannelMessage>,
) {
    let license = get_license_status().await;
    match license {
        LicenseState::Unknown => {
            info!("Temporary error finding license status. Will retry.");
        }
        LicenseState::Denied => {
            info!("Your license is invalid. Please contact support if you are still using LTS.");
        }
        LicenseState::Valid { .. } => {
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
        debug!("Sent submission: {} bytes.", submission_buffer.len());
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
