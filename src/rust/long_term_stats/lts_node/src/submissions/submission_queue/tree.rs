use influxdb2::{Client, models::DataPoint};
use lqos_bus::long_term_stats::StatsTreeNode;
use pgdb::OrganizationDetails;
use futures::prelude::*;

pub async fn collect_tree(
    org: &OrganizationDetails,
    node_id: &str,
    timestamp: i64,
    totals: &Option<Vec<StatsTreeNode>>,
) -> anyhow::Result<()> {
    if let Some(tree) = totals {
        println!("{tree:?}");
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(&influx_url, &org.influx_org, &org.influx_token);
        let mut points: Vec<DataPoint> = Vec::new();

        for node in tree.iter() {
            points.push(DataPoint::builder("tree")
                .tag("host_id", node_id.to_string())
                .tag("organization_id", org.key.to_string())
                .tag("node_name", node.name.to_string())
                .tag("direction", "down".to_string())
                .timestamp(timestamp)
                .field("bits", node.current_throughput.0 as i64)
                .build()?);
            points.push(DataPoint::builder("tree")
                .tag("host_id", node_id.to_string())
                .tag("organization_id", org.key.to_string())
                .tag("node_name", node.name.to_string())
                .tag("direction", "up".to_string())
                .timestamp(timestamp)
                .field("bits", node.current_throughput.1 as i64)
                .build()?);
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