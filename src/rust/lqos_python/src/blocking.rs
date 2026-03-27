use anyhow::{Error, Result};
use lqos_bus::{BusClientError, BusRequest, BusResponse, bus_request};
use std::{
    thread::sleep,
    time::{Duration, Instant},
};

pub fn run_query(requests: Vec<BusRequest>) -> Result<Vec<BusResponse>> {
    let mut replies = Vec::with_capacity(8);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            replies.extend_from_slice(&bus_request(requests).await?);
            Ok(replies)
        })
}

fn should_retry_bus_error(error: &Error) -> bool {
    let Some(bus_error) = error.downcast_ref::<BusClientError>() else {
        return false;
    };
    matches!(
        bus_error,
        BusClientError::SocketNotFound
            | BusClientError::StreamReadError
            | BusClientError::StreamWriteError
    )
}

/// Runs a bus query, retrying briefly while the local bus socket is still starting up.
pub fn run_query_wait_for_bus(
    requests: Vec<BusRequest>,
    timeout: Duration,
    retry_interval: Duration,
) -> Result<Vec<BusResponse>> {
    let deadline = Instant::now() + timeout;
    loop {
        match run_query(requests.clone()) {
            Ok(replies) => return Ok(replies),
            Err(error) => {
                if !should_retry_bus_error(&error) || Instant::now() >= deadline {
                    return Err(error);
                }
                sleep(retry_interval);
            }
        }
    }
}
