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
}

#[derive(Serialize)]
struct Blackboard {
    system: HashMap<String, String>,
    sites: HashMap<String, String>,
    circuits: HashMap<String, String>,
    devices: HashMap<String, String>,
}

pub fn start_blackboard() {
    let (tx, rx) = crossbeam_channel::bounded(65535);
    std::thread::spawn(move || {
        let mut board = Blackboard {
            system: HashMap::new(),
            sites: HashMap::new(),
            circuits: HashMap::new(),
            devices: HashMap::new(),
        };

        loop {
            match rx.recv() {
                Ok(BlackboardCommand::FinishSession) => {
                    // If empty, do nothing
                    if board.circuits.is_empty() && board.sites.is_empty() && board.system.is_empty() {
                        continue;
                    }

                    // Serialize JSON to a vec of u8
                    let json = match serde_json::to_vec(&board) {
                        Ok(j) => j,
                        Err(e) => {
                            warn!("Failed to serialize blackboard: {}", e);
                            continue;
                        }
                    };
                    lts2_sys::blackboard(&json);
                    board.circuits.clear();
                    board.sites.clear();
                    board.system.clear();
                }
                Ok(BlackboardCommand::BlackboardData { subsystem, key, value }) => {
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
                Err(_) => break,
            }
        }
        warn!("Blackboard thread exiting");
    });
    let _ = BLACKBOARD_SENDER.set(tx);
}