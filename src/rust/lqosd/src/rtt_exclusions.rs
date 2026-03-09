use arc_swap::ArcSwap;
use fxhash::FxHashSet;
use lqos_overrides::OverrideFile;
use lqos_utils::hash_to_i64;
use once_cell::sync::Lazy;
use std::sync::Arc;
use tracing::warn;

static EXCLUDED_CIRCUIT_HASHES: Lazy<ArcSwap<FxHashSet<i64>>> =
    Lazy::new(|| ArcSwap::from_pointee(FxHashSet::default()));

fn store_from_override_file(of: &OverrideFile) {
    let mut set: FxHashSet<i64> = FxHashSet::default();
    set.reserve(of.rtt_excluded_circuits().len());
    for circuit_id in of.rtt_excluded_circuits() {
        set.insert(hash_to_i64(circuit_id));
    }
    EXCLUDED_CIRCUIT_HASHES.store(Arc::new(set));
}

/// Reload RTT exclusions from `lqos_overrides.json` (if present).
pub fn refresh_from_disk() {
    match OverrideFile::load() {
        Ok(of) => store_from_override_file(&of),
        Err(e) => warn!("Unable to load lqos_overrides.json for RTT exclusions: {e:?}"),
    }
}

/// Returns true if this circuit hash is excluded from RTT aggregation/summarization.
#[inline]
pub fn is_excluded_hash(circuit_hash: i64) -> bool {
    EXCLUDED_CIRCUIT_HASHES.load().contains(&circuit_hash)
}

/// Returns true if this circuit id is excluded from RTT aggregation/summarization.
#[inline]
pub fn is_excluded_circuit_id(circuit_id: &str) -> bool {
    is_excluded_hash(hash_to_i64(circuit_id))
}

/// Add/remove a circuit from the RTT exclusion list. Returns true if changed.
pub fn set_excluded_circuit_id(circuit_id: &str, excluded: bool) -> anyhow::Result<bool> {
    let mut of = OverrideFile::load()?;
    let changed = of.set_circuit_rtt_excluded_return_changed(circuit_id, excluded);
    if changed {
        of.save()?;
    }
    store_from_override_file(&of);
    Ok(changed)
}
