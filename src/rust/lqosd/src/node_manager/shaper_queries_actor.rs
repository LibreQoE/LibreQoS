mod caches;
mod commands;
mod queries;
mod timed_cache;

use crate::lts2_sys::control_channel::ControlChannelCommand;
pub use crate::node_manager::shaper_queries_actor::commands::ShaperQueryCommand;
use crate::node_manager::shaper_queries_actor::queries::shaper_queries;

pub async fn shaper_queries_actor(
    control_tx: tokio::sync::mpsc::Sender<ControlChannelCommand>,
) -> tokio::sync::mpsc::Sender<ShaperQueryCommand> {
    let (tx, rx) = tokio::sync::mpsc::channel(128);
    tokio::spawn(shaper_queries(rx, control_tx));
    tx
}
