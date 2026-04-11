use crate::state;
use crate::{DaemonHooks, load_network_json, load_shaped_devices};
use anyhow::{Result, anyhow};
use crossbeam_channel::{Receiver, Sender};
use once_cell::sync::OnceCell;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tracing::{debug, error, warn};

const RELOAD_RETRY_DELAY_MS: u64 = 500;
const RELOAD_ATTEMPTS: usize = 2;

static ACTOR_SENDER: OnceCell<Sender<NetworkDevicesCommand>> = OnceCell::new();

pub(crate) fn start_actor(hooks: Option<Arc<dyn DaemonHooks>>) -> Result<()> {
    if ACTOR_SENDER.get().is_some() {
        return Ok(());
    }

    let (tx, rx) = crossbeam_channel::bounded::<NetworkDevicesCommand>(64);
    let _ = ACTOR_SENDER.set(tx);

    std::thread::Builder::new()
        .name("lqos_network_devices".to_string())
        .spawn(move || actor_loop(rx, hooks))?;

    Ok(())
}

fn sender() -> Result<Sender<NetworkDevicesCommand>> {
    ACTOR_SENDER
        .get()
        .cloned()
        .ok_or_else(|| anyhow!("lqos_network_devices runtime actor is not running"))
}

pub(crate) fn request_reload_shaped_devices(reason: &str) -> Result<()> {
    let (reply_tx, reply_rx) = oneshot::channel();
    sender()?.send(NetworkDevicesCommand::ReloadShapedDevices {
        reason: reason.to_string(),
        reply: reply_tx,
    })?;
    reply_rx
        .blocking_recv()
        .map_err(|_| anyhow!("ShapedDevices reload reply channel closed"))?
}

pub(crate) fn request_reload_network_json(reason: &str) -> Result<()> {
    let (reply_tx, reply_rx) = oneshot::channel();
    sender()?.send(NetworkDevicesCommand::ReloadNetworkJson {
        reason: reason.to_string(),
        reply: reply_tx,
    })?;
    reply_rx
        .blocking_recv()
        .map_err(|_| anyhow!("NetworkJson reload reply channel closed"))?
}

pub(crate) fn apply_shaped_devices_snapshot(
    reason: &str,
    shaped: lqos_config::ConfigShapedDevices,
) -> Result<()> {
    let (reply_tx, reply_rx) = oneshot::channel();
    sender()?.send(NetworkDevicesCommand::ApplyShapedDevicesSnapshot {
        reason: reason.to_string(),
        shaped: Box::new(shaped),
        reply: reply_tx,
    })?;
    reply_rx
        .blocking_recv()
        .map_err(|_| anyhow!("Apply shaped devices reply channel closed"))?
}

enum NetworkDevicesCommand {
    ReloadShapedDevices {
        reason: String,
        reply: oneshot::Sender<Result<()>>,
    },
    ReloadNetworkJson {
        reason: String,
        reply: oneshot::Sender<Result<()>>,
    },
    ApplyShapedDevicesSnapshot {
        reason: String,
        shaped: Box<lqos_config::ConfigShapedDevices>,
        reply: oneshot::Sender<Result<()>>,
    },
}

fn actor_loop(rx: Receiver<NetworkDevicesCommand>, hooks: Option<Arc<dyn DaemonHooks>>) {
    debug!("lqos_network_devices actor starting");

    if let Err(err) = reload_shaped_devices_inner("startup", hooks.as_deref()) {
        warn!("Initial shaped-devices load failed: {err}");
    }
    if let Err(err) = reload_network_json_inner("startup", hooks.as_deref()) {
        warn!("Initial network-json load failed: {err}");
    }

    while let Ok(command) = rx.recv() {
        match command {
            NetworkDevicesCommand::ReloadShapedDevices { reason, reply } => {
                let result = reload_shaped_devices_inner(&reason, hooks.as_deref());
                let _ = reply.send(result);
            }
            NetworkDevicesCommand::ReloadNetworkJson { reason, reply } => {
                let result = reload_network_json_inner(&reason, hooks.as_deref());
                let _ = reply.send(result);
            }
            NetworkDevicesCommand::ApplyShapedDevicesSnapshot {
                reason,
                shaped,
                reply,
            } => {
                debug!("Publishing shaped-devices snapshot reason={reason}");
                state::publish_shaped_devices(*shaped);
                if let Some(hooks) = &hooks {
                    hooks.on_shaped_devices_updated();
                }
                let _ = reply.send(Ok(()));
            }
        }
    }

    error!("lqos_network_devices actor stopped");
}

fn reload_shaped_devices_inner(reason: &str, hooks: Option<&dyn DaemonHooks>) -> Result<()> {
    for attempt in 1..=RELOAD_ATTEMPTS {
        match load_shaped_devices() {
            Ok(shaped) => {
                debug!("Loaded shaped devices reason={reason}");
                state::publish_shaped_devices(shaped);
                if let Some(hooks) = hooks {
                    hooks.on_shaped_devices_updated();
                }
                return Ok(());
            }
            Err(err) if attempt < RELOAD_ATTEMPTS => {
                warn!(
                    "ShapedDevices reload reason={reason} attempt {attempt}/{} failed: {err}. Retrying after {} ms.",
                    RELOAD_ATTEMPTS, RELOAD_RETRY_DELAY_MS
                );
                std::thread::sleep(Duration::from_millis(RELOAD_RETRY_DELAY_MS));
            }
            Err(err) => {
                warn!(
                    "ShapedDevices reload reason={reason} failed after {} attempts: {err}. Keeping last-known-good snapshot with {} devices.",
                    RELOAD_ATTEMPTS,
                    state::shaped_devices_snapshot().devices.len()
                );
                return Err(err);
            }
        }
    }
    unreachable!("reload loop must return");
}

fn reload_network_json_inner(reason: &str, hooks: Option<&dyn DaemonHooks>) -> Result<()> {
    for attempt in 1..=RELOAD_ATTEMPTS {
        match load_network_json() {
            Ok(net_json) => {
                debug!("Loaded network json reason={reason}");
                state::publish_network_json(net_json);
                if let Some(hooks) = hooks {
                    hooks.on_network_json_updated();
                }
                return Ok(());
            }
            Err(err) if attempt < RELOAD_ATTEMPTS => {
                warn!(
                    "NetworkJson reload reason={reason} attempt {attempt}/{} failed: {err}. Retrying after {} ms.",
                    RELOAD_ATTEMPTS, RELOAD_RETRY_DELAY_MS
                );
                std::thread::sleep(Duration::from_millis(RELOAD_RETRY_DELAY_MS));
            }
            Err(err) => {
                warn!(
                    "NetworkJson reload reason={reason} failed after {} attempts: {err}. Keeping last-known-good snapshot.",
                    RELOAD_ATTEMPTS
                );
                return Err(err);
            }
        }
    }
    unreachable!("reload loop must return");
}
