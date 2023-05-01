use futures::prelude::*;
use influxdb2::{models::DataPoint, Client};
use lts_client::StatsTreeNode;
use pgdb::{
    sqlx::{Pool, Postgres},
    OrganizationDetails,
};

const SQL: &str = "INSERT INTO site_tree (key, host_id, site_name, index, parent, site_type) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (key, host_id, site_name) DO UPDATE SET index = $4, parent = $5, site_type = $6 WHERE site_tree.key=$1 AND site_tree.host_id=$2 AND site_tree.site_name=$3";

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

        for (i, node) in tree.iter().enumerate() {
            points.push(
                DataPoint::builder("tree")
                    .tag("host_id", node_id.to_string())
                    .tag("organization_id", org.key.to_string())
                    .tag("node_name", node.name.to_string())
                    .tag("direction", "down".to_string())
                    .timestamp(timestamp)
                    .field("bits", node.current_throughput.0 as i64)
                    .build()?,
            );
            points.push(
                DataPoint::builder("tree")
                    .tag("host_id", node_id.to_string())
                    .tag("organization_id", org.key.to_string())
                    .tag("node_name", node.name.to_string())
                    .tag("direction", "up".to_string())
                    .timestamp(timestamp)
                    .field("bits", node.current_throughput.1 as i64)
                    .build()?,
            );
            points.push(
                DataPoint::builder("tree")
                    .tag("host_id", node_id.to_string())
                    .tag("organization_id", org.key.to_string())
                    .tag("node_name", node.name.to_string())
                    .timestamp(timestamp)
                    .field("rtt_min", node.rtt.0 as i64 / 100)
                    .field("rtt_max", node.rtt.1 as i64 / 100)
                    .field("rtt_avg", node.rtt.2 as i64 / 100)
                    .build()?,
            );

            let result = pgdb::sqlx::query(SQL)
                .bind(org.key.to_string())
                .bind(node_id)
                .bind(&node.name)
                .bind(i as i32)
                .bind(node.immediate_parent.unwrap_or(0) as i32)
                .bind(node.node_type.as_ref().unwrap_or(&String::new()).clone())
                .execute(&mut trans)
                .await;
            if let Err(e) = result {
                log::error!("Error inserting tree node: {}", e);
            }
        }

        let result = trans.commit().await;
        log::info!("Transaction committed");
        if let Err(e) = result {
            log::error!("Error committing transaction: {}", e);
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

