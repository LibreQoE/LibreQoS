use crate::submissions::get_org_details;
use axum::extract::ws::{WebSocket, Message};
use chrono::{DateTime, FixedOffset, Utc};
use influxdb2::{models::Query, Client, FromDataPoint};
use pgdb::sqlx::{Pool, Postgres};
use serde::Serialize;

#[derive(Debug, FromDataPoint)]
pub struct BitsAndPackets {
    direction: String,
    host_id: String,
    min: f64,
    max: f64,
    avg: f64,
    time: DateTime<FixedOffset>,
}

impl Default for BitsAndPackets {
    fn default() -> Self {
        Self {
            direction: "".to_string(),
            host_id: "".to_string(),
            min: 0.0,
            max: 0.0,
            avg: 0.0,
            time: DateTime::<Utc>::MIN_UTC.into(),
        }
    }
}

#[derive(Debug, FromDataPoint)]
pub struct Rtt {
    host_id: String,
    min: f64,
    max: f64,
    avg: f64,
    time: DateTime<FixedOffset>,
}

impl Default for Rtt {
    fn default() -> Self {
        Self {
            host_id: "".to_string(),
            min: 0.0,
            max: 0.0,
            avg: 0.0,
            time: DateTime::<Utc>::MIN_UTC.into(),
        }
    }
}

#[derive(Serialize)]
struct Packets {
    value: f64,
    date: String,
    l: f64,
    u: f64,
}

#[derive(Serialize)]
struct PacketChart {
    msg: String,
    down: Vec<Packets>,
    up: Vec<Packets>,
}

#[derive(Serialize)]
struct RttChart {
    msg: String,
    data: Vec<Packets>,
    histo: Vec<u64>,
}


pub async fn packets(cnn: Pool<Postgres>, socket: &mut WebSocket, key: &str) {
    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        let qs = format!(
            "from(bucket: \"{}\")
        |> range(start: -5m)
        |> filter(fn: (r) => r[\"_measurement\"] == \"packets\")
        |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
        |> aggregateWindow(every: 10s, fn: mean, createEmpty: false)
        |> yield(name: \"last\")",
            org.influx_bucket, org.key
        );

        let query = Query::new(qs);
        let rows = client.query::<BitsAndPackets>(Some(query)).await;
        match rows {
            Err(e) => {
                tracing::error!("Error querying InfluxDB: {}", e);
            }
            Ok(rows) => {
                // Parse and send the data
                //println!("{rows:?}");

                let mut down = Vec::new();
                let mut up = Vec::new();

                // Fill download
                for row in rows
                    .iter()
                    .filter(|r| r.direction == "down")
                {
                    down.push(Packets {
                        value: row.avg,
                        date: row.time.format("%H:%M:%S").to_string(),
                        l: row.min,
                        u: row.max - row.min,
                    });
                }

                // Fill upload
                for row in rows
                    .iter()
                    .filter(|r| r.direction == "up")
                {
                    up.push(Packets {
                        value: row.avg,
                        date: row.time.format("%H:%M:%S").to_string(),
                        l: row.min,
                        u: row.max - row.min,
                    });
                }


                // Send it
                let chart = PacketChart { msg: "packetChart".to_string(), down, up };
                let json = serde_json::to_string(&chart).unwrap();
                socket.send(Message::Text(json)).await.unwrap();
            }
        }
    }
}

pub async fn bits(cnn: Pool<Postgres>, socket: &mut WebSocket, key: &str) {
    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        let qs = format!(
            "from(bucket: \"{}\")
        |> range(start: -5m)
        |> filter(fn: (r) => r[\"_measurement\"] == \"bits\")
        |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
        |> aggregateWindow(every: 10s, fn: mean, createEmpty: false)
        |> yield(name: \"last\")",
            org.influx_bucket, org.key
        );

        let query = Query::new(qs);
        let rows = client.query::<BitsAndPackets>(Some(query)).await;
        match rows {
            Err(e) => {
                tracing::error!("Error querying InfluxDB: {}", e);
            }
            Ok(rows) => {
                // Parse and send the data
                //println!("{rows:?}");

                let mut down = Vec::new();
                let mut up = Vec::new();

                // Fill download
                for row in rows
                    .iter()
                    .filter(|r| r.direction == "down")
                {
                    down.push(Packets {
                        value: row.avg,
                        date: row.time.format("%H:%M:%S").to_string(),
                        l: row.min,
                        u: row.max - row.min,
                    });
                }

                // Fill upload
                for row in rows
                    .iter()
                    .filter(|r| r.direction == "up")
                {
                    up.push(Packets {
                        value: row.avg,
                        date: row.time.format("%H:%M:%S").to_string(),
                        l: row.min,
                        u: row.max - row.min,
                    });
                }


                // Send it
                let chart = PacketChart { msg: "bitsChart".to_string(), down, up };
                let json = serde_json::to_string(&chart).unwrap();
                socket.send(Message::Text(json)).await.unwrap();
            }
        }
    }
}

pub async fn rtt(cnn: Pool<Postgres>, socket: &mut WebSocket, key: &str) {
    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        let qs = format!(
            "from(bucket: \"{}\")
        |> range(start: -5m)
        |> filter(fn: (r) => r[\"_measurement\"] == \"rtt\")
        |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
        |> aggregateWindow(every: 10s, fn: mean, createEmpty: false)
        |> yield(name: \"last\")",
            org.influx_bucket, org.key
        );

        let query = Query::new(qs);
        let rows = client.query::<Rtt>(Some(query)).await;
        match rows {
            Err(e) => {
                tracing::error!("Error querying InfluxDB: {}", e);
            }
            Ok(rows) => {
                // Parse and send the data
                //println!("{rows:?}");

                let mut data = Vec::new();
                let mut histo = vec![0; 20];

                for row in rows
                    .iter()
                {
                    data.push(Packets {
                        value: row.avg,
                        date: row.time.format("%H:%M:%S").to_string(),
                        l: row.min,
                        u: row.max - row.min,
                    });
                    let bucket = u64::min(19, (row.avg / 200.0) as u64);
                    histo[bucket as usize] += 1;
                }

                // Send it
                let chart = RttChart { msg: "rttChart".to_string(), data, histo };
                let json = serde_json::to_string(&chart).unwrap();
                socket.send(Message::Text(json)).await.unwrap();
            }
        }
    }
}