use lqos_bus::{bus_request, BusRequest, BusResponse};
use std::collections::HashMap;

use super::Task;

pub struct Throughput {
    bits_per_second: (u64, u64),
    packets_per_second: (u64, u64),
    shaped_bits_per_second: (u64, u64),
}

pub struct CircuitThroughput {
    circuits: HashMap<String, Throughput>
}

impl Throughput {
    async fn get() -> Self {
        if let Ok(messages) = bus_request(vec![BusRequest::GetCurrentThroughput]).await {
            for msg in messages {
                let Throughput {bits_per_second, packets_per_second, shaped_bits_per_second } = msg;
                return msg;
            }
        }
        Throughput {
            bits_per_second: (0, 0),
            packets_per_second: (0, 0),
            shaped_bits_per_second: (0, 0),
        }
    }
}

impl Task for Throughput {
    fn execute(&self) -> TaskResult {
        self.get()
    }

    fn key(&self) -> String {
        String::from("THROUGHPUT")
    }

    fn cacheable(&self) -> bool { true }
}