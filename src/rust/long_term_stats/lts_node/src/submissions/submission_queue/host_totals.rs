use lqos_bus::long_term_stats::StatsTotals;
use pgdb::OrganizationDetails;
use futures::prelude::*;
use influxdb2::models::DataPoint;
use influxdb2::Client;

pub async fn collect_host_totals(org: &OrganizationDetails, node_id: &str, timestamp: i64, totals: Option<StatsTotals>) -> anyhow::Result<()> {
    if let Some(totals) = totals {
        let client = Client::new(&org.influx_host, &org.influx_org, &org.influx_token);
        let points = vec![
            DataPoint::builder("packets_down")
                .timestamp(timestamp)
                .tag("node", node_id.to_string())
                .field("min", totals.packets.min.0 as i64)
                .field("max", totals.packets.max.0 as i64)
                .field("avg", totals.packets.avg.0 as i64)
                .build()?,
            DataPoint::builder("packets_up")
                .tag("node", node_id.to_string())
                .field("min", totals.packets.min.1 as i64)
                .field("max", totals.packets.max.1 as i64)
                .field("avg", totals.packets.avg.1 as i64)
                .build()?,
            DataPoint::builder("bits_down")
                .tag("node", node_id.to_string())
                .field("min", totals.bits.min.0 as i64)
                .field("max", totals.bits.max.0 as i64)
                .field("avg", totals.bits.avg.0 as i64)
                .build()?,
            DataPoint::builder("bits_up")
                .tag("node", node_id.to_string())
                .field("min", totals.bits.min.1 as i64)
                .field("max", totals.bits.max.1 as i64)
                .field("avg", totals.bits.avg.1 as i64)
                .build()?,
            DataPoint::builder("shaped_bits_down")
                .tag("node", node_id.to_string())
                .field("min", totals.shaped_bits.min.0 as i64)
                .field("max", totals.shaped_bits.max.0 as i64)
                .field("avg", totals.shaped_bits.avg.0 as i64)
                .build()?,
            DataPoint::builder("shaped_bits_up")
                .tag("node", node_id.to_string())
                .field("min", totals.shaped_bits.min.1 as i64)
                .field("max", totals.shaped_bits.max.1 as i64)
                .field("avg", totals.shaped_bits.avg.1 as i64)
                .build()?,
        ];

        client.write_with_precision(&org.influx_bucket, stream::iter(points), influxdb2::api::write::TimestampPrecision::Seconds).await?;
    }
    Ok(())
}