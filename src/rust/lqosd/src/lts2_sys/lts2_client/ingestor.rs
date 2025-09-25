pub(crate) mod commands;
mod message_queue;

use crate::lts2_sys::{control_channel::ControlChannelCommand, lts2_client::ingestor::commands::IngestorCommand};
use crate::lts2_sys::lts2_client::ingestor::message_queue::MessageQueue;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use parking_lot::Mutex;
use tracing::{info, warn};

pub fn start_ingestor() -> Sender<IngestorCommand> {
    println!("Starting ingestor");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || ingestor_loop(rx));
    println!("Ingestor started");
    tx
}

fn ingestor_loop(rx: std::sync::mpsc::Receiver<IngestorCommand>) {
    let message_queue = Arc::new(Mutex::new(MessageQueue::new()));
    //let my_message_queue = message_queue.clone();
    //std::thread::spawn(move || ticker_timer(my_message_queue));

    info!("Starting ingestor loop");
    let mut serial = 0;
    while let Ok(msg) = rx.recv() {
        if let IngestorCommand::IngestBatchComplete { submit } = msg {
            info!("Ingestor received batch complete command");
            let mut session_data: MessageQueue = {
                let mut message_queue_lock = message_queue.lock();
                let data = message_queue_lock.clone();
                message_queue_lock.clear();
                data
            };
            if !session_data.is_empty() {
                let Ok(chunks) = session_data.build_chunks() else {
                    tracing::error!("Failed to build chunks");
                    continue;
                };
                // If we have chunks, submit them
                if let Ok(permit) = submit.try_reserve() {
                    permit.send(ControlChannelCommand::SubmitChunks { serial, chunks });
                }
                serial += 1;
            }

        } else {
            let mut message_queue_lock = message_queue.lock();
            message_queue_lock.ingest(msg);
        }
    }
    warn!("Ingestor loop exited");
}
