//! Provides a centralized TC queue manager. This will eventually grow into a full state
//! tracking system (ala NLNet), for now it's deliberately quite simple.

mod tc_control;

use anyhow::Result;
use tracing::warn;

pub enum TcQueueCommand {
    DeleteRoot { interface: String, reply: oneshot::Sender<bool> },
    SetMqRoot { interface: String, reply: oneshot::Sender<bool> },
    SetParent { interface: String, queue_number: i32, rate_mbits: i32, ceil_mbits: i32, quantum: i32, sqm: String, reply: oneshot::Sender<bool> },
    SetDefault { interface: String, queue_number: i32, rate_mbits: i32, ceil_mbits: i32, sqm: String, reply: oneshot::Sender<bool> },
    AddHtbBranch { interface: String, parent: String, class_id: String, rate: i32, ceil: i32, quantum: i32, priority: i32, reply: oneshot::Sender<bool> },
}

pub fn start_queue_manager() -> Result<crossbeam_channel::Sender<TcQueueCommand>>
{
    let (tx, rx) = crossbeam_channel::bounded(65536);
    std::thread::Builder::new().name("lqos_queue_manager".to_string()).spawn(move || {
        let result = main_loop(rx);
        warn!("lqos_queue_manager thread terminated: {:?}", &result);
    })?;
    Ok(tx)
}

fn main_loop(
    rx: crossbeam_channel::Receiver<TcQueueCommand>,
) -> Result<()> {
    while let Ok(command) = rx.recv() {
        handle_command(command);
    }
    Ok(())
}

fn handle_command(command: TcQueueCommand) {
    match command {
        TcQueueCommand::DeleteRoot { interface, reply } => {
            let result = tc_control::delete_interface_root(&interface);
            if let Err(e) = result {
                warn!("Failed to delete interface root {:?}", e);
                let _ = reply.send(false);
            } else {
                let _ = reply.send(true);
            }
        }
        TcQueueCommand::SetMqRoot { interface, reply } => {
            let result = tc_control::replace_root_with_mq(&interface);
            if let Err(e) = result {
                warn!("Failed to replace interface root {:?}", e);
                let _ = reply.send(false);
            } else {
                let _ = reply.send(true);
            }
        }
        TcQueueCommand::SetParent { interface, queue_number, rate_mbits, ceil_mbits, quantum, sqm, reply } => {
            let result = tc_control::add_parent(&interface, queue_number, rate_mbits, ceil_mbits, quantum, &sqm);
            if let Err(e) = result {
                warn!("Failed to add parent {:?}", e);
                let _ = reply.send(false);
            } else {
                let _ = reply.send(true);
            }
        }
        TcQueueCommand::SetDefault { interface, queue_number, rate_mbits, ceil_mbits, sqm, reply } => {
            let result = tc_control::add_default(&interface, queue_number, rate_mbits, ceil_mbits, &sqm);
            if let Err(e) = result {
                warn!("Failed to add default {:?}", e);
                let _ = reply.send(false);
            } else {
                let _ = reply.send(true);
            }
        }
        TcQueueCommand::AddHtbBranch { interface, parent, class_id, rate, ceil, quantum, priority, reply } => {
            let result = tc_control::add_htb_branch(&interface, &parent, &class_id, rate, ceil, quantum, priority);
            if let Err(e) = result {
                warn!("Failed to add htb branch {:?}", e);
                let _ = reply.send(false);
            } else {
                let _ = reply.send(true);
            }
        }
    }
}