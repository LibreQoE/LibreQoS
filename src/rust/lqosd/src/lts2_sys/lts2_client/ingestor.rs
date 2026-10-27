pub(crate) mod commands;
mod message_queue;

use crate::lts2_sys::lts2_client::ingestor::message_queue::MessageQueue;
use crate::lts2_sys::{
    control_channel::ControlChannelCommand, lts2_client::ingestor::commands::IngestorCommand,
};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use tracing::{debug, info, warn};

/// `.2.2` Shape A: the blackboard split size (bytes), owned by the message
/// queue (it knows the session budget and the ≤2× field overhead).
pub(crate) fn blackboard_fragment_max_bytes() -> usize {
    message_queue::BLACKBOARD_FRAGMENT_MAX_BYTES
}

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
            debug!("Ingestor received batch complete command");
            let mut session_data: MessageQueue = {
                let mut message_queue_lock = message_queue.lock();
                let data = message_queue_lock.clone();
                message_queue_lock.clear();
                data
            };
            // `lqos-74t.2.1` instrument: the per-window EMITTED flow count,
            // logged even when the box cannot submit (no license) so the
            // export volume is measurable on any box. Removal gate 2026-12-31
            // (DRIVER-owned).
            let (pending_one_way, pending_two_way) = session_data.pending_flow_counts();
            if pending_one_way + pending_two_way > 0 {
                info!(target: "lqos_lts_volume",
                    "LTS2 ingest flush window: pending one_way_flows={pending_one_way} \
                     two_way_flows={pending_two_way} flows_total={}",
                    pending_one_way + pending_two_way
                );
            }
            if !session_data.is_empty() {
                let Ok(sessions) = session_data.build_sessions() else {
                    tracing::error!("Failed to build chunks");
                    continue;
                };
                // `lqos-74t.2.1` STEP 3: each session gets its own `serial`
                // (the server reassembles per `unique_id`); the guardrail keeps
                // every session within the byte budget.
                for chunks in sessions {
                    if chunks.is_empty() {
                        continue;
                    }
                    match submit.try_reserve() {
                        Ok(permit) => {
                            permit.send(ControlChannelCommand::SubmitChunks { serial, chunks });
                            serial += 1;
                        }
                        Err(error) => {
                            warn!(
                                "LTS2 session DROPPED (submit channel full): {error}; serial={serial}"
                            );
                        }
                    }
                }
            }
        } else {
            if !crate::lts2_sys::can_submit_long_term_stats() {
                continue;
            }
            let mut message_queue_lock = message_queue.lock();
            message_queue_lock.ingest(msg);
        }
    }
    warn!("Ingestor loop exited");
}
