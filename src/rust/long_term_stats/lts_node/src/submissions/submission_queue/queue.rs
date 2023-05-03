//! Provides a queue of submissions to be processed by the long-term storage.
//! This is a "fan in" pattern: multi-producer, single-consumer messages
//! send data into the queue, which is managed by a single consumer
//! thread. The consumer thread spawns tokio tasks to actually
//! perform the processing.

use crate::submissions::submission_queue::{
    organization_cache::get_org_details,
};
use lts_client::transport_data::{NodeIdAndLicense, LtsCommand};
use pgdb::sqlx::{Pool, Postgres};
use tokio::{
    sync::mpsc::{Receiver, Sender},
};

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
        log::info!("Received a message from the submission queue");
        let (node_id, command) = message;
        tokio::spawn(ingest_stats(cnn.clone(), node_id, command));
    }
    Ok(())
}

async fn ingest_stats(
    cnn: Pool<Postgres>,
    node_id: NodeIdAndLicense,
    command: LtsCommand,
) -> anyhow::Result<()> {
    log::info!("Ingesting stats for node {}", node_id.node_id);

    if let Some(org) = get_org_details(cnn.clone(), &node_id.license_key).await {
        //println!("{:?}", command);
        match command {
            LtsCommand::Devices(devices) => {
                println!("Devices: {:?}", devices);
            },
            LtsCommand::Submit(submission) => {
                println!("Submission: {:?}", submission);
            },
        }
/*         let ts = stats.timestamp as i64;
        // TODO: Error handling
        let _ = join!(
            update_last_seen(cnn.clone(), &node_id),
            collect_host_totals(&org, &node_id.node_id, ts, &stats.totals),
            //collect_per_host(cnn.clone(), &org, &node_id.node_id, ts, &stats.hosts),
            //collect_tree(cnn.clone(), &org, &node_id.node_id, ts, &stats.tree),
            collect_node_perf(
                &org,
                &node_id.node_id,
                ts,
                &stats.cpu_usage,
                stats.ram_percent
            ),
        );*/
    } else {
        log::warn!(
            "Unable to find organization for license {}",
            node_id.license_key
        );
    }
    Ok(())
}

async fn update_last_seen(cnn: Pool<Postgres>, details: &NodeIdAndLicense) {
    let res = pgdb::new_stats_arrived(cnn, &details.license_key, &details.node_id).await;
    if res.is_err() {
        log::error!(
            "Unable to update last seen for node {}: {}",
            details.node_id,
            res.unwrap_err()
        );
    }
}
