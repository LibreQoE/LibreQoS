use crate::queue_structure::{
    QueueStructureError, queue_network::QueueNetwork, queue_node::QueueNode,
    read_queueing_structure,
};
use arc_swap::ArcSwap;
use lqos_utils::file_watcher::FileWatcher;
use lqos_utils::normalize_circuit_id_key;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use thiserror::Error;
use tracing::{debug, error, info, warn};

/// Global queue structure (from `queueingStructure.json`)
pub static QUEUE_STRUCTURE: Lazy<ArcSwap<QueueStructure>> =
    Lazy::new(|| ArcSwap::new(Arc::new(QueueStructure::new())));
static INITIAL_EFFECTIVE_RATES: Lazy<EffectiveRates> = Lazy::new(|| {
    let initial = QUEUE_STRUCTURE.load();
    initial
        .maybe_queues
        .as_deref()
        .map(build_effective_rates)
        .unwrap_or_default()
});
/// Global effective node-rate overlay derived from `queuingStructure.json`.
///
/// This contains only named queue nodes that map cleanly back to authored network-tree
/// entries. Circuit/device rows and generated placeholder nodes are intentionally excluded.
pub static EFFECTIVE_NODE_RATES: Lazy<ArcSwap<HashMap<String, (f64, f64)>>> =
    Lazy::new(|| ArcSwap::new(Arc::new(INITIAL_EFFECTIVE_RATES.nodes.clone())));
/// Global effective circuit-rate overlay derived from `queuingStructure.json`.
///
/// This contains the currently programmed circuit queue rates keyed by normalized circuit ID.
pub static EFFECTIVE_CIRCUIT_RATES: Lazy<ArcSwap<HashMap<String, (f64, f64)>>> =
    Lazy::new(|| ArcSwap::new(Arc::new(INITIAL_EFFECTIVE_RATES.circuits.clone())));
/// Set when StormGuard should reconsider a non-live queue-plan snapshot.
pub static QUEUE_STRUCTURE_CHANGED_STORMGUARD: AtomicBool = AtomicBool::new(false);

#[allow(missing_docs)]
#[derive(Clone)]
/// Snapshot of the current flattened queue tree loaded from `queuingStructure.json`.
pub struct QueueStructure {
    pub maybe_queues: Option<Vec<QueueNode>>,
}

#[derive(Default)]
struct EffectiveRates {
    nodes: HashMap<String, (f64, f64)>,
    circuits: HashMap<String, (f64, f64)>,
}

impl QueueStructure {
    fn new() -> Self {
        if let Ok(queues) = read_queueing_structure() {
            Self {
                maybe_queues: Some(queues),
            }
        } else {
            Self { maybe_queues: None }
        }
    }
}

fn build_effective_rates(queues: &[QueueNode]) -> EffectiveRates {
    let mut node_rates = HashMap::with_capacity(queues.len());
    let mut circuit_rates = HashMap::new();
    for queue in queues {
        if let Some(name) = queue.name.as_ref()
            && !name.starts_with("Generated_PN_")
            && queue.circuit_id.is_none()
            && queue.device_id.is_none()
        {
            node_rates.insert(
                name.clone(),
                (
                    queue.download_bandwidth_mbps as f64,
                    queue.upload_bandwidth_mbps as f64,
                ),
            );
        }

        if queue.device_id.is_some() {
            continue;
        }
        let Some(circuit_id) = queue
            .circuit_id
            .as_deref()
            .map(normalize_circuit_id_key)
            .filter(|key| !key.is_empty())
        else {
            continue;
        };

        circuit_rates.insert(
            circuit_id,
            (
                queue.download_bandwidth_mbps as f64,
                queue.upload_bandwidth_mbps as f64,
            ),
        );
    }
    EffectiveRates {
        nodes: node_rates,
        circuits: circuit_rates,
    }
}

/// Global file watched for `queueStructure.json`.
/// Reloads the queue structure when it is available.
pub fn spawn_queue_structure_monitor() -> anyhow::Result<()> {
    std::thread::Builder::new()
        .name("Queue Structure Monitor".to_string())
        .spawn(|| {
            if let Err(e) = watch_for_queueing_structure_changing() {
                error!("Error watching for queueingStructure.json: {:?}", e);
            }
        })?;

    Ok(())
}

/// Reloads the queue structure from its generated JSON file.
///
/// This is exposed for consumers that must synchronize an in-memory snapshot with a
/// successfully applied shaping-tree generation instead of waiting for the file watcher.
pub fn reload_queue_structure() -> Result<(), QueueStructureError> {
    debug!("queueingStructure.json reload requested");
    let queues = read_queueing_structure()?;
    let effective_rates = build_effective_rates(&queues);
    let new_queue_structure = QueueStructure {
        maybe_queues: Some(queues),
    };
    QUEUE_STRUCTURE.store(Arc::new(new_queue_structure));
    EFFECTIVE_NODE_RATES.store(Arc::new(effective_rates.nodes));
    EFFECTIVE_CIRCUIT_RATES.store(Arc::new(effective_rates.circuits));
    Ok(())
}

fn update_queue_structure() {
    if let Err(err) = reload_queue_structure() {
        if QUEUE_STRUCTURE.load().maybe_queues.is_some() {
            warn!(
                "Failed to reload queuingStructure.json ({err:?}); preserving last-known-good snapshot"
            );
        } else {
            warn!(
                "Failed to load queuingStructure.json ({err:?}); leaving queue structure unavailable"
            );
            QUEUE_STRUCTURE.store(Arc::new(QueueStructure { maybe_queues: None }));
            EFFECTIVE_NODE_RATES.store(Arc::new(HashMap::new()));
            EFFECTIVE_CIRCUIT_RATES.store(Arc::new(HashMap::new()));
        }
    } else {
        QUEUE_STRUCTURE_CHANGED_STORMGUARD.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Fires up a Linux file system watcher than notifies
/// when `queuingStructure.json` changes, and triggers a reload.
fn watch_for_queueing_structure_changing() -> Result<(), QueueWatcherError> {
    // Get the path to watch
    let Ok(watch_path) = QueueNetwork::path() else {
        error!("Could not create path for queuingStructure.json");
        return Err(QueueWatcherError::CannotCreatePath);
    };

    // Do the watching
    let mut watcher = FileWatcher::new("queueingStructure.json", watch_path);
    watcher.set_file_created_callback(update_queue_structure);
    watcher.set_file_changed_callback(update_queue_structure);
    loop {
        let retval = watcher.watch();
        if retval.is_err() {
            info!("File watcher returned {retval:?}");
        }
    }
}

#[derive(Error, Debug)]
pub enum QueueWatcherError {
    #[error("Could not create the path buffer to find queuingStructure.json")]
    CannotCreatePath,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named_node(name: &str, down: u64, up: u64) -> QueueNode {
        QueueNode {
            name: Some(name.to_string()),
            download_bandwidth_mbps: down,
            upload_bandwidth_mbps: up,
            ..QueueNode::default()
        }
    }

    fn circuit_node(circuit_id: &str, down: u64, up: u64) -> QueueNode {
        QueueNode {
            circuit_id: Some(circuit_id.to_string()),
            download_bandwidth_mbps: down,
            upload_bandwidth_mbps: up,
            ..QueueNode::default()
        }
    }

    #[test]
    fn effective_circuit_rates_include_programmed_circuit_queues() {
        let queues = [
            named_node("Tower", 500, 200),
            circuit_node(" Circuit-42 ", 115, 25),
        ];

        let rates = build_effective_rates(&queues).circuits;

        assert_eq!(rates.get("circuit-42"), Some(&(115.0, 25.0)));
        assert!(!rates.contains_key("Tower"));
    }

    #[test]
    fn effective_circuit_rates_ignore_device_rows() {
        let queues = [QueueNode {
            circuit_id: Some("Circuit-42".to_string()),
            device_id: Some("device-1".to_string()),
            download_bandwidth_mbps: 50,
            upload_bandwidth_mbps: 10,
            ..QueueNode::default()
        }];

        let rates = build_effective_rates(&queues).circuits;

        assert!(rates.is_empty());
    }

    #[test]
    fn effective_node_rates_keep_node_overlay_filters() {
        let queues = [
            named_node("Tower", 500, 200),
            named_node("Generated_PN_Tower", 100, 40),
            QueueNode {
                name: Some("Device Row".to_string()),
                device_id: Some("device-1".to_string()),
                download_bandwidth_mbps: 50,
                upload_bandwidth_mbps: 10,
                ..QueueNode::default()
            },
        ];

        let rates = build_effective_rates(&queues).nodes;

        assert_eq!(rates.get("Tower"), Some(&(500.0, 200.0)));
        assert!(!rates.contains_key("Generated_PN_Tower"));
        assert!(!rates.contains_key("Device Row"));
    }

    #[test]
    fn named_circuit_queue_goes_only_to_circuit_overlay() {
        let queues = [QueueNode {
            name: Some("Named Circuit Queue".to_string()),
            circuit_id: Some("Circuit-42".to_string()),
            download_bandwidth_mbps: 115,
            upload_bandwidth_mbps: 25,
            ..QueueNode::default()
        }];

        let rates = build_effective_rates(&queues);

        assert!(!rates.nodes.contains_key("Named Circuit Queue"));
        assert_eq!(rates.circuits.get("circuit-42"), Some(&(115.0, 25.0)));
    }
}
