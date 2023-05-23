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
            "15m" => "10s",
            "1h" => "10s",
            "6h" => "1m",
            "12h" => "2m",
            "24h" => "4m",
            "7d" => "30m",
            "28d" => "1h",
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