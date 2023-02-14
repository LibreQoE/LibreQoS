use anyhow::Result;
use lqos_bus::{bus_request, BusRequest, BusResponse};

pub fn run_query(requests: Vec<BusRequest>) -> Result<Vec<BusResponse>> {
  let mut replies = Vec::with_capacity(8);
  tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(
    async {
      replies.extend_from_slice(&bus_request(requests).await?);
      Ok(replies)
    },
  )
}
