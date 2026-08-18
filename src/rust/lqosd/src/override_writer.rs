//! Serialized override file writes for `lqosd`.
//!
//! The writer actor coordinates normal override mutations issued by `lqosd` and
//! bus clients. It keeps retry and batching behavior in one place while the
//! existing process file lock remains the cross-process compatibility guard.

use crossbeam_channel::{Receiver, RecvTimeoutError, SendTimeoutError, Sender, bounded};
use lqos_bus::{BusResponse, OverrideLayerSelection, OverrideMutation, OverrideMutationResult};
use lqos_overrides::{OverrideFile, OverrideLayer, OverrideStore};
use std::sync::OnceLock;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, warn};

const ACTOR_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const OVERRIDE_LOCK_RETRY_ATTEMPTS: usize = 10;
const OVERRIDE_LOCK_INITIAL_RETRY_DELAY: Duration = Duration::from_millis(100);
const OVERRIDE_LOCK_MAX_RETRY_DELAY: Duration = Duration::from_millis(500);

static OVERRIDE_WRITER_SENDER: OnceLock<Sender<OverrideWriterCommand>> = OnceLock::new();

/// Errors returned by the override writer actor.
#[derive(Debug, Error)]
pub(crate) enum OverrideWriterError {
    /// The actor has not been started yet.
    #[error("override writer actor is not running")]
    NotRunning,
    /// The actor command queue remained full past the request timeout.
    #[error("override writer command timed out while queueing")]
    QueueTimeout,
    /// The actor command queue disconnected.
    #[error("override writer actor stopped before accepting the command")]
    QueueDisconnected,
    /// The actor did not reply before the request timeout.
    #[error("override writer timed out waiting for actor reply")]
    ReplyTimeout,
    /// The actor reply channel disconnected.
    #[error("override writer reply channel closed")]
    ReplyDisconnected,
    /// Override file I/O or parsing failed.
    #[error("{details}")]
    Store {
        /// Human-readable store error.
        details: String,
    },
}

#[derive(Debug)]
enum OverrideWriterCommand {
    Apply {
        layer: OverrideLayerSelection,
        mutations: Vec<OverrideMutation>,
        reply: Sender<Result<OverrideMutationResult, OverrideWriterError>>,
    },
}

/// Starts the override writer actor.
///
/// Side effects: spawns a long-running background thread and registers the actor sender globally.
pub(crate) fn start_override_writer_actor() -> Result<(), OverrideWriterError> {
    if OVERRIDE_WRITER_SENDER.get().is_some() {
        return Ok(());
    }

    let (tx, rx) = bounded::<OverrideWriterCommand>(64);
    std::thread::Builder::new()
        .name("override_writer".to_string())
        .spawn(move || actor_loop(rx))
        .map_err(|err| OverrideWriterError::Store {
            details: err.to_string(),
        })?;
    let _ = OVERRIDE_WRITER_SENDER.set(tx);
    Ok(())
}

fn actor_loop(rx: Receiver<OverrideWriterCommand>) {
    debug!("override writer actor starting");
    while let Ok(command) = rx.recv() {
        match command {
            OverrideWriterCommand::Apply {
                layer,
                mutations,
                reply,
            } => {
                let result = apply_mutations_with_retry(layer, &mutations);
                let _ = reply.send(result);
            }
        }
    }
    warn!("override writer actor command channel disconnected; exiting actor");
}

/// Applies a batch of mutations through the override writer actor.
///
/// Side effects: sends a command to the override writer actor. The actor may read and write an
/// override file.
pub(crate) fn apply_mutation_batch(
    layer: OverrideLayerSelection,
    mutations: Vec<OverrideMutation>,
) -> Result<OverrideMutationResult, OverrideWriterError> {
    let (reply_tx, reply_rx) = bounded(1);
    send_command(OverrideWriterCommand::Apply {
        layer,
        mutations,
        reply: reply_tx,
    })?;
    receive_reply(reply_rx)
}

/// Handles one bus override mutation batch request.
///
/// Side effects: sends a command to the override writer actor and returns a bus response.
pub(crate) fn apply_bus_mutation_batch(
    layer: OverrideLayerSelection,
    mutations: Vec<OverrideMutation>,
) -> BusResponse {
    match apply_mutation_batch(layer, mutations) {
        Ok(result) => BusResponse::OverrideMutationResult(result),
        Err(err) => BusResponse::Fail(err.to_string()),
    }
}

fn send_command(command: OverrideWriterCommand) -> Result<(), OverrideWriterError> {
    let sender = OVERRIDE_WRITER_SENDER
        .get()
        .cloned()
        .ok_or(OverrideWriterError::NotRunning)?;
    sender
        .send_timeout(command, ACTOR_COMMAND_TIMEOUT)
        .map_err(|err| match err {
            SendTimeoutError::Timeout(_) => OverrideWriterError::QueueTimeout,
            SendTimeoutError::Disconnected(_) => OverrideWriterError::QueueDisconnected,
        })
}

fn receive_reply<T>(
    reply_rx: Receiver<Result<T, OverrideWriterError>>,
) -> Result<T, OverrideWriterError> {
    match reply_rx.recv_timeout(ACTOR_COMMAND_TIMEOUT) {
        Ok(result) => result,
        Err(RecvTimeoutError::Timeout) => Err(OverrideWriterError::ReplyTimeout),
        Err(RecvTimeoutError::Disconnected) => Err(OverrideWriterError::ReplyDisconnected),
    }
}

fn apply_mutations_with_retry(
    layer: OverrideLayerSelection,
    mutations: &[OverrideMutation],
) -> Result<OverrideMutationResult, OverrideWriterError> {
    retry_lock_contention(|| apply_mutations_once(layer, mutations))
}

fn retry_lock_contention<T>(
    mut operation: impl FnMut() -> anyhow::Result<T>,
) -> Result<T, OverrideWriterError> {
    let mut delay = OVERRIDE_LOCK_INITIAL_RETRY_DELAY;
    let mut last_error = None;
    for attempt in 1..=OVERRIDE_LOCK_RETRY_ATTEMPTS {
        match operation() {
            Ok(result) => return Ok(result),
            Err(err) if is_retryable_override_lock_error(&err) => {
                last_error = Some(err.to_string());
                if attempt == OVERRIDE_LOCK_RETRY_ATTEMPTS {
                    break;
                }
                std::thread::sleep(delay);
                delay = delay.saturating_mul(2).min(OVERRIDE_LOCK_MAX_RETRY_DELAY);
            }
            Err(err) => {
                return Err(OverrideWriterError::Store {
                    details: err.to_string(),
                });
            }
        }
    }

    Err(OverrideWriterError::Store {
        details: format!(
            "override file remained locked after {OVERRIDE_LOCK_RETRY_ATTEMPTS} attempts: {}",
            last_error.unwrap_or_else(|| "unknown lock error".to_string())
        ),
    })
}

fn apply_mutations_once(
    layer: OverrideLayerSelection,
    mutations: &[OverrideMutation],
) -> anyhow::Result<OverrideMutationResult> {
    if mutations.is_empty() {
        return Ok(OverrideMutationResult {
            changed: false,
            changed_entities: Vec::new(),
        });
    }

    let override_layer = to_override_layer(layer);
    let mut overrides = OverrideStore::load_layer(override_layer)?;
    let result = apply_mutations_to_file(&mut overrides, mutations);
    if result.changed {
        OverrideStore::save_layer(override_layer, &overrides)?;
    }
    Ok(result)
}

fn to_override_layer(layer: OverrideLayerSelection) -> OverrideLayer {
    match layer {
        OverrideLayerSelection::Operator => OverrideLayer::Operator,
        OverrideLayerSelection::Stormguard => OverrideLayer::Stormguard,
        OverrideLayerSelection::Treeguard => OverrideLayer::Treeguard,
    }
}

fn is_retryable_override_lock_error(err: &anyhow::Error) -> bool {
    let message = err.to_string().to_ascii_lowercase();
    message.contains("lqos_overrides_locked") || message.contains("locked by another process")
}

fn apply_mutations_to_file(
    overrides: &mut OverrideFile,
    mutations: &[OverrideMutation],
) -> OverrideMutationResult {
    let mut changed_entities = Vec::new();

    for mutation in mutations {
        apply_one_mutation(overrides, mutation, &mut changed_entities);
    }

    changed_entities.sort();
    changed_entities.dedup();
    OverrideMutationResult {
        changed: !changed_entities.is_empty(),
        changed_entities,
    }
}

fn apply_one_mutation(
    overrides: &mut OverrideFile,
    mutation: &OverrideMutation,
    changed_entities: &mut Vec<String>,
) {
    match mutation {
        OverrideMutation::ClearNodeVirtualBatch { node_names } => {
            changed_entities.extend(overrides.remove_network_node_virtual_by_names(node_names));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_bus_mutation_batch, apply_mutations_to_file, is_retryable_override_lock_error,
        retry_lock_contention,
    };
    use lqos_bus::{BusResponse, OverrideLayerSelection, OverrideMutation};
    use lqos_overrides::OverrideFile;
    use std::cell::Cell;

    #[test]
    fn clear_node_virtual_batch_reports_only_changed_nodes() {
        let mut overrides = OverrideFile::default();
        overrides.set_network_node_virtual("Node A".to_string(), true);
        overrides.set_network_node_virtual("Node B".to_string(), true);

        let result = apply_mutations_to_file(
            &mut overrides,
            &[OverrideMutation::ClearNodeVirtualBatch {
                node_names: vec![
                    "Node B".to_string(),
                    "Node Missing".to_string(),
                    "Node A".to_string(),
                ],
            }],
        );

        assert!(result.changed);
        assert_eq!(
            result.changed_entities,
            vec!["Node A".to_string(), "Node B".to_string()]
        );
        assert!(overrides.network_adjustments().is_empty());
    }

    #[test]
    fn lock_contention_detection_is_specific_to_override_lock_errors() {
        let lock_error = anyhow::anyhow!(
            "LQOS_OVERRIDES_LOCKED: The LibreQoS overrides file lock is locked by another process"
        );
        let parse_error = anyhow::anyhow!("expected value at line 1 column 1");

        assert!(is_retryable_override_lock_error(&lock_error));
        assert!(!is_retryable_override_lock_error(&parse_error));
    }

    #[test]
    fn retry_lock_contention_retries_then_succeeds() {
        let attempts = Cell::new(0);

        let result = retry_lock_contention(|| {
            attempts.set(attempts.get() + 1);
            if attempts.get() < 3 {
                anyhow::bail!(
                    "LQOS_OVERRIDES_LOCKED: The LibreQoS overrides file lock is locked by another process"
                );
            }
            Ok("updated")
        });

        assert_eq!(result.expect("retry should eventually succeed"), "updated");
        assert_eq!(attempts.get(), 3);
    }

    #[test]
    fn retry_lock_contention_fails_immediately_on_non_lock_errors() {
        let attempts = Cell::new(0);

        let result = retry_lock_contention::<()>(|| {
            attempts.set(attempts.get() + 1);
            anyhow::bail!("invalid overrides json")
        });

        assert!(result.is_err());
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn bus_mutation_reports_failure_when_actor_is_not_running() {
        let response = apply_bus_mutation_batch(
            OverrideLayerSelection::Treeguard,
            vec![OverrideMutation::ClearNodeVirtualBatch {
                node_names: vec!["Node A".to_string()],
            }],
        );

        assert_eq!(
            response,
            BusResponse::Fail("override writer actor is not running".to_string())
        );
    }
}
