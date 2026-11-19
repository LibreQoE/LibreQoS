//! Store I/O for the dynamic-circuit overlay.
//!
//! Two files, both under `config.lqos_directory`:
//! - `dynamic_circuits.json` — membership and rates. Its bytes are hashed into
//!   the topology source generation, so it must never carry liveness.
//! - `dynamic_circuits_state.json` — the liveness checkpoint
//!   (`{circuit_id: last_seen_unix}`), deliberately unhashed.
//!
//! Both files are written atomically and only when their bytes would change. A
//! liveness-only RADIUS interim therefore writes nothing at all: the store bytes
//! are unchanged, and liveness reaches the checkpoint on the maintenance tick.

use crate::DynamicCircuit;
use anyhow::{Context, Result, anyhow};
use lqos_config::{
    Config, DynamicCircuitsFile, ShapedDevice, dynamic_circuits_path, dynamic_circuits_state_path,
};
use lqos_utils::{hash_to_i64, normalize_circuit_id_key};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use tracing::warn;

fn recompute_hashes(device: &mut ShapedDevice) {
    device.circuit_hash = hash_to_i64(&device.circuit_id);
    device.device_hash = hash_to_i64(&device.device_id);
    device.parent_hash = hash_to_i64(&device.parent_node);
}

/// Writes `raw` to `path` through a same-directory temp file plus rename.
fn write_bytes_atomically(path: &Path, raw: &[u8]) -> Result<()> {
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)
        .with_context(|| format!("create temp file {}", temp_path.display()))?;
    file.write_all(raw)
        .with_context(|| format!("write temp file {}", temp_path.display()))?;
    file.sync_all()
        .with_context(|| format!("sync temp file {}", temp_path.display()))?;
    std::fs::rename(&temp_path, path).with_context(|| {
        format!(
            "rename temp file {} to {}",
            temp_path.display(),
            path.display()
        )
    })?;
    Ok(())
}

/// Serializes `value` as pretty JSON and writes it only when the resulting
/// bytes differ from the file already on disk.
///
/// Returns whether it wrote. An unchanged file is left completely untouched: no
/// temp file, no rename, no fsync.
///
/// Side effects: reads `path`; on a content change, writes a temp file, fsyncs
/// it, and renames it over `path`.
fn write_json_if_changed<T: Serialize>(path: &Path, value: &T) -> Result<bool> {
    let raw = serde_json::to_string_pretty(value)
        .with_context(|| format!("serialize {}", path.display()))?;
    match std::fs::read(path) {
        Ok(existing) if existing == raw.as_bytes() => return Ok(false),
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| format!("read {}", path.display()));
        }
    }
    write_bytes_atomically(path, raw.as_bytes())
        .with_context(|| format!("write {}", path.display()))?;
    Ok(true)
}

/// The liveness checkpoint document: normalized circuit id to `last_seen_unix`.
///
/// REV-C2: a stamp of 0 means "no evidence" — a clock that never became
/// readable, or a degraded checkpoint the loader could not restore. Such an
/// entry is omitted rather than persisted, so a dead-clock tick can never write
/// a zero over a good stamp.
fn liveness_checkpoint(circuits: &[DynamicCircuit]) -> BTreeMap<String, u64> {
    circuits
        .iter()
        .filter(|circuit| circuit.last_seen_unix > 0)
        .map(|circuit| {
            (
                normalize_circuit_id_key(&circuit.shaped.circuit_id),
                circuit.last_seen_unix,
            )
        })
        .collect()
}

/// Reads the membership/rates store, recomputing hashes and de-duplicating by
/// circuit id (case-insensitive, keeping the last entry).
///
/// A MISSING or EMPTY file is an empty membership (`Ok`). An unreadable or
/// unparseable one is NOT: it returns the error, because a store this process
/// cannot read still holds the circuits other rows must not overwrite.
///
/// Side effects: reads `dynamic_circuits.json`.
fn read_store(config: &Config) -> Result<Vec<DynamicCircuit>> {
    let path = dynamic_circuits_path(config);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("read dynamic circuits file {}", path.display()));
        }
    };

    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut parsed: DynamicCircuitsFile = serde_json::from_str(&raw).with_context(|| {
        format!(
            "parse dynamic circuits file {} (the file exists and its rows must not be dropped)",
            path.display()
        )
    })?;

    for circuit in &mut parsed.circuits {
        recompute_hashes(&mut circuit.shaped);
    }

    Ok(parsed
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
        }))
}

/// Reads the liveness checkpoint. The second return value is false when the
/// file is missing or unparseable, which the loader treats as "no checkpoint
/// evidence" and answers with one bounded startup grace.
///
/// Side effects: reads `dynamic_circuits_state.json`.
fn read_liveness_checkpoint(config: &Config) -> (BTreeMap<String, u64>, bool) {
    let path = dynamic_circuits_state_path(config);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return (BTreeMap::new(), false);
    };
    match serde_json::from_str::<BTreeMap<String, u64>>(&raw) {
        Ok(checkpoint) => (checkpoint, true),
        Err(err) => {
            warn!(
                "Unable to parse dynamic-circuit liveness checkpoint {} ({err}); granting one startup grace",
                path.display()
            );
            (BTreeMap::new(), false)
        }
    }
}

/// Loads the membership/rates store and restores liveness from the checkpoint.
///
/// Liveness precedence: the checkpoint wins; a legacy `last_seen_unix` still
/// present in the store file (pre-checkpoint releases serialized it there) is
/// imported once and the store is rewritten without it. Restored rows that carry
/// no liveness EVIDENCE get one bounded startup grace (their `last_seen` becomes
/// now) instead of expiring on the first maintenance tick. No evidence means: no
/// checkpoint file, an unparseable one, an EMPTY one (a degraded path may have
/// written `{}` over a valid checkpoint), or a per-row stamp of 0. Future
/// timestamps are clamped to now.
///
/// Side effects: reads `dynamic_circuits.json` and `dynamic_circuits_state.json`;
/// rewrites them when the one-time legacy import or the startup grace changed
/// their content.
///
/// Returns `Err` when the store exists but cannot be read or parsed. A missing
/// or empty file is an empty membership, not a failure. Callers must not treat
/// the error as "empty": the file still holds circuits this process never saw.
pub(crate) fn load_dynamic_circuits_from_disk(config: &Config) -> Result<Vec<DynamicCircuit>> {
    let mut circuits = read_store(config)?;
    if circuits.is_empty() {
        return Ok(circuits);
    }

    let (checkpoint, parsed) = read_liveness_checkpoint(config);
    // REV-C1(c): an EMPTY checkpoint is not evidence. The tick used to write
    // `{}` from an empty in-memory snapshot, and treating that as authoritative
    // would leave every row at `last_seen = 0` and let the first tick delete the
    // whole overlay.
    let checkpoint_usable = parsed && !checkpoint.is_empty();
    // REV-C1(b): an unavailable clock is not a timestamp. `None` disables both
    // the clamp and the grace write, so no value derived from a dead clock is
    // ever persisted.
    let now_unix = lqos_utils::unix_time::unix_now().ok();
    let clamp = |stamp: u64| now_unix.map_or(stamp, |now| stamp.min(now));
    let mut needs_liveness_persist = false;

    for circuit in &mut circuits {
        let key = normalize_circuit_id_key(&circuit.shaped.circuit_id);
        // A zero stamp is "no evidence" for that row, exactly like a missing
        // entry: it must fall through to the grace arm, never be restored as 0.
        if let Some(seen) = checkpoint.get(&key).copied().filter(|seen| *seen > 0) {
            circuit.last_seen_unix = clamp(seen);
            continue;
        }
        if circuit.last_seen_unix > 0 {
            // Legacy store file: the value moves to the checkpoint on this load.
            circuit.last_seen_unix = clamp(circuit.last_seen_unix);
            needs_liveness_persist = true;
            continue;
        }
        if !checkpoint_usable {
            match now_unix {
                Some(now) => {
                    circuit.last_seen_unix = now;
                    needs_liveness_persist = true;
                }
                None => warn!(
                    "Unable to read the clock; dynamic circuit '{}' keeps no startup grace and is not persisted (liveness stays unproven, so TTL can never expire it)",
                    circuit.shaped.circuit_id
                ),
            }
        }
    }

    if needs_liveness_persist {
        // Both writes are best-effort at startup: the in-memory world is the
        // authority for this process, and a failed persist must not abort the
        // load.
        if let Err(err) = persist_dynamic_circuits_to_disk(config, &circuits) {
            warn!("Unable to persist migrated dynamic circuits: {err:?}");
        }
        if let Err(err) = persist_liveness_checkpoint(config, &circuits) {
            warn!("Unable to persist migrated dynamic-circuit liveness: {err:?}");
        }
    }

    Ok(circuits)
}

/// Refuses a membership mutation when this process does not hold the store's
/// full membership.
///
/// A failed startup load (an unreadable or unparseable `dynamic_circuits.json`,
/// or a config that never loaded) leaves the in-memory snapshot EMPTY while the
/// file still holds every circuit. The first mutation would then rewrite the
/// file from that empty snapshot and delete the rest of the overlay. Writes are
/// therefore refused until a load succeeds; the error names the file, because
/// the operator has to repair it.
///
/// Side effects: none (reads process state).
pub(crate) fn require_store_membership(config: &Config) -> Result<()> {
    if crate::state::dynamic_store_membership() == crate::state::DynamicStoreMembership::Failed {
        let path = dynamic_circuits_path(config);
        return Err(anyhow!(
            "refusing to write dynamic-circuit membership: this process could not load {} and does not hold the full membership, so the write would drop the circuits it never read",
            path.display()
        ));
    }
    Ok(())
}

/// Whether the maintenance tick should checkpoint liveness at all (REV-C1(a)).
///
/// An EMPTY snapshot is not evidence — a failed startup `load_config()` leaves
/// it empty for the process lifetime while the store file is non-empty — and an
/// install that does not run dynamic circuits must not grow a stray
/// `dynamic_circuits_state.json`. Checkpointing is therefore gated on a
/// non-empty snapshot AND the configured feature.
pub(crate) fn should_checkpoint_liveness(
    config: &Config,
    circuits: &[DynamicCircuit],
) -> bool {
    !circuits.is_empty()
        && config
            .dynamic_circuits
            .as_ref()
            .is_some_and(|cfg| cfg.enabled)
}

/// Writes the membership/rates store, and with it the liveness checkpoint, when
/// their content changed.
///
/// A liveness-only renewal leaves the store bytes identical, so neither file is
/// touched here; liveness motion is checkpointed by the actor's maintenance
/// tick instead.
///
/// The checkpoint write is BEST-EFFORT (REV-C4): once the membership write has
/// landed, a liveness-persist failure must not fail an operation the caller is
/// told succeeded, and must not skip the in-memory snapshot publish that keeps
/// memory in step with disk.
///
/// Side effects: writes `dynamic_circuits.json` and, only when the store
/// actually changed, `dynamic_circuits_state.json`.
pub(crate) fn persist_dynamic_circuits_to_disk(
    config: &Config,
    circuits: &[DynamicCircuit],
) -> Result<()> {
    let file = DynamicCircuitsFile {
        schema_version: 1,
        circuits: circuits.to_vec(),
    };
    let store_changed = write_json_if_changed(&dynamic_circuits_path(config), &file)
        .context("write dynamic circuits store")?;
    if store_changed
        && let Err(err) = persist_liveness_checkpoint(config, circuits)
    {
        warn!("Membership was stored but the liveness checkpoint failed: {err:?}");
    }
    Ok(())
}

/// Checkpoints liveness for the current membership, writing
/// `dynamic_circuits_state.json` only when its bytes changed.
///
/// This is the bounded-interval write path: the actor calls it from its
/// maintenance tick, never per RADIUS interim.
///
/// REV-C2: a membership that carries NO stamp at all (every row at 0 — the
/// dead-clock boot of REV-C1(b)) is not evidence, so nothing is written and a
/// valid on-disk checkpoint survives. A membership that is genuinely EMPTY still
/// writes `{}`: that is the truth about the membership, and it clears the stamps
/// of removed circuits.
///
/// Returns whether a checkpoint was written.
///
/// Side effects: may write `dynamic_circuits_state.json`.
pub(crate) fn persist_liveness_checkpoint(
    config: &Config,
    circuits: &[DynamicCircuit],
) -> Result<bool> {
    let checkpoint = liveness_checkpoint(circuits);
    if checkpoint.is_empty() && !circuits.is_empty() {
        return Ok(false);
    }
    write_json_if_changed(&dynamic_circuits_state_path(config), &checkpoint)
        .context("write dynamic circuits liveness checkpoint")
}
