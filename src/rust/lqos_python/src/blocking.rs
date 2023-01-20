use anyhow::Result;
use lqos_bus::{
    BusRequest, BusResponse, bus_request,
};

pub fn run_query(requests: Vec<BusRequest>) -> Result<Vec<BusResponse>> {
    let mut replies = Vec::with_capacity(8);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            replies.extend_from_slice(&bus_request(requests).await?);
            Ok(replies)
        })
}
