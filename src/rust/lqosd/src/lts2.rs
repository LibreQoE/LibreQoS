//! Support for Long-Term Statistics, protocol version 2

mod lts_status;
mod lts_client;

use anyhow::Result;
use tokio::spawn;
use lqos_config::load_config;
pub use lts_status::{get_lts_status, get_lts_trial_days_remaining};

pub async fn start_lts2() -> Result<()> {
    log::info!("Staring Long-Term Stats 2 Support");

    spawn(lts_status::poll_lts_status());

    Ok(())
}