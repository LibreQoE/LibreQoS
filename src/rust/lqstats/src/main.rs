use anyhow::Result;
use lqos_bus::{bus_request, BusRequest, BusResponse, StatsRequest};

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> Result<()> {
  for resp in bus_request(vec![BusRequest::GetLongTermStats(StatsRequest::CurrentTotals)]).await? {
    if let BusResponse::LongTermTotals(stats) = resp {
      println!("{stats:?}");
    }
  }
  for resp in bus_request(vec![BusRequest::GetLongTermStats(StatsRequest::AllHosts)]).await? {
    if let BusResponse::LongTermHosts(stats) = resp {
      println!("{stats:?}");
    }
  }
  for resp in bus_request(vec![BusRequest::GetLongTermStats(StatsRequest::Tree)]).await? {
    if let BusResponse::LongTermTree(stats) = resp {
      println!("{stats:?}");
    }
  }
  Ok(())
}
