mod caches;
mod commands;
mod queries;
mod remote_insight;
mod timed_cache;
mod ws_message;

pub use crate::node_manager::shaper_queries_actor::commands::ShaperQueryCommand;
use crate::node_manager::shaper_queries_actor::queries::shaper_queries;

pub async fn shaper_queries_actor() -> tokio::sync::mpsc::Sender<ShaperQueryCommand> {
    let (tx, rx) = tokio::sync::mpsc::channel(128);
    tokio::spawn(shaper_queries(rx));
    tx
}
