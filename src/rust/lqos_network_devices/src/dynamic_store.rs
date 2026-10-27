use crate::DynamicCircuit;
use anyhow::{Context, Result};
use lqos_config::{DynamicCircuitsFile, ShapedDevice, dynamic_circuits_path};
use lqos_utils::{hash_to_i64, normalize_circuit_id_key};
use serde::Serialize;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use tracing::warn;

fn recompute_hashes(device: &mut ShapedDevice) {
    device.circuit_hash = hash_to_i64(&device.circuit_id);
    device.device_hash = hash_to_i64(&device.device_id);
    device.parent_hash = hash_to_i64(&device.parent_node);
}

fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let raw = serde_json::to_string_pretty(value).context("serialize dynamic circuits JSON")?;
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)
        .with_context(|| format!("create temp dynamic circuits file {}", temp_path.display()))?;
    file.write_all(raw.as_bytes())
        .with_context(|| format!("write temp dynamic circuits file {}", temp_path.display()))?;
    file.sync_all()
        .with_context(|| format!("sync temp dynamic circuits file {}", temp_path.display()))?;
    std::fs::rename(&temp_path, path).with_context(|| {
        format!(
            "rename temp dynamic circuits file {} to {}",
            temp_path.display(),
            path.display()
        )
    })?;
    Ok(())
}

pub(crate) fn load_dynamic_circuits_from_disk() -> Vec<DynamicCircuit> {
    let Ok(config) = lqos_config::load_config() else {
        warn!("Unable to load lqos config while loading dynamic circuits; treating as empty");
        return Vec::new();
    };

    let path = dynamic_circuits_path(config.as_ref());
    if !path.exists() {
        return Vec::new();
    }

    let Ok(raw) = std::fs::read_to_string(&path) else {
        warn!(
            "Unable to read dynamic circuits file {}; treating as empty",
            path.display()
        );
        return Vec::new();
    };

    if raw.trim().is_empty() {
        return Vec::new();
    }

    let mut parsed: DynamicCircuitsFile = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(err) => {
            warn!(
                "Unable to parse dynamic circuits file {} ({err}); treating as empty",
                path.display()
            );
            return Vec::new();
        }
    };

    for circuit in &mut parsed.circuits {
        recompute_hashes(&mut circuit.shaped);
    }

    // De-duplicate by circuit_id (case-insensitive), keeping last entry.
    parsed
        .circuits
        .into_iter()
        .fold(Vec::<DynamicCircuit>::new(), |mut acc, circuit| {
            let key = normalize_circuit_id_key(&circuit.shaped.circuit_id);
            if let Some(pos) = acc
                .iter()
                .position(|c| normalize_circuit_id_key(&c.shaped.circuit_id) == key)
            {
                acc[pos] = circuit;
            } else {
                acc.push(circuit);
            }
            acc
        })
}

pub(crate) fn persist_dynamic_circuits_to_disk(circuits: &[DynamicCircuit]) -> Result<()> {
    let config = lqos_config::load_config().context("load lqos config")?;
    let path = dynamic_circuits_path(config.as_ref());
    let file = DynamicCircuitsFile {
        schema_version: 1,
        circuits: circuits.to_vec(),
    };
    atomic_write_json(&path, &file)
        .with_context(|| format!("write dynamic circuits file {}", path.display()))
}
