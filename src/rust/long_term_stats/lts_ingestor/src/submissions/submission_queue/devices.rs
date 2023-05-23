use lqos_config::ShapedDevice;
use pgdb::{OrganizationDetails, sqlx::{Pool, Postgres}};
use tracing::{warn, error};

pub async fn ingest_shaped_devices(
    cnn: Pool<Postgres>,
    org: &OrganizationDetails,
    node_id: &str,
    devices: &[ShapedDevice],
) -> anyhow::Result<()> {
    let mut trans = cnn.begin().await?;

    // Clear existing data from shaped devices
    pgdb::sqlx::query("DELETE FROM shaped_devices WHERE key=$1 AND node_id=$2")
        .bind(org.key.to_string())
        .bind(node_id)
        .execute(&mut trans)
        .await?;

    // Clear existing data from shaped devices IP lists
    pgdb::sqlx::query("DELETE FROM shaped_device_ip WHERE key=$1 AND node_id=$2")
        .bind(org.key.to_string())
        .bind(node_id)
        .execute(&mut trans)
        .await?;

    const SQL_INSERT: &str = "INSERT INTO shaped_devices
    (key, node_id, circuit_id, device_id, circuit_name, device_name, parent_node, mac, download_min_mbps, upload_min_mbps, download_max_mbps, upload_max_mbps, comment)
    VALUES
    ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)";

    const SQL_IP_INSERT: &str = "INSERT INTO public.shaped_device_ip
    (key, node_id, circuit_id, ip_range, subnet)
    VALUES
    ($1, $2, $3, $4, $5)
    ON CONFLICT (key, node_id, circuit_id, ip_range, subnet) DO NOTHING;";

    for device in devices.iter() {
        pgdb::sqlx::query(SQL_INSERT)
            .bind(org.key.to_string())
            .bind(node_id)
            .bind(device.circuit_id.clone())
            .bind(device.device_id.clone())
            .bind(device.circuit_name.clone())
            .bind(device.device_name.clone())
            .bind(device.parent_node.clone())
            .bind(device.mac.clone())
            .bind(device.download_min_mbps as i32)
            .bind(device.upload_min_mbps as i32)
            .bind(device.download_max_mbps as i32)
            .bind(device.upload_max_mbps as i32)
            .bind(device.comment.clone())
            .execute(&mut trans)
            .await?;

        for ip in device.ipv4.iter() {
            pgdb::sqlx::query(SQL_IP_INSERT)
                .bind(org.key.to_string())
                .bind(node_id)
                .bind(device.circuit_id.clone())
                .bind(ip.0.to_string())
                .bind(ip.1 as i32)
                .execute(&mut trans)
                .await?;
        }
        for ip in device.ipv6.iter() {
            pgdb::sqlx::query(SQL_IP_INSERT)
                .bind(org.key.to_string())
                .bind(node_id)
                .bind(device.circuit_id.clone())
                .bind(ip.0.to_string())
                .bind(ip.1 as i32)
                .execute(&mut trans)
                .await?;
        }
    }

    let result = trans.commit().await;
        warn!("Transaction committed");
        if let Err(e) = result {
            error!("Error committing transaction: {}", e);
        }

    Ok(())
}