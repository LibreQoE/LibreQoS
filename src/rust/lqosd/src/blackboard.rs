use std::collections::HashMap;
use std::sync::OnceLock;
use crossbeam_channel::Sender;
use serde::Serialize;
use tracing::warn;
use lqos_bus::BlackboardSystem;

pub static BLACKBOARD_SENDER: OnceLock<Sender<BlackboardCommand>> = OnceLock::new();

pub enum BlackboardCommand {
    FinishSession,
    BlackboardData {
        subsystem: BlackboardSystem,
        key: String,
        value: String,
    },
    BlackboardBlob {
        tag: String,
        blob: Vec<u8>,
    },
}

#[derive(Serialize)]
struct Blackboard {
    system: HashMap<String, String>,
    sites: HashMap<String, String>,
    circuits: HashMap<String, String>,
    devices: HashMap<String, String>,
    blobs: HashMap<String, Vec<u8>>,
}

pub fn start_blackboard() {
    let (tx, rx) = crossbeam_channel::bounded(65535);
    std::thread::spawn(move || {
        let mut board = Blackboard {
            system: HashMap::new(),
            sites: HashMap::new(),
            circuits: HashMap::new(),
            devices: HashMap::new(),
            blobs: HashMap::new(),
        };

        loop {
            match rx.recv() {
                Ok(BlackboardCommand::FinishSession) => {
                    // If empty, do nothing
                    if board.circuits.is_empty() && board.sites.is_empty() && board.system.is_empty() {
                        continue;
                    }

                    // Serialize CBOR to a vec of u8
                    let cbor = match serde_cbor::to_vec(&board) {
                        Ok(j) => j,
                        Err(e) => {
                            warn!("Failed to serialize blackboard: {}", e);
                            continue;
                        }
                    };
                    lts2_sys::blackboard(&cbor);
                    board.circuits.clear();
                    board.sites.clear();
                    board.system.clear();
                    board.devices.clear();
                    board.blobs.clear();
                }
                Ok(BlackboardCommand::BlackboardData { subsystem, key, value }) => {
                    println!("Received data: {} = {}", key, value);
                    match subsystem {
                        BlackboardSystem::System => {
                            board.system.insert(key, value);
                        }
                        BlackboardSystem::Site => {
                            board.sites.insert(key, value);
                        }
                        BlackboardSystem::Circuit => {
                            board.circuits.insert(key, value);
                        }
                        BlackboardSystem::Device => {
                            board.devices.insert(key, value);
                        }
                    }
                }
                Ok(BlackboardCommand::BlackboardBlob { tag, blob }) => {
                    println!("Received blob: {}", tag);
                    board.blobs.insert(tag, blob);
                }
                Err(_) => break,
            }
        }
        warn!("Blackboard thread exiting");
    });
    let _ = BLACKBOARD_SENDER.set(tx);
}