#![allow(dead_code)]
#[derive(Clone, Debug)]
pub struct InfluxTimePeriod {
    pub start: String,
    pub aggregate: String,
    sample: i32,
}

const fn minutes_to_seconds(minutes: i32) -> i32 {
    minutes * 60
}

const fn hours_to_seconds(hours: i32) -> i32 {
    minutes_to_seconds(hours * 60)
}

const fn days_to_seconds(days: i32) -> i32 {
    hours_to_seconds(days * 24)
}

const SAMPLES_PER_GRAPH: i32 = 30;

const fn aggregate_window(seconds: i32) -> i32 {
    seconds / SAMPLES_PER_GRAPH
}

impl InfluxTimePeriod {
    pub fn new(period: &str) -> Self {
        let start_seconds = match period {
            "5m" => minutes_to_seconds(5),
            "15m" => minutes_to_seconds(15),
            "1h" => hours_to_seconds(1),
            "6h" => hours_to_seconds(6),
            "12h" => hours_to_seconds(12),
            "24h" => hours_to_seconds(24),
            "7d" => days_to_seconds(7),
            "28d" => days_to_seconds(28),
            _ => {
                tracing::warn!("Unknown period: {}", period);
                minutes_to_seconds(5)
            }
        };
        let start = format!("-{}s", start_seconds);       
        let aggregate_seconds = aggregate_window(start_seconds);
        let aggregate = format!("{}s", aggregate_seconds);
        let sample = start_seconds / 100;

        println!("Period: {period}, Seconds: {start_seconds}, AggSec: {aggregate_seconds}, Samples: {sample}");

        Self {
            start: start.to_string(),
            aggregate: aggregate.to_string(),
            sample
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

    pub fn aggregate_window_empty(&self) -> String {
        format!(
            "aggregateWindow(every: {}, fn: mean, createEmpty: true)",
            self.aggregate
        )
    }

    pub fn aggregate_window_fn(&self, mode: &str) -> String {
        format!(
            "aggregateWindow(every: {}, fn: {mode}, createEmpty: false)",
            self.aggregate
        )
    }

    pub fn sample(&self) -> String {
        format!("sample(n: {}, pos: 1)", self.sample)
    }
}

impl From<&String> for InfluxTimePeriod {
    fn from(period: &String) -> Self {
        Self::new(period)
    }
}