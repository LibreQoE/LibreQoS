use chrono::{DateTime, FixedOffset, Utc};
use influxdb2::FromDataPoint;

#[derive(Debug, FromDataPoint)]
pub struct ThroughputRow {
    pub direction: String,
    pub host_id: String,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub time: DateTime<FixedOffset>,
}

impl Default for ThroughputRow {
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
pub struct ThroughputRowBySite {
    pub direction: String,
    pub host_id: String,
    pub bits_min: f64,
    pub bits_max: f64,
    pub bits_avg: f64,
    pub time: DateTime<FixedOffset>,
}

impl Default for ThroughputRowBySite {
    fn default() -> Self {
        Self {
            direction: "".to_string(),
            host_id: "".to_string(),
            bits_min: 0.0,
            bits_max: 0.0,
            bits_avg: 0.0,
            time: DateTime::<Utc>::MIN_UTC.into(),
        }
    }
}

#[derive(Debug, FromDataPoint)]
pub struct ThroughputRowByCircuit {
    pub direction: String,
    pub ip: String,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub time: DateTime<FixedOffset>,
}

impl Default for ThroughputRowByCircuit {
    fn default() -> Self {
        Self {
            direction: "".to_string(),
            ip: "".to_string(),
            min: 0.0,
            max: 0.0,
            avg: 0.0,
            time: DateTime::<Utc>::MIN_UTC.into(),
        }
    }
}