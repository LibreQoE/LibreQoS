//! Rung 1.5 §1.5: E2-backed effective-rate lookups for the two zero-CSV
//! consumers (the network-tree transport overlay and the circuit page).
//!
//! E2 (`topology_operator_view.json`) is the Rust-native source; the legacy
//! queuingStructure.json-backed effective-rate overlays were retired in K2
//! slice 0. The cache is refreshed when the operator view changes.

use lqos_config::TopologyOperatorViewFile;
use lqos_utils::normalize_circuit_id_key;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::collections::HashMap;

static E2_NODE_RATES: Lazy<RwLock<HashMap<String, (f64, f64)>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));
static E2_CIRCUIT_RATES: Lazy<RwLock<HashMap<String, (f64, f64)>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Rebuilds the caches from the current E2 artifact. A read failure leaves the
/// previous caches in place (never clears a healthy view).
pub fn refresh(config: &lqos_config::Config) {
    let Ok(view) = TopologyOperatorViewFile::load(config) else {
        return;
    };
    let mut nodes = HashMap::new();
    let mut circuits = HashMap::new();
    for node in &view.nodes {
        let (Some(down), Some(up)) = (node.download_mbps, node.upload_mbps) else {
            continue;
        };
        if node.is_circuit() {
            circuits.insert(normalize_circuit_id_key(&node.id), (down, up));
        } else {
            nodes.insert(node.name.clone(), (down, up));
            nodes.insert(node.id.clone(), (down, up));
        }
    }
    *E2_NODE_RATES.write() = nodes;
    *E2_CIRCUIT_RATES.write() = circuits;
}

/// Effective node rates by E2 display name or id.
pub fn node_rate(name: &str) -> Option<(f64, f64)> {
    E2_NODE_RATES.read().get(name).copied()
}

/// Effective circuit rates by normalized circuit key.
pub fn circuit_rate(circuit_key: &str) -> Option<(f64, f64)> {
    E2_CIRCUIT_RATES.read().get(circuit_key).copied()
}

#[cfg(test)]
pub(crate) fn replace_for_test(
    nodes: HashMap<String, (f64, f64)>,
    circuits: HashMap<String, (f64, f64)>,
) -> (
    HashMap<String, (f64, f64)>,
    HashMap<String, (f64, f64)>,
) {
    let previous_nodes = std::mem::replace(&mut *E2_NODE_RATES.write(), nodes);
    let previous_circuits = std::mem::replace(&mut *E2_CIRCUIT_RATES.write(), circuits);
    (previous_nodes, previous_circuits)
}

#[cfg(test)]
pub(crate) fn restore_for_test(
    nodes: HashMap<String, (f64, f64)>,
    circuits: HashMap<String, (f64, f64)>,
) {
    *E2_NODE_RATES.write() = nodes;
    *E2_CIRCUIT_RATES.write() = circuits;
}

#[cfg(test)]
mod tests {
    use super::*;
    use lqos_config::{Config, topology_operator_view_path};

    fn temp_config(tag: &str) -> Config {
        let dir = std::env::temp_dir().join(format!(
            "lqosd-e2-rates-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("test invariant")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("test invariant");
        Config {
            lqos_directory: dir.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        }
    }

    /// The bridge-mode E2 refresh (the topology watcher's callback): a
    /// published operator view populates the effective node + circuit rate
    /// caches. This is the behavior a bridge-only box gained by moving the
    /// watcher spawn to the common startup path.
    #[test]
    fn refresh_populates_node_and_circuit_rate_caches() {
        let config = temp_config("refresh");
        let path = topology_operator_view_path(&config);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("test invariant");
        }
        std::fs::write(
            &path,
            br#"{"schema_version":1,"nodes":[
                {"id":"site:root","name":"Root Site","type":"site","download_mbps":500.0,"upload_mbps":250.0},
                {"id":"circuit:abc","name":"Circuit ABC","type":"circuit","download_mbps":100.0,"upload_mbps":50.0}
            ]}"#,
        )
        .expect("test invariant");

        let previous = replace_for_test(HashMap::new(), HashMap::new());
        refresh(&config);
        assert_eq!(node_rate("Root Site"), Some((500.0, 250.0)));
        assert_eq!(node_rate("site:root"), Some((500.0, 250.0)));
        assert_eq!(
            circuit_rate(&normalize_circuit_id_key("circuit:abc")),
            Some((100.0, 50.0))
        );
        restore_for_test(previous.0, previous.1);
        let _ = std::fs::remove_dir_all(&config.lqos_directory);
    }
}
