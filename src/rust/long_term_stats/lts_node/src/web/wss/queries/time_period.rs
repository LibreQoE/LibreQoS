#[derive(Clone)]
pub struct InfluxTimePeriod {
    start: String,
    aggregate: String,
}

impl InfluxTimePeriod {
    pub fn new(period: &str) -> Self {
        let start = match period {
            "5m" => "-5m",
            "15m" => "-15m",
            "1h" => "-60m",
            "6h" => "-360m",
            "12h" => "-720m",
            "24h" => "-1440m",
            "7d" => "-10080m",
            "28d" => "-40320m",
            _ => "-5m",
        };

        let aggregate = match period {
            "5m" => "10s",
            "15m" => "30s",
            "1h" => "1m",
            "6h" => "6m",
            "12h" => "12m",
            "24h" => "24m",
            "7d" => "210m",
            "28d" => "4h",
            _ => "10s",
        };

        Self {
            start: start.to_string(),
            aggregate: aggregate.to_string(),
        }
    }

    pub fn range(&self) -> String {
        format!("range(start: {})", self.start)
    }

    pub fn aggregate_window(&self) -> String {
        format!(
            "aggregateWindow(every: {}, fn: mean, createEmpty: false)",
            self.aggregate
        )
    }
}

impl From<&String> for InfluxTimePeriod {
    fn from(period: &String) -> Self {
        Self::new(period)
    }
}