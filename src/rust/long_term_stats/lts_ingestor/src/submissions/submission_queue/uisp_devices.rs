use futures::prelude::*;
use influxdb2::{models::DataPoint, Client};
use lts_client::transport_data::UispExtDevice;
use pgdb::{
    sqlx::{Pool, Postgres},
    OrganizationDetails,
};

pub async fn collect_uisp_devices(
    cnn: Pool<Postgres>,
    org: &OrganizationDetails,
    devices: &Option<Vec<UispExtDevice>>,
    ts: i64,
) {
    let (sql, influx) = tokio::join!(uisp_sql(cnn, org, devices), uisp_influx(org, devices, ts),);

    if let Err(e) = sql {
        tracing::error!("Error writing uisp sql: {:?}", e);
    }
    if let Err(e) = influx {
        tracing::error!("Error writing uisp influx: {:?}", e);
    }
}

async fn uisp_sql(
    cnn: Pool<Postgres>,
    org: &OrganizationDetails,
    devices: &Option<Vec<UispExtDevice>>,
) -> anyhow::Result<()> {
    if let Some(devices) = devices {
        let mut trans = cnn.begin().await.unwrap();

        // Handle the SQL portion (things that don't need to be graphed, just displayed)

        pgdb::sqlx::query("DELETE FROM uisp_devices_ext WHERE key=$1")
            .bind(org.key.to_string())
            .execute(&mut trans)
            .await?;

        pgdb::sqlx::query("DELETE FROM uisp_devices_interfaces WHERE key=$1")
            .bind(org.key.to_string())
            .execute(&mut trans)
            .await?;

        for device in devices.iter() {
            pgdb::sqlx::query("INSERT INTO uisp_devices_ext (key, device_id, name, model, firmware, status, mode) VALUES ($1, $2, $3, $4, $5, $6, $7)")
                .bind(org.key.to_string())
                .bind(&device.device_id)
                .bind(&device.name)
                .bind(&device.model)
                .bind(&device.firmware)
                .bind(&device.status)
                .bind(&device.mode)
                .execute(&mut trans)
                .await?;

            for interface in device.interfaces.iter() {
                let mut ip_list = String::new();
                for ip in interface.ip.iter() {
                    ip_list.push_str(&format!("{} ", ip));
                }
                pgdb::sqlx::query("INSERT INTO uisp_devices_interfaces (key, device_id, name, mac, status, speed, ip_list) VALUES ($1, $2, $3, $4, $5, $6, $7)")
                    .bind(org.key.to_string())
                    .bind(&device.device_id)
                    .bind(&interface.name)
                    .bind(&interface.mac)
                    .bind(&interface.status)
                    .bind(&interface.speed)
                    .bind(ip_list)
                    .execute(&mut trans)
                    .await?;
            }
        }

        trans.commit().await?;
    }
    Ok(())
}

async fn uisp_influx(
    org: &OrganizationDetails,
    devices: &Option<Vec<UispExtDevice>>,
    timestamp: i64,
) -> anyhow::Result<()> {
    if let Some(devices) = devices {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(&influx_url, &org.influx_org, &org.influx_token);
        let mut points: Vec<DataPoint> = Vec::new();

        for device in devices.iter() {
            points.push(
                DataPoint::builder("device_ext")
                    .tag("device_id", &device.device_id)
                    .tag("organization_id", org.key.to_string())
                    .tag("direction", "down".to_string())
                    .timestamp(timestamp)
                    .field("rx_signal", device.rx_signal as i64)
                    .field("noise_floor", device.noise_floor as i64)
                    .field("dl_capacity", device.downlink_capacity_mbps as i64)
                    .field("ul_capacity", device.uplink_capacity_mbps as i64)
                    .build()?,
            );
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
