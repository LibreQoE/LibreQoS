//! Provides a queue of submissions to be processed by the long-term storage.
//! This is a "fan in" pattern: multi-producer, single-consumer messages
//! send data into the queue, which is managed by a single consumer
//! thread. The consumer thread spawns tokio tasks to actually
//! perform the processing.

use crate::submissions::submission_queue::{
    devices::ingest_shaped_devices, host_totals::collect_host_totals, node_perf::collect_node_perf,
    organization_cache::get_org_details, tree::collect_tree, per_host::collect_per_host, uisp_devices::collect_uisp_devices,
};
use lts_client::transport_data::{LtsCommand, NodeIdAndLicense};
use pgdb::sqlx::{Pool, Postgres};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{info, error, warn};

const SUBMISSION_QUEUE_SIZE: usize = 100;
pub type SubmissionType = (NodeIdAndLicense, LtsCommand);

pub async fn submissions_queue(cnn: Pool<Postgres>) -> anyhow::Result<Sender<SubmissionType>> {
    // Create a channel to send data to the consumer thread
    let (tx, rx) = tokio::sync::mpsc::channel::<SubmissionType>(SUBMISSION_QUEUE_SIZE);
    tokio::spawn(run_queue(cnn, rx)); // Note that'we *moving* rx into the spawned task
    Ok(tx)
}

async fn run_queue(cnn: Pool<Postgres>, mut rx: Receiver<SubmissionType>) -> anyhow::Result<()> {
    while let Some(message) = rx.recv().await {
        info!("Received a message from the submission queue");
        let (node_id, command) = message;
        tokio::spawn(ingest_stats(cnn.clone(), node_id, command));
    }
    Ok(())
}

//#[tracing::instrument]
async fn ingest_stats(
    cnn: Pool<Postgres>,
    node_id: NodeIdAndLicense,
    command: LtsCommand,
) -> anyhow::Result<()> {
    info!("Ingesting stats for node {}", node_id.node_id);

    if let Some(org) = get_org_details(&cnn, &node_id.license_key).await {
        //println!("{:?}", command);
        match command {
            LtsCommand::Devices(devices) => {
                info!("Ingesting Shaped Devices");
                update_last_seen(cnn.clone(), &node_id).await;
                if let Err(e) = ingest_shaped_devices(cnn, &org, &node_id.node_id, &devices).await {
                    error!("Error ingesting shaped devices: {}", e);
                }
            }
            LtsCommand::Submit(stats) => {
                //println!("Submission: {:?}", submission);
                info!("Ingesting statistics dump");
                let ts = stats.timestamp as i64;
                let _ = tokio::join!(
                    update_last_seen(cnn.clone(), &node_id),
                    collect_host_totals(&org, &node_id.node_id, ts, &stats.totals),                    
                    collect_node_perf(
                        &org,
                        &node_id.node_id,
                        ts,
                        &stats.cpu_usage,
                        &stats.ram_percent
                    ),
                    collect_tree(cnn.clone(), &org, &node_id.node_id, ts, &stats.tree),
                    collect_per_host(&org, &node_id.node_id, ts, &stats.hosts),
                    collect_uisp_devices(cnn.clone(), &org, &stats.uisp_devices, ts),
                );
            }
        }
    } else {
        warn!(
            "Unable to find organization for license {}",
            node_id.license_key
        );
    }
    Ok(())
}

async fn update_last_seen(cnn: Pool<Postgres>, details: &NodeIdAndLicense) {
    let res = pgdb::new_stats_arrived(cnn, &details.license_key, &details.node_id).await;
    if res.is_err() {
        error!(
            "Unable to update last seen for node {}: {}",
            details.node_id,
            res.unwrap_err()
        );
    }
}
