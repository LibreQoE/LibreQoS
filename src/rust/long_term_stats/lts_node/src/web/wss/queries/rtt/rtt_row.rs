use chrono::{DateTime, FixedOffset, Utc};
use influxdb2::FromDataPoint;

#[derive(Debug, FromDataPoint)]
pub struct RttRow {
    pub host_id: String,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub time: DateTime<FixedOffset>,
}

impl Default for RttRow {
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

#[derive(Debug, FromDataPoint)]
pub struct RttSiteRow {
    pub host_id: String,
    pub rtt_min: f64,
    pub rtt_max: f64,
    pub rtt_avg: f64,
    pub time: DateTime<FixedOffset>,
}

impl Default for RttSiteRow {
    fn default() -> Self {
        Self {
            host_id: "".to_string(),
            rtt_min: 0.0,
            rtt_max: 0.0,
            rtt_avg: 0.0,
            time: DateTime::<Utc>::MIN_UTC.into(),
        }
    }
}

#[derive(Debug, FromDataPoint)]
pub struct RttCircuitRow {
    pub host_id: String,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub time: DateTime<FixedOffset>,
}

impl Default for RttCircuitRow {
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