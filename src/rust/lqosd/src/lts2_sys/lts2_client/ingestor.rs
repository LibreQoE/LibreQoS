pub(crate) mod commands;
mod message_queue;

use crate::lts2_sys::lts2_client::ingestor::commands::IngestorCommand;
use crate::lts2_sys::lts2_client::ingestor::message_queue::MessageQueue;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use parking_lot::Mutex;
use std::time::Duration;
use timerfd::{SetTimeFlags, TimerFd, TimerState};
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
    let my_message_queue = message_queue.clone();
    std::thread::spawn(move || ticker_timer(my_message_queue));

    info!("Starting ingestor loop");
    while let Ok(msg) = rx.recv() {
        let mut message_queue_lock = message_queue.lock();
        message_queue_lock.ingest(msg);
    }
    warn!("Ingestor loop exited");
}

fn ticker_timer(message_queue: Arc<Mutex<MessageQueue>>) {
    let mut tfd = TimerFd::new().unwrap();
    assert_eq!(tfd.get_state(), TimerState::Disarmed);
    tfd.set_state(
        TimerState::Periodic {
            current: Duration::from_secs(60),
            interval: Duration::from_secs(60),
        },
        SetTimeFlags::Default,
    );

    loop {
        let missed_ticks = tfd.read();
        if missed_ticks > 1 {
            info!("Missed queue submission ticks: {}", missed_ticks - 1);
        }

        let mut session_data: MessageQueue = {
            let mut message_queue_lock = message_queue.lock();
            let data = message_queue_lock.clone();
            message_queue_lock.clear();
            data
        };

        if !session_data.is_empty() {
            let start = std::time::Instant::now();
            if let Err(e) = session_data.send() {
                info!("Failed to send queue: {e:?}");
            }
            info!("Queue send took: {:?}s", start.elapsed().as_secs_f32());
        }
    }
}
