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

fn period_string_to_seconds(period: &str) -> i32 {
    let last_char = period.chars().last().unwrap();
    let number_part = &period[..period.len() - 1];
    let number = number_part.parse::<i32>().unwrap_or(5);
    let start_seconds = match last_char {
        's' => number,
        'm' => minutes_to_seconds(number),
        'h' => hours_to_seconds(number),
        'd' => days_to_seconds(number),
        _ => {
            tracing::warn!("Unknown time unit: {last_char}");
            minutes_to_seconds(5)      
        }
    };
    start_seconds
}

impl InfluxTimePeriod {
    pub fn new(period: &str) -> Self {
        let last_char = period.chars().last().unwrap();
        let number_part = &period[..period.len() - 1];
        let number = number_part.parse::<i32>().unwrap_or(5);
        let start_seconds = match last_char {
            's' => number,
            'm' => minutes_to_seconds(number),
            'h' => hours_to_seconds(number),
            'd' => days_to_seconds(number),
            _ => {
                tracing::warn!("Unknown time unit: {last_char}");
                minutes_to_seconds(5)      
            }
        };

        let start = format!("-{}s", start_seconds);       
        let aggregate_seconds = aggregate_window(start_seconds);
        let aggregate = format!("{}s", aggregate_seconds);
        let sample = start_seconds / 100;

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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_period_to_seconds() {
        assert_eq!(period_string_to_seconds("5s"), 5);
        assert_eq!(period_string_to_seconds("5m"), 300);
        assert_eq!(period_string_to_seconds("5h"), 18000);
        assert_eq!(period_string_to_seconds("5d"), 432000);

        // Test that an unknown returns the default
        assert_eq!(period_string_to_seconds("5x"), 300);
    }
}