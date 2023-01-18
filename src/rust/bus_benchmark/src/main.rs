use std::time::Instant;
use anyhow::Result;
use lqos_bus::{bus_request, BusRequest};

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> Result<()> {
    const RUNS: usize = 100;

    println!("Sending {RUNS} bus pings, please wait.");
    let mut times = Vec::new();
    for _ in 0 .. RUNS {
        let now = Instant::now();
        let responses = bus_request(vec![BusRequest::Ping]).await?;
        let runtime = now.elapsed();
        assert_eq!(responses.len(), 1);
        times.push(runtime);
    }
    let sum_usec: u128 = times.iter().map(|t| t.as_nanos()).sum();
    let avg_usec = sum_usec / RUNS as u128;
    println!("Average bus time: {avg_usec} nanoseconds");
    Ok(())
}
