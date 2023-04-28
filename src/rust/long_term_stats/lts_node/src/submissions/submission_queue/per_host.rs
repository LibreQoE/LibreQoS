use influxdb2::{Client, models::DataPoint};
use lqos_bus::long_term_stats::StatsHost;
use pgdb::{OrganizationDetails, sqlx::{Pool, Postgres}};
use futures::prelude::*;

pub async fn collect_per_host(
    cnn: Pool<Postgres>,
    org: &OrganizationDetails,
    node_id: &str,
    timestamp: i64,
    totals: &Option<Vec<StatsHost>>,
) -> anyhow::Result<()> {
    if let Some(hosts) = totals {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(&influx_url, &org.influx_org, &org.influx_token);
        let mut points: Vec<DataPoint> = Vec::new();
        
        let mut trans = cnn.begin().await?;
        log::info!("Received per-host stats, {} hosts", hosts.len());

        for host in hosts.iter().filter(|h| !h.device_id.is_empty()) {
            points.push(DataPoint::builder("host_bits")
                .tag("host_id", node_id.to_string())
                .tag("organization_id", org.key.to_string())
                .tag("direction", "down".to_string())
                .tag("circuit_id", host.circuit_id.to_string())
                .tag("device_id", host.device_id.to_string())
                .tag("ip", host.ip_address.to_string())
                .timestamp(timestamp)
                .field("min", host.bits.min.0 as i64)
                .field("max", host.bits.max.0 as i64)
                .field("avg", host.bits.avg.0 as i64)
                .build()?);
            points.push(DataPoint::builder("host_bits")
                .tag("host_id", node_id.to_string())
                .tag("organization_id", org.key.to_string())
                .tag("direction", "up".to_string())
                .tag("circuit_id", host.circuit_id.to_string())
                .tag("device_id", host.device_id.to_string())
                .tag("ip", host.ip_address.to_string())
                .timestamp(timestamp)
                .field("min", host.bits.min.1 as i64)
                .field("max", host.bits.max.1 as i64)
                .field("avg", host.bits.avg.1 as i64)
                .build()?);
            points.push(DataPoint::builder("rtt")
                .tag("host_id", node_id.to_string())
                .tag("organization_id", org.key.to_string())
                .tag("circuit_id", host.circuit_id.to_string())
                .tag("device_id", host.device_id.to_string())
                .tag("ip", host.ip_address.to_string())
                .timestamp(timestamp)
                .field("min", host.rtt.avg as f64 / 100.0)
                .field("max", host.rtt.max as f64 / 100.0)
                .field("avg", host.rtt.avg as f64 / 100.0)
                .build()?);

            const SQL: &str = "INSERT INTO devices (key, host_id, circuit_id, device_id, circuit_name, device_name, parent_node, mac_address, ip_address) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) ON CONFLICT (key, host_id, device_id) DO UPDATE SET circuit_id = $3, circuit_name = $5, device_name = $6, parent_node = $7, mac_address = $8, ip_address = $9;";

                log::info!("Submitting device");
                let result = pgdb::sqlx::query(SQL)
                    .bind(org.key.to_string())
                    .bind(node_id)
                    .bind(&host.circuit_id)
                    .bind(&host.device_id)
                    .bind(&host.circuit_name)
                    .bind(&host.device_name)
                    .bind(&host.parent_node)
                    .bind(&host.mac)
                    .bind(&host.ip_address)
                    .execute(&mut trans)
                    .await;
                if let Err(e) = result {
                    log::error!("Error inserting tree node: {}", e);
                    panic!();
                }
        }

        let result = trans.commit().await;
        log::warn!("Transaction committed");
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