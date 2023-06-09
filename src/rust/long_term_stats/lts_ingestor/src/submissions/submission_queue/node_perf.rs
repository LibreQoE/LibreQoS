use futures::prelude::*;
use influxdb2::{models::DataPoint, Client};
use pgdb::OrganizationDetails;

pub async fn collect_node_perf(
    org: &OrganizationDetails,
    node_id: &str,
    timestamp: i64,
    cpu: &Option<Vec<u32>>,
    ram: &Option<u32>,
) -> anyhow::Result<()> {
    if let (Some(cpu), Some(ram)) = (cpu, ram) {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(&influx_url, &org.influx_org, &org.influx_token);
        let cpu_sum = cpu.iter().sum::<u32>();
        let cpu_avg = cpu_sum / cpu.len() as u32;
        let cpu_max = *cpu.iter().max().unwrap();
        let points = vec![DataPoint::builder("perf")
            .tag("host_id", node_id.to_string())
            .tag("organization_id", org.key.to_string())
            .timestamp(timestamp)
            .field("ram", *ram as i64)
            .field("cpu", cpu_avg as i64)
            .field("cpu_max", cpu_max as i64)
            .build()?];
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
