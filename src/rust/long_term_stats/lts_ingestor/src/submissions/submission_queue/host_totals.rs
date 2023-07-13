use futures::prelude::*;
use influxdb2::models::DataPoint;
use influxdb2::Client;
use lts_client::transport_data::StatsTotals;
use pgdb::OrganizationDetails;

pub async fn collect_host_totals(
    org: &OrganizationDetails,
    node_id: &str,
    timestamp: i64,
    totals: &Option<StatsTotals>,
) -> anyhow::Result<()> {
    if let Some(totals) = totals {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(&influx_url, &org.influx_org, &org.influx_token);
        let points = vec![
            DataPoint::builder("packets")
                .tag("host_id", node_id.to_string())
                .tag("organization_id", org.key.to_string())
                .tag("direction", "down".to_string())
                .timestamp(timestamp)
                .field("min", totals.packets.min.0 as i64)
                .field("max", totals.packets.max.0 as i64)
                .field("avg", totals.packets.avg.0 as i64)
                .build()?,
            DataPoint::builder("packets")
                .tag("host_id", node_id.to_string())
                .tag("organization_id", org.key.to_string())
                .tag("direction", "up".to_string())
                .timestamp(timestamp)
                .field("min", totals.packets.min.1 as i64)
                .field("max", totals.packets.max.1 as i64)
                .field("avg", totals.packets.avg.1 as i64)
                .build()?,
            DataPoint::builder("bits")
                .tag("host_id", node_id.to_string())
                .tag("organization_id", org.key.to_string())
                .tag("direction", "down".to_string())
                .timestamp(timestamp)
                .field("min", totals.bits.min.0 as i64)
                .field("max", totals.bits.max.0 as i64)
                .field("avg", totals.bits.avg.0 as i64)
                .build()?,
            DataPoint::builder("bits")
                .tag("host_id", node_id.to_string())
                .tag("organization_id", org.key.to_string())
                .tag("direction", "up".to_string())
                .timestamp(timestamp)
                .field("min", totals.bits.min.1 as i64)
                .field("max", totals.bits.max.1 as i64)
                .field("avg", totals.bits.avg.1 as i64)
                .build()?,
            DataPoint::builder("shaped_bits")
                .tag("host_id", node_id.to_string())
                .tag("organization_id", org.key.to_string())
                .tag("direction", "down".to_string())
                .timestamp(timestamp)
                .field("min", totals.shaped_bits.min.0 as i64)
                .field("max", totals.shaped_bits.max.0 as i64)
                .field("avg", totals.shaped_bits.avg.0 as i64)
                .build()?,
            DataPoint::builder("shaped_bits")
                .tag("host_id", node_id.to_string())
                .tag("organization_id", org.key.to_string())
                .tag("direction", "up".to_string())
                .timestamp(timestamp)
                .field("min", totals.shaped_bits.min.1 as i64)
                .field("max", totals.shaped_bits.max.1 as i64)
                .field("avg", totals.shaped_bits.avg.1 as i64)
                .build()?,
        ];

        //client.write(&org.influx_bucket, stream::iter(points)).await?;
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
