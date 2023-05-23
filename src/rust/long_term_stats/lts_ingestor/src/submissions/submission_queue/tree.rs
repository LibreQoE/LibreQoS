use futures::prelude::*;
use influxdb2::{models::DataPoint, Client};
use lts_client::transport_data::StatsTreeNode;
use pgdb::{
    sqlx::{Pool, Postgres},
    OrganizationDetails,
};
use tracing::{info, error};

const SQL: &str = "INSERT INTO site_tree (key, host_id, site_name, index, parent, site_type, max_up, max_down, current_up, current_down, current_rtt) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) ON CONFLICT (key, host_id, site_name) DO NOTHING";

pub async fn collect_tree(
    cnn: Pool<Postgres>,
    org: &OrganizationDetails,
    node_id: &str,
    timestamp: i64,
    totals: &Option<Vec<StatsTreeNode>>,
) -> anyhow::Result<()> {
    if let Some(tree) = totals {
        //println!("{tree:?}");
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(&influx_url, &org.influx_org, &org.influx_token);
        let mut points: Vec<DataPoint> = Vec::new();

        let mut trans = cnn.begin().await?;

        pgdb::sqlx::query("DELETE FROM site_tree WHERE key=$1 AND host_id=$2")
            .bind(org.key.to_string())
            .bind(node_id)
            .execute(&mut trans)
            .await?;

        for node in tree.iter() {
            points.push(
                DataPoint::builder("tree")
                    .tag("host_id", node_id.to_string())
                    .tag("organization_id", org.key.to_string())
                    .tag("node_name", node.name.to_string())
                    .tag("direction", "down".to_string())
                    .timestamp(timestamp)
                    .field("bits_min", node.current_throughput.min.0 as i64)
                    .field("bits_max", node.current_throughput.max.0 as i64)
                    .field("bits_avg", node.current_throughput.avg.0 as i64)
                    .build()?,
            );
            points.push(
                DataPoint::builder("tree")
                    .tag("host_id", node_id.to_string())
                    .tag("organization_id", org.key.to_string())
                    .tag("node_name", node.name.to_string())
                    .tag("direction", "up".to_string())
                    .timestamp(timestamp)
                    .field("bits_min", node.current_throughput.min.1 as i64)
                    .field("bits_max", node.current_throughput.max.1 as i64)
                    .field("bits_avg", node.current_throughput.avg.1 as i64)
                    .build()?,
            );
            points.push(
                DataPoint::builder("tree")
                    .tag("host_id", node_id.to_string())
                    .tag("organization_id", org.key.to_string())
                    .tag("node_name", node.name.to_string())
                    .timestamp(timestamp)
                    .field("rtt_min", node.rtt.min as i64 / 100)
                    .field("rtt_max", node.rtt.max as i64 / 100)
                    .field("rtt_avg", node.rtt.avg as i64 / 100)
                    .build()?,
            );

            let result = pgdb::sqlx::query(SQL)
                .bind(org.key.to_string())
                .bind(node_id)
                .bind(&node.name)
                .bind(node.index as i32)
                .bind(node.immediate_parent.unwrap_or(0) as i32)
                .bind(node.node_type.as_ref().unwrap_or(&String::new()).clone())
                .bind(node.max_throughput.1 as i64)
                .bind(node.max_throughput.0 as i64)
                .bind(node.current_throughput.max.1 as i64)
                .bind(node.current_throughput.max.0 as i64)
                .bind(node.rtt.avg as i64)
                .execute(&mut trans)
                .await;
            if let Err(e) = result {
                error!("Error inserting tree node: {}", e);
            }
        }

        let result = trans.commit().await;
        info!("Transaction committed");
        if let Err(e) = result {
            error!("Error committing transaction: {}", e);
        }

        client
            .write_with_precision(
                &org.influx_bucket,
                stream::iter(points),
                influxdb2::api::write::TimestampPrecision::Seconds,
            )
            .await?;
    }
    Ok(())
}

