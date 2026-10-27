//! K1b §4.6 pin 3: StormGuard's read-only bus access to the bridge publish
//! outcome ring, following the topology thread's `ProbeBatch` pattern (a
//! consumer polling an `lqosd` service over the in-process bus).

use lqos_bus::{BridgePublishStatus, BusReply, BusRequest, BusResponse};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

/// The in-process bus sender type shared with `lqosd`'s bus dispatcher.
pub type StormguardBusSender =
    tokio::sync::mpsc::Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>;

const BUS_CALL_POLL_INTERVAL: Duration = Duration::from_millis(10);
const BUS_CALL_REPLY_TIMEOUT: Duration = Duration::from_millis(500);

/// Sends one request and awaits its reply, bounded by `timeout`.
async fn bus_call(
    bus_tx: &StormguardBusSender,
    request: BusRequest,
    timeout: Duration,
) -> anyhow::Result<BusReply> {
    let (once_tx, once_rx) = oneshot::channel();
    tokio::time::timeout(timeout, bus_tx.send((once_tx, request)))
        .await
        .map_err(|_| anyhow::anyhow!("timed out sending StormGuard bus request"))?
        .map_err(|_| anyhow::anyhow!("bus closed while sending StormGuard bus request"))?;
    let reply = tokio::time::timeout(timeout, once_rx)
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for StormGuard bus reply"))?
        .map_err(|_| anyhow::anyhow!("bus reply channel closed for StormGuard request"))?;
    Ok(reply)
}

/// Polls the bridge broker's outcome ring for one minted generation until it
/// settles (`Applied`/`Rejected`) or `timeout` elapses. `Pending`/`Retrying`
/// responses keep polling; an absent generation also keeps polling (the
/// topology manager may not have republished yet).
pub async fn wait_for_publish(
    bus_tx: StormguardBusSender,
    generation: &str,
    timeout: Duration,
) -> anyhow::Result<BridgePublishStatus> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| anyhow::anyhow!("Invalid publish wait timeout: {timeout:?}"))?;
    loop {
        let response = bus_call(
            &bus_tx,
            BusRequest::BridgePublishStatus {
                generation: generation.to_string(),
            },
            BUS_CALL_REPLY_TIMEOUT,
        )
        .await?
        .responses
        .into_iter()
        .next();

        match response {
            Some(BusResponse::BridgePublishStatus { status, .. }) => match status {
                Some(BridgePublishStatus::Applied) => {
                    return Ok(BridgePublishStatus::Applied);
                }
                Some(BridgePublishStatus::Rejected { reason }) => {
                    return Ok(BridgePublishStatus::Rejected { reason });
                }
                Some(BridgePublishStatus::Pending { .. })
                | Some(BridgePublishStatus::Retrying { .. })
                | None => {}
            },
            Some(BusResponse::Fail(message)) => {
                anyhow::bail!("bridge publish status lookup failed: {message}");
            }
            other => {
                anyhow::bail!("unexpected bridge publish status response: {other:?}");
            }
        }

        if Instant::now() >= deadline {
            anyhow::bail!(
                "timed out waiting for generation {generation} to reach a settled publish state"
            );
        }
        tokio::time::sleep(BUS_CALL_POLL_INTERVAL).await;
    }
}
