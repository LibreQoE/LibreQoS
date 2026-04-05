use lqos_probe::{ProbeClass, ProbeClient, ProbeObservation, ProbeRequest};
use std::sync::OnceLock;
use std::time::Duration;
use tracing::warn;

static PROBE_CLIENT: OnceLock<ProbeClient> = OnceLock::new();

/// Registers the process-wide shared probe client for `lqosd` services.
pub fn install_probe_client(probe_client: ProbeClient) {
    if PROBE_CLIENT.set(probe_client).is_err() {
        warn!("Shared probe client was already installed; keeping the original instance");
    }
}

/// Issues a blocking probe batch against the shared probe manager.
///
/// Side effects: blocks the current Tokio worker thread while awaiting the
/// shared probe manager response.
pub fn probe_batch_blocking(
    requests: Vec<ProbeRequest>,
    max_age: Duration,
) -> Result<Vec<ProbeObservation>, String> {
    if requests.is_empty() {
        return Ok(Vec::new());
    }

    let client = probe_client()?.clone();
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current()
            .block_on(client.probe_batch(requests, max_age))
            .map_err(|err| format!("shared probe batch failed: {err}"))
    })
}

/// Issues an RTT probe against the shared probe manager.
///
/// Side effects: requests an active ICMP probe from the shared probe manager.
pub async fn probe_round_trip_time(
    target: String,
    class: ProbeClass,
    timeout: Duration,
    max_age: Duration,
) -> Result<ProbeObservation, String> {
    let client = probe_client()?.clone();
    client
        .probe_round_trip_time(target, class, timeout, max_age)
        .await
        .map_err(|err| format!("shared RTT probe failed: {err}"))
}

fn probe_client() -> Result<&'static ProbeClient, String> {
    PROBE_CLIENT
        .get()
        .ok_or_else(|| "shared probe client is not initialized".to_string())
}
