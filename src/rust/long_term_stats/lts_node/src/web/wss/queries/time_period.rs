use serde_json::Value;

#[derive(Clone)]
pub struct InfluxTimePeriod {
    start: String,
    aggregate: String,
}

impl InfluxTimePeriod {
    pub fn new(period: Option<Value>) -> Self {
        if let Some(period) = period {
            let start = match period.as_str() {
                Some("5m") => "-5m",
                Some("15m") => "-15m",
                Some("1h") => "-60m",
                Some("6h") => "-360m",
                Some("12h") => "-720m",
                Some("24h") => "-1440m",
                Some("7d") => "-10080m",
                Some("28d") => "-40320m",
                _ => "-5m",
            };

            let aggregate = match period.as_str() {
                Some("5m") => "10s",
                Some("15m") => "10s",
                Some("1h") => "10s",
                Some("6h") => "1m",
                Some("12h") => "2m",
                Some("24h") => "4m",
                Some("7d") => "30m",
                Some("28d") => "1h",
                _ => "10s"
            };

            Self {
                start: start.to_string(),
                aggregate: aggregate.to_string(),
            }
        } else {
            Self {
                start: "-5m".to_string(),
                aggregate: "10s".to_string(),
            }
        }
    }

    pub fn range(&self) -> String {
        format!("range(start: {})", self.start)
    }

    pub fn aggregate_window(&self) -> String {
        format!("aggregateWindow(every: {}, fn: mean, createEmpty: false)", self.aggregate)
    }
}