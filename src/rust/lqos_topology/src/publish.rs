struct EffectivePublishLock {
    _lock: ProcessFileLock,
}

fn acquire_effective_publish_lock(config: &Config) -> Result<EffectivePublishLock> {
    let lock_dir = Path::new(&config.lqos_directory);
    let lock_path = lock_dir.join(TOPOLOGY_EFFECTIVE_PUBLISH_LOCK_FILENAME);
    let guard_path = lock_dir.join(TOPOLOGY_EFFECTIVE_PUBLISH_LOCK_GUARD_FILENAME);
    let lock_config = ProcessLockConfig::new(
        &lock_path,
        lock_dir,
        guard_path,
        "publish effective topology artifacts",
        TOPOLOGY_EFFECTIVE_PUBLISH_LOCK_CONTENTION_CODE,
        "The LibreQoS topology effective publish lock",
    );
    let lock = ProcessFileLock::acquire(&lock_config).with_context(|| {
        format!(
            "Unable to acquire topology effective publish lock at {:?}",
            lock_path
        )
    })?;
    Ok(EffectivePublishLock { _lock: lock })
}

/// F-NA (docs/false_no_ack_outcome_card.md): mints the generation the
/// NEXT publish will produce for the current world, WITHOUT writing
/// through its publish legs — the twin of this file's publish chain.
/// Uses the RUNTIME build variant (the one the topology runtime
/// publishes through; binding gate 1 — the manager's non-runtime
/// variant hashes a different world and would re-invert the bug), then
/// the same build_shaping_inputs + content-hash stamp. No publish lock
/// is taken; a racing writer merely means the minted generation is
/// superseded by a newer one, never mis-correlated.
///
/// Phase D: the write arm runs the explicit repairs BEFORE loading (cadence
/// parity — the mint migrated/quarantined here before the inversion). A read
/// path that must never write uses [`mint_next_shaping_generation_read_only`].
pub fn mint_next_shaping_generation(config: &Config) -> Result<Option<String>> {
    TopologyOverridesFile::repair(config).context("Unable to repair topology overrides")?;
    lqos_config::repair_stale_topology_state(config)
        .context("Unable to repair stale topology state")?;
    let canonical = load_canonical_topology_state(config);
    let overrides = TopologyOverridesFile::load_for_config(config)
        .context("Unable to load topology overrides file")?;
    let health_state = TopologyAttachmentHealthStateFile::load(config)
        .context("Unable to load topology attachment health state")?;
    let artifacts =
        build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
            config,
            &canonical,
            &overrides,
            &health_state,
        )
        .map_err(|errors| {
            anyhow::anyhow!(
                "Unable to build effective topology for the generation mint: {}",
                errors.join(" | ")
            )
        })?;
    let Some(mut inputs) = build_shaping_inputs(config, &artifacts)? else {
        return Ok(None);
    };
    // §4.8: the effective export is part of the transaction's content, so the
    // minted generation must content-address it too (the same augmentation the
    // publish path applies).
    let effective_network_value = artifacts.effective_network.as_ref().map(|network| {
        let mut augmented = network.clone();
        recompile_effective_network_commits(&mut augmented, &inputs);
        augmented
    });
    inputs.shaping_generation = inputs
        .compute_shaping_generation_with_export(effective_network_value.as_ref())
        .context("Unable to hash shaping inputs for the generation mint")?;
    Ok(Some(inputs.shaping_generation))
}

/// Strictly read-only variant of [`mint_next_shaping_generation`]: loads the
/// canonical state, overrides, and attachment health WITHOUT creating,
/// migrating, or quarantining any file, then mints the shaping inputs (with
/// the generation stamped) through the read-only build variant. Used by the
/// scheduler readiness verify, which runs per tick and must never write.
fn mint_next_shaping_inputs_read_only(
    config: &Config,
) -> Result<Option<TopologyShapingInputsFile>> {
    let canonical = TopologyCanonicalStateFile::load(config)
        .context("Unable to load canonical topology state for the read-only mint")?;
    let overrides = TopologyOverridesFile::load_for_config_read_only(config)
        .context("Unable to load topology overrides (read-only)")?;
    let health_state = TopologyAttachmentHealthStateFile::load(config)
        .context("Unable to load topology attachment health state")?;
    let artifacts =
        build_effective_topology_artifacts_from_canonical_with_runtime_queue_context_read_only(
            config,
            &canonical,
            &overrides,
            &health_state,
        )
        .map_err(|errors| {
            anyhow::anyhow!(
                "Unable to build effective topology for the read-only generation mint: {}",
                errors.join(" | ")
            )
        })?;
    let Some(mut inputs) = build_shaping_inputs_with(config, &artifacts, true)? else {
        return Ok(None);
    };
    // §4.8: the effective export is part of the transaction's content, so the
    // minted generation must content-address it too (the same augmentation the
    // publish path applies).
    let effective_network_value = artifacts.effective_network.as_ref().map(|network| {
        let mut augmented = network.clone();
        recompile_effective_network_commits(&mut augmented, &inputs);
        augmented
    });
    inputs.shaping_generation = inputs
        .compute_shaping_generation_with_export(effective_network_value.as_ref())
        .context("Unable to hash shaping inputs for the read-only generation mint")?;
    Ok(Some(inputs))
}

/// Strictly read-only variant of [`mint_next_shaping_generation`] that returns
/// the minted generation string only.
pub fn mint_next_shaping_generation_read_only(config: &Config) -> Result<Option<String>> {
    Ok(mint_next_shaping_inputs_read_only(config)?.map(|inputs| inputs.shaping_generation))
}

/// The custody LEG that failed (the deriver ``; the implementer `` R1-R2).
///
/// Every `verified: Some(false)` verdict names exactly ONE leg; `None` is
/// reserved for `verified: None` (no inputs) and `Some(true)`. The 6
/// cause-known sites also `tracing::warn!`; the 3 content-mismatch sites
/// (`WorldGenerationMismatch`, `FileMintMismatch`, `EffectiveMismatch`) were
/// SILENT — this enum + the compared values make them attributable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapingCustodyLeg {
    /// Leg 1 — the freshly-minted world generation differs from the status.
    WorldGenerationMismatch,
    /// Leg 2 — the status names no `shaping_inputs.json` path.
    FilePathEmpty,
    /// Leg 2 — the on-disk shaping inputs could not be loaded.
    FileLoadError,
    /// Leg 2 — the on-disk shaping inputs are not semantically equal to the mint.
    FileMintMismatch,
    /// Leg 3 — the status names no operator-view path.
    OperatorPathEmpty,
    /// Leg 3 — the operator view could not be read.
    OperatorReadError,
    /// Leg 3 — the operator view could not be parsed.
    OperatorParseError,
    /// Leg 3 — the status names no effective generation.
    EffectiveGenerationEmpty,
    /// Leg 3 — the operator-view generation differs from the status effective one.
    EffectiveMismatch,
}

impl ShapingCustodyLeg {
    /// The stable token logged + carried across the pyfunction boundary.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorldGenerationMismatch => "world_generation_mismatch",
            Self::FilePathEmpty => "file_path_empty",
            Self::FileLoadError => "file_load_error",
            Self::FileMintMismatch => "file_mint_mismatch",
            Self::OperatorPathEmpty => "operator_path_empty",
            Self::OperatorReadError => "operator_read_error",
            Self::OperatorParseError => "operator_parse_error",
            Self::EffectiveGenerationEmpty => "effective_generation_empty",
            Self::EffectiveMismatch => "effective_mismatch",
        }
    }
}

/// The outcome of [`verify_published_shaping_generation`]: the custody verdict
/// plus the freshly-minted source-generation token for the world measured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapingGenerationVerification {
    /// `Some(true)` when every custody leg holds, `Some(false)` when any leg
    /// fails, and `None` when the current world produces no shaping inputs.
    pub verified: Option<bool>,
    /// The stable source generation of the world this verify measured. The
    /// scheduler uses it as the readiness/wait token.
    pub generation: String,
    /// The custody leg that failed; `None` iff `verified` is `Some(true)`/`None`.
    pub failed_leg: Option<ShapingCustodyLeg>,
    /// The first compared value for the failing leg (diagnostic, never a decision).
    pub expected_value: Option<String>,
    /// The second compared value for the failing leg (diagnostic, never a decision).
    pub actual_value: Option<String>,
}

/// Builds a `verified: Some(false)` verdict that NAMES its failing leg (R1) and
/// carries the two compared values (R2). Measurement-only: never alters control
/// flow or the `verified` value.
fn custody_fail(
    generation: String,
    leg: ShapingCustodyLeg,
    expected_value: Option<String>,
    actual_value: Option<String>,
) -> ShapingGenerationVerification {
    ShapingGenerationVerification {
        verified: Some(false),
        generation,
        failed_leg: Some(leg),
        expected_value,
        actual_value,
    }
}

/// Bounded SOURCE-lag tolerance for custody Leg 1 (seconds). The live source
/// churns on the integration cadence (~5-6 min), so a status published one
/// round ago legitimately differs; accept the lag while the status is within
/// this window (≥ 2 churn cycles) and fail a genuinely stale/wrong source.
const SOURCE_LAG_TOLERANCE_SECS: u64 = 900;

/// Clock-skew slack (seconds) for the freshness bound. A `generated_unix`
/// MORE than this far in the FUTURE is not a valid freshness signal (a bogus
/// far-future stamp would `saturating_sub` to age 0 and be tolerated forever),
/// so it is treated as NOT fresh.
const SOURCE_LAG_FUTURE_SLACK_SECS: u64 = 60;

/// Verifies the published runtime shaping generation against a fresh read-only
/// dry-run mint of the current world (K1b §4.8 content-addressed generation).
///
/// This is the complete custody check — three legs:
/// 1. WORLD ↔ GENERATION: the fresh mint (canonical + overrides + health →
///    fold → recompile export → with-export hash) must equal the published
///    `shaping_generation`.
/// 2. FILE ↔ MINT: the on-disk `shaping_inputs.json` named by the status must
///    be semantically equal to the fresh mint, so a hand-edited artifact
///    diverged from the world is not silently applied.
/// 3. OPERATOR VIEW ↔ STATUS: the operator-view file named by the status must
///    hash to the published `effective_generation`. This is the match-assert
///    the scheduler used to perform by re-deriving the hash in Python; it now
///    lives inside the custody door (Phase A §1 ruling 2).
///
/// Returns `Ok(ShapingGenerationVerification { verified: None, .. })` when the
/// current world produces no shaping inputs (no published generation to
/// compare). The `generation` token is always populated.
///
/// Side effects: reads only. Creates, migrates, or quarantines nothing.
pub fn verify_published_shaping_generation(
    config: &Config,
) -> Result<ShapingGenerationVerification> {
    let generation = lqos_config::compute_topology_source_generation(config)
        .context("Unable to compute the topology source generation for verification")?;
    let Some(expected) = mint_next_shaping_inputs_read_only(config)? else {
        return Ok(ShapingGenerationVerification {
            verified: None,
            generation,
            failed_leg: None,
            expected_value: None,
            actual_value: None,
        });
    };
    let status = TopologyRuntimeStatusFile::load(config)
        .context("Unable to load topology runtime status for generation verification")?;

    // Leg 1 — WORLD ↔ GENERATION. Compare the SOURCE generation (the world's
    // content-addressed identity) against the status's `source_generation`,
    // NOT the freshly-minted `shaping_generation` against the status's — the
    // latter lags by exactly ONE PUBLISH ROUND while the world churns (the
    // publish mints + writes the status on the scheduler cadence; this verify
    // runs on another), so a strict mint==status compare fails SPURIOUSLY and
    // freezes the effective-parent cache (the WebUI no-throughput chain: the
    // devices then render zero live data). The SOURCE generation is stable
    // across the mint/publish lag — it changes only when the world's inputs
    // change. The artifact CONTENT stays checked by Legs 2 (FILE ↔ MINT) and
    // 3 (OPERATOR VIEW ↔ STATUS), so a tampered world is still caught.
    let minted_generation = expected.shaping_generation.trim().to_string();
    let status_shaping_generation = status.shaping_generation.trim().to_string();
    let status_source_generation = status.source_generation.trim().to_string();
    // The WORLD identity is the SOURCE generation. The live source churns
    // (the UISP integration rewrites the import every ~5-6 min), so it differs
    // from the status-stamped source BETWEEN publishes even though the status
    // is the last consistent world — a HARD compare then failed every publish
    // and froze the tracker's effective-parent cache (the WebUI no-throughput
    // chain). Compare the source generations with a BOUNDED LAG TOLERANCE: a
    // mismatched source is accepted while the status is FRESH (within
    // [`SOURCE_LAG_TOLERANCE_SECS`], ≥ 2 churn cycles — the world is merely
    // ahead and the runtime will republish); a GENUINELY stale/wrong source
    // still FAILS Leg 1. The artifact CONTENT stays checked by Legs 2 (FILE ↔
    // MINT) and 3 (OPERATOR VIEW ↔ STATUS), so a tampered world is still caught.
    // Leg 1 is a FRESHNESS check: a source mismatch is tolerated while the
    // status is recent. A FUTURE `generated_unix` (beyond the clock-skew slack)
    // is NOT fresh — without this a bogus far-future stamp reads age 0 via
    // `saturating_sub` and a wrong source would be tolerated forever.
    let status_age_secs = match (now_unix(), status.generated_unix) {
        (Some(now), Some(generated)) => {
            if generated > now.saturating_add(SOURCE_LAG_FUTURE_SLACK_SECS) {
                u64::MAX
            } else {
                now.saturating_sub(generated)
            }
        }
        _ => u64::MAX,
    };
    // A FRESH status (within the bounded lag window) is the signal both the Leg-1
    // SOURCE-lag and the Leg-2 FILE↔MINT lag tolerances key on: the world churns
    // on the integration cadence, so the last-published artifact trails the fresh
    // mint by exactly one round. A genuinely STALE mismatch still fails.
    let status_fresh = status_age_secs <= SOURCE_LAG_TOLERANCE_SECS;
    let source_lag_tolerated = generation != status_source_generation && status_fresh;
    if generation != status_source_generation && !source_lag_tolerated {
        tracing::warn!(
            "Topology readiness: custody leg world_generation_mismatch (source={generation} status_source={status_source_generation} age_secs={status_age_secs} minted={minted_generation})"
        );
        return Ok(custody_fail(
            generation.clone(),
            ShapingCustodyLeg::WorldGenerationMismatch,
            Some(status_source_generation),
            Some(generation),
        ));
    }

    // Leg 2 — FILE ↔ MINT: the artifact of record the applied world came from.
    let path = status.shaping_inputs_path.trim();
    if path.is_empty() {
        tracing::warn!(
            "Topology readiness: custody leg file_path_empty; treating the artifact as unverified"
        );
        return Ok(custody_fail(
            generation,
            ShapingCustodyLeg::FilePathEmpty,
            None,
            None,
        ));
    }
    let on_disk = match load_shaping_inputs_at_path(Path::new(path)) {
        Ok(inputs) => inputs,
        Err(err) => {
            let reason = err.to_string();
            tracing::warn!(
                "Topology readiness: custody leg file_load_error at {path}: {reason}"
            );
            return Ok(custody_fail(
                generation,
                ShapingCustodyLeg::FileLoadError,
                Some(path.to_string()),
                Some(reason),
            ));
        }
    };
    if !on_disk.semantic_equals(&expected) {
        let file_generation = on_disk.shaping_generation.trim().to_string();
 // TWO-TIER (review: a blanket fresh-tolerate would let a
        // FRESH TAMPERED artifact load. So:
        //  (i) EXACT when the WORLDS MATCH (`generation == status.source_generation`):
        //      the on-disk artifact MUST equal the fresh mint's content — a tamper
        //      on the same world FAILS even on a fresh status;
        //  (ii) LAG-ONLY when the world CHURNED: the verify cannot re-mint the
        //      status's PAST world, so it requires the artifact-of-record IDENTITY
        //      (`on_disk.shaping_generation == status.shaping_generation`) and a
        //      FRESH status, else FAIL. (Residual, flagged: a tamper on a churned
        //      world rides the identity check.)
        let worlds_match = generation == status_source_generation;
        let record_identity =
            !status_shaping_generation.is_empty() && file_generation == status_shaping_generation;
        // EXACT content check in the lag tier: recompute the ON-DISK artifact's
        // content-only digest and require it == the status's recorded digest (a
        // load->edit->save preserves the `shaping_generation` field, so the
        // digest is what catches a churned-world content tamper). Backward
        // compatible: an older publish without the digest falls back to the
        // identity check.
        let content_exact = if status.shaping_content_generation.trim().is_empty() {
            record_identity
        } else {
            matches!(
                on_disk.compute_shaping_generation_with_export(None),
                Ok(recomputed) if recomputed == status.shaping_content_generation.trim()
            )
        };
        if !worlds_match && status_fresh && content_exact {
            tracing::debug!(
                "Topology readiness: custody leg file_mint_mismatch TOLERATED (fresh status; one-churn artifact lag; content digest matches) (mint={minted_generation} file={file_generation})"
            );
        } else {
            tracing::warn!(
                "Topology readiness: custody leg file_mint_mismatch (mint={minted_generation} file={file_generation} worlds_match={worlds_match} record_identity={record_identity} content_exact={content_exact} age_secs={status_age_secs})"
            );
            return Ok(custody_fail(
                generation,
                ShapingCustodyLeg::FileMintMismatch,
                Some(minted_generation),
                Some(file_generation),
            ));
        }
    }

    // Leg 3 — OPERATOR VIEW ↔ STATUS: the scheduled match-assert, moved here.
    let operator_view_path = status.operator_view_file_path();
    if operator_view_path.is_empty() {
        tracing::warn!(
            "Topology readiness: custody leg operator_path_empty; treating the operator view as unverified"
        );
        return Ok(custody_fail(
            generation,
            ShapingCustodyLeg::OperatorPathEmpty,
            None,
            None,
        ));
    }
    let raw = match std::fs::read_to_string(operator_view_path) {
        Ok(raw) => raw,
        Err(err) => {
            let reason = err.to_string();
            tracing::warn!(
                "Topology readiness: custody leg operator_read_error at {operator_view_path}: {reason}"
            );
            return Ok(custody_fail(
                generation,
                ShapingCustodyLeg::OperatorReadError,
                Some(operator_view_path.to_string()),
                Some(reason),
            ));
        }
    };
    let operator_view = match serde_json::from_str::<Value>(&raw) {
        Ok(value) => value,
        Err(err) => {
            let reason = err.to_string();
            tracing::warn!(
                "Topology readiness: custody leg operator_parse_error at {operator_view_path}: {reason}"
            );
            return Ok(custody_fail(
                generation,
                ShapingCustodyLeg::OperatorParseError,
                Some(operator_view_path.to_string()),
                Some(reason),
            ));
        }
    };
    let expected_effective = status.effective_generation.trim().to_string();
    if expected_effective.is_empty() {
        tracing::warn!(
            "Topology readiness: custody leg effective_generation_empty; treating the operator view as unverified"
        );
        return Ok(custody_fail(
            generation,
            ShapingCustodyLeg::EffectiveGenerationEmpty,
            None,
            None,
        ));
    }
    let computed_effective = compute_effective_network_generation(&operator_view)
        .context("Unable to compute the operator view generation for verification")?;
    let verified = computed_effective == expected_effective;
    if !verified {
        tracing::warn!(
            "Topology readiness: custody leg effective_mismatch (status={expected_effective} computed={computed_effective})"
        );
    }
    Ok(ShapingGenerationVerification {
        verified: Some(verified),
        generation,
        failed_leg: if verified {
            None
        } else {
            Some(ShapingCustodyLeg::EffectiveMismatch)
        },
        expected_value: if verified {
            None
        } else {
            Some(expected_effective)
        },
        actual_value: if verified { None } else { Some(computed_effective) },
    })
}

/// R1-R3 operator-view generation diagnostic (measurement-only; cold path).
///
/// Reuses `lqos_config::compute_effective_network_file_generation` (the SAME
/// hash site the status uses — no 4th hash site) to compare the ON-DISK
/// operator view against the in-memory `effective_generation`, and NAMES the
/// serialization asymmetry (a write SKIPPED by the struct-equality dedup gate
/// leaves a key omitted from the file while the in-memory struct hashes the
/// defaulted key). R2: logs a POSITIVE verdict when the top-level KEYS are
/// equal (`keys equal; scalar(s) differ at …`) so a key-only diff is not
/// mistaken for "no diff". R3: logs the write-skip flag + the file mtime.
fn diagnose_operator_view_generation(
    config: &Config,
    path: &Path,
    in_memory: &Value,
    effective_generation: &str,
    write_skipped: bool,
) {
 // log the STATUS's own operator-view path + generation, so a
    // 2nd-writer / path mismatch is distinguishable from a round-trip
    // non-identity. And the IN-MEMORY round-trip hash: if it equals
    // `effective_generation` while the FILE does not, the file was written from
    // a DIFFERENT value (a 2nd writer); if it does NOT, the value itself is not
    // round-trip-stable (the R3 off-box test says it should be).
    let status = TopologyRuntimeStatusFile::load(config).ok();
    let status_path = status
        .as_ref()
        .map(|status| status.operator_view_file_path().to_string());
    let status_effective = status.as_ref().map(|status| status.effective_generation.clone());
    let roundtrip_generation = serde_json::to_string_pretty(in_memory)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|parsed| compute_effective_network_generation(&parsed).ok());
    let mtime_unix = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|dur| dur.as_secs());
    let file_generation = match lqos_config::compute_effective_network_file_generation(path) {
        Ok(generation) => generation,
        Err(err) => {
            tracing::warn!(
                "Topology operator-view diag: unable to hash the on-disk operator view at {path:?}: {err} (skipped_write={write_skipped}, file_mtime_unix={mtime_unix:?})"
            );
            return;
        }
    };
    if file_generation == effective_generation {
        tracing::info!(
            "Topology operator-view diag: file generation MATCHES status {effective_generation} (roundtrip={roundtrip_generation:?}, skipped_write={write_skipped}, file_mtime_unix={mtime_unix:?}, diag_path={path:?}, status_operator_view_path={status_path:?})"
        );
        return;
    }
    let on_disk = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let Some(disk) = on_disk.as_ref() else {
 // R2: an unreadable/unparseable on-disk file has NO
        // diff — a distinct verdict, never a misleading "keys equal".
        tracing::warn!(
            "Topology operator-view diag: file generation MISMATCH (status={effective_generation} file={file_generation} roundtrip={roundtrip_generation:?}) — on-disk operator view unreadable/unparseable; diff unavailable; skipped_write={write_skipped}, file_mtime_unix={mtime_unix:?}, diag_path={path:?}, status_operator_view_path={status_path:?}, status_effective={status_effective:?}"
        );
        return;
    };
    let (key_only, scalar_diff) = operator_view_diff(in_memory, disk);
    let (nodes_equal, nodes_paths, nodes_diff_count) = operator_view_nodes_diff(in_memory, disk);
    tracing::warn!(
        "Topology operator-view diag: file generation MISMATCH (status={effective_generation} file={file_generation} roundtrip={roundtrip_generation:?}) — keys_equal={} key_only_diff={key_only:?} scalar_diff_at={scalar_diff:?} nodes_equal={nodes_equal} nodes_paths={nodes_paths:?} nodes_diff_count={nodes_diff_count} skipped_write={write_skipped} file_mtime_unix={mtime_unix:?} diag_path={path:?} status_operator_view_path={status_path:?} status_effective={status_effective:?}",
        key_only.is_empty()
    );
}

/// R1/R2; a BOUNDED recursive diff of the
/// operator-view `nodes` array — PATHS only (`nodes[*].<field>` / `nodes[<i>]`),
/// first <= 8 distinct + a total count — plus a deep `nodes` equality bool. A
/// 925-node recursive value dump would flood the journal/capture.
fn operator_view_nodes_diff(in_memory: &Value, on_disk: &Value) -> (bool, Vec<String>, usize) {
    let in_nodes = in_memory.get("nodes");
    let disk_nodes = on_disk.get("nodes");
    if in_nodes == disk_nodes {
        return (true, Vec::new(), 0);
    }
    let (Some(a), Some(b)) = (
        in_nodes.and_then(Value::as_array),
        disk_nodes.and_then(Value::as_array),
    ) else {
        return (false, vec!["nodes".to_string()], 1);
    };
 // `zip` stops at the shorter array, so a LENGTH-only
    // difference would log `nodes_equal=false` yet no paths/count. Name it.
    let length_only = a.len() != b.len();
    let mut fields: Vec<String> = Vec::new();
    let mut node_only = 0usize;
    for (a_node, b_node) in a.iter().zip(b.iter()) {
        if a_node == b_node {
            continue;
        }
        match (a_node.as_object(), b_node.as_object()) {
            (Some(ao), Some(bo)) => {
                let mut keys: Vec<&String> = ao.keys().chain(bo.keys()).collect();
                keys.sort();
                keys.dedup();
                for key in keys {
                    if ao.get(key) == bo.get(key) {
                        continue;
                    }
                    if !fields.iter().any(|seen| seen == key) {
                        fields.push(key.clone());
                    }
                }
            }
            _ => node_only += 1,
        }
    }
    let mut paths: Vec<String> = Vec::new();
    if length_only {
        paths.push(format!("nodes (len {} vs {})", a.len(), b.len()));
    }
    paths.extend(fields.iter().map(|f| format!("nodes[*].{f}")));
    if node_only > 0 {
        paths.push(format!("nodes[<i>]x{node_only}"));
    }
    let count = fields.len() + node_only + usize::from(length_only);
    if paths.len() > 8 {
        paths.truncate(8);
    }
    (false, paths, count)
}

/// R2: classifies an in-memory vs on-disk operator-view difference into the
/// top-level keys present on only one side and the shared keys whose scalar
/// value differs. Sorted + deduplicated. Pure (no I/O) so it is unit-testable.
fn operator_view_diff(in_memory: &Value, on_disk: &Value) -> (Vec<String>, Vec<String>) {
    let (mut key_only, mut scalar_diff): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    if let (Some(in_mem), Some(disk)) = (in_memory.as_object(), on_disk.as_object()) {
        for key in in_mem.keys().chain(disk.keys()) {
            if !in_mem.contains_key(key) || !disk.contains_key(key) {
                if !key_only.contains(key) {
                    key_only.push(key.clone());
                }
            } else if in_mem.get(key) != disk.get(key) && !scalar_diff.contains(key) {
                scalar_diff.push(key.clone());
            }
        }
    }
    key_only.sort();
    scalar_diff.sort();
    (key_only, scalar_diff)
}

/// §8 (`docs/lqos_conf_single_source_card.md`): the emitter stamps the DERIVED
/// slot capacity on the transaction the bridge consumes. `n_circuits` is the
/// emitted world size (one leaf per circuit/range — the same count the bridge's
/// own fail-safe derivation uses); `carried` is the bridge's last reported
/// applied value, falling back to the previously published desired transaction
/// so the bound never shrinks across an emitter restart either.
fn stamp_circuit_capacity(transaction: &mut DataplaneTransaction, artifact_path: &Path) {
    let n_circuits = transaction
        .leaf_ops
        .iter()
        .filter(|op| matches!(op, DataplaneLeafOp::Upsert { .. }))
        .count();
    let carried = crate::circuit_capacity::applied_circuit_capacity()
        .or_else(|| prior_desired_capacity(artifact_path));
    transaction.circuit_capacity = Some(
        crate::circuit_capacity::derive_circuit_capacity(n_circuits, carried) as usize,
    );
}

/// The capacity carried by the previously published desired transaction (the
/// pre-report fallback). A missing/unparseable/no-field file is `None`.
fn prior_desired_capacity(artifact_path: &Path) -> Option<u32> {
    let bytes = std::fs::read(artifact_path).ok()?;
    let tx: DataplaneTransaction = serde_json::from_slice(&bytes).ok()?;
    tx.circuit_capacity
        .and_then(|value| u32::try_from(value).ok())
}

/// Publishes effective topology artifacts and shaping inputs for one source generation.
pub fn publish_effective_topology_artifacts(    config: &Config,
    artifacts: &EffectiveTopologyArtifacts) -> Result<()> {
    let _lock = acquire_effective_publish_lock(config)?;

    let effective_state_path = topology_effective_state_path(config);
    let effective_state_value = serde_json::to_value(&artifacts.effective)?;

    let effective_network_path = topology_effective_network_path(config);

    let shaping_inputs_path = topology_shaping_inputs_path(config);
    let shaping_inputs = build_shaping_inputs(config, artifacts)?;

    // Augment the effective network with per-node commit (guaranteed-rate) so
    // the DPDK HTB tree reads ceil + commit from a single artifact. §4.8: this
    // post-fold effective export is part of the transaction's content, so the
    // generation must content-address it (computed before the generation).
    let mut effective_network_value = match (
        artifacts.effective_network.as_ref(),
        shaping_inputs.as_ref(),
    ) {
        (Some(network), Some(inputs)) => {
            let mut augmented = network.clone();
            recompile_effective_network_commits(&mut augmented, inputs);
            Some(augmented)
        }
        (Some(network), None) => Some(network.clone()),
        (None, _) => None,
    };

    // §lqos-74t.29 (`tree.html?parent=0`): the effective network carries the
    // attachment-only radios as TOP-LEVEL nodes (the canonical network tree
    // includes them; effective reparenting leaves them at root). This tree IS the
    // DPDK compiler input (below), so the dedup MUST keep any circuit host: a
    // node named by a circuit's `effective_parent_node_id` REMAINS. The removed
    // nodes are childless + circuit-less attachment radios — the compiler elides
    // them anyway, so the compiled content_key is UNCHANGED.
    if let Some(network) = effective_network_value.as_mut() {
        let circuit_parents: BTreeSet<&str> = shaping_inputs
            .as_ref()
            .map(|inputs| {
                inputs
                    .circuits
                    .iter()
                    .map(|circuit| circuit.effective_parent_node_id.as_str())
                    .collect()
            })
            .unwrap_or_default();
        remove_top_level_attachment_only_nodes(network, &artifacts.canonical, &circuit_parents);
    }

    let prepared_shaping_inputs = match shaping_inputs {
        Some(mut shaping_inputs) => {
            shaping_inputs.shaping_generation = shaping_inputs
                .compute_shaping_generation_with_export(effective_network_value.as_ref())?;
            let shaping_inputs_value = serde_json::to_value(&shaping_inputs)?;
            Some((shaping_inputs, shaping_inputs_value))
        }
        None => None,
    };

    let current_effective_state = TopologyEffectiveStateFile::load(config).ok();
    if !current_effective_state
        .as_ref()
        .is_some_and(|current| effective_state_payload_equals(current, &artifacts.effective))
    {
        atomic_write_json_value(&effective_state_path, &effective_state_value).with_context(
            || {
                format!(
                    "Unable to publish effective topology state at {:?}",
                    effective_state_path
                )
            },
        )?;
    }

    // Compile the DPDK dataplane transaction from the augmented network + shaping
    // inputs and publish it as a sibling artifact (the bridge consumes it).
    let dataplane_transaction_path = topology_dataplane_transaction_path(config);
    // Dynamic-ranges H1: the configured pool ranges are injected at the
    // transaction stage (post-catalog), gated on the promotion feature.
    let dynamic_ranges: Vec<lqos_config::DynamicCircuitRangeRule> = config
        .dynamic_circuits
        .as_ref()
        .filter(|cfg| cfg.enabled && cfg.enable_unknown_ip_promotion)
        .map(|cfg| cfg.ranges.clone())
        .unwrap_or_default();
    let dataplane_transaction_value = match (
        effective_network_value.as_ref(),
        prepared_shaping_inputs.as_ref(),
    ) {
        (Some(network), Some((shaping_inputs, _))) => {
            let mut transaction = compile_dataplane_transaction_with_ranges_and_sqm(
                network,
                shaping_inputs,
                &dynamic_ranges,
                &config.queues.default_sqm,
            );
            // §8: the emitter OWNS the derived slot capacity (never an operator
            // key). Stamp it before serializing; the read of the prior artifact
            // is the pre-report carried fallback and must precede the write.
            stamp_circuit_capacity(&mut transaction, &dataplane_transaction_path);
            Some(serde_json::to_value(&transaction)?)
        }
        _ => None,
    };
    if let Some(transaction) = dataplane_transaction_value.as_ref() {
        atomic_write_json_value(&dataplane_transaction_path, transaction).with_context(|| {
            format!(
                "Unable to publish dataplane transaction at {:?}",
                dataplane_transaction_path
            )
        })?;
    } else if dataplane_transaction_path.exists() {
        std::fs::remove_file(&dataplane_transaction_path).with_context(|| {
            format!(
                "Unable to remove stale dataplane transaction at {:?}",
                dataplane_transaction_path
            )
        })?;
    }

    // T5 rung 1: E2 — the operator/tree view, the SECOND projection of
    // the one canonical model (ruling 2). Full topology including
    // circuit-less sites (the F-LM §8-4 asymmetry becomes two honest
    // projections), devices inline on circuit nodes (F10), flat
    // id-sorted deterministic list. The model validates BEFORE the
    // projection is built (ruling 6 — a failed validate is a loud
    // publish error, never a silent pass-through).
    // Rung 1.5 §1.5: the operator override layer supplies the E2 rate-source
    // markers. A read failure degrades to base markers, never blocks publish.
    let operator_overrides = lqos_overrides::OverrideFile::load_for_config(config).ok();
    // Recompute the SOURCE generation AFTER all artifact writes (the built
    // artifacts, shaping inputs and transaction are stamped ABOVE) and BEFORE
    // the operator view + status are stamped. The caller computed its
    // `source_generation` BEFORE building the artifacts — a build that writes
    // compiled-shaping inputs the source hash covers — so that caller value is
    // STALE. Stamping the stale value made `verify_published_shaping_generation`
    // fail Leg 1 spuriously every round, freezing the effective-parent cache
    // (the WebUI no-throughput chain: the devices render zero live data).
    let source_generation = lqos_config::compute_topology_source_generation(config)
        .context("Unable to recompute the topology source generation at publish")?;
    let source_generation = source_generation.as_str();
    let operator_view = match build_operator_view(
        &artifacts.canonical,
        prepared_shaping_inputs.as_ref().map(|(inputs, _)| inputs),
        effective_network_value.as_ref(),
        operator_overrides.as_ref(),
        source_generation,
    ) {
        Ok(view) => view,
        Err(validation_errors) => {
            let error = format!(
                "Topology model failed validation at publish: {validation_errors:?}"
            );
            tracing::warn!("{error}");
            publish_topology_runtime_error_status(config, source_generation, &error)?;
            anyhow::bail!("{error}");
        }
    };
    let operator_view_path = topology_operator_view_path(config);
    let operator_view_value = serde_json::to_value(&operator_view)?;
    // R3 provenance: the struct-equality dedup gate — a false value SKIPS the
    // write and is exactly what the serialization-asymmetry mechanism predicts.
    let operator_view_write_skipped = operator_view.persisted_equals(config);
    if !operator_view_write_skipped {
        atomic_write_json_value(&operator_view_path, &operator_view_value).with_context(|| {
            format!(
                "Unable to publish operator view at {:?}",
                operator_view_path
            )
        })?;
    }

    // T5 rung 1 (ruling 3): network.effective.json DIES — HARD STOP,
    // no deprecation window. The writer arm is removed; a stale file
    // from an earlier build is REMOVED so any missed consumer fails
    // LOUD instead of silently reading stale data.
    if effective_network_path.exists() {
        std::fs::remove_file(&effective_network_path).with_context(|| {
            format!(
                "Unable to remove stale effective topology network at {:?}",
                effective_network_path
            )
        })?;
        tracing::warn!(
            "Removed stale network.effective.json at {:?} — the interchange retired in T5 rung 1; consumers read topology_operator_view.json",
            effective_network_path
        );
    }

    // Status fold (the #12 HIGH): the readiness chain must never name
    // a dead file — `operator_view_path` names the E2 operator view and
    // `effective_generation` is its content hash.
 // (b) /: `effective_generation` is the
    // CONTENT hash of the PERSISTED operator view. `serde_json::to_value` of an
    // f32 field does NOT round-trip through JSON (a 1-ULP loss), so hash EXACTLY
    // the serialized form — the SAME form `atomic_write_json_value` writes and
    // the custody verify re-reads — making status == file BY CONSTRUCTION.
    let persisted_operator_view_value: Value =
        serde_json::from_str(&serde_json::to_string_pretty(&operator_view_value)?).with_context(
            || "Unable to round-trip the operator view for its generation".to_string(),
        )?;
    let effective_generation = compute_effective_network_generation(&persisted_operator_view_value)
        .with_context(|| {
            "Unable to compute the operator view generation".to_string()
        })?;
 // R1-R3 DIAG; the operator-view
    // serialization asymmetry left a PERSISTENT `effective_mismatch` (Leg 3).
    // After the (possibly skipped) write, compare the ON-DISK semantic
    // generation against the in-memory status hash using the EXISTING file-gen
    // (no 4th hash site) and NAME the difference. Measurement-only.
    diagnose_operator_view_generation(
        config,
        &operator_view_path,
        &operator_view_value,
        &effective_generation,
        operator_view_write_skipped,
    );

    match prepared_shaping_inputs {
        Some((shaping_inputs, shaping_inputs_value)) => {
            let current_shaping_inputs = TopologyShapingInputsFile::load(config).ok();
            if !current_shaping_inputs
                .as_ref()
                .is_some_and(|current| current.semantic_equals(&shaping_inputs))
            {
                atomic_write_json_value(&shaping_inputs_path, &shaping_inputs_value).with_context(
                    || {
                        format!(
                            "Unable to publish runtime shaping inputs at {:?}",
                            shaping_inputs_path
                        )
                    },
                )?;
            }
            // The content-only digest of the artifact-of-record: custody Leg 2's
            // lag tier recomputes it from the on-disk artifact, so a
            // load->edit->save content TAMPER is caught even on a churned world.
            let content_generation = shaping_inputs.compute_shaping_generation_with_export(None)?;
            publish_topology_runtime_status(
                config,
                source_generation,
                Some(&shaping_inputs.shaping_generation),
                Some(content_generation.as_str()),
                Some(effective_generation.as_str()),
                true,
                None,
            )?;
        }
        None => {
            if shaping_inputs_path.exists() {
                std::fs::remove_file(&shaping_inputs_path).with_context(|| {
                    format!(
                        "Unable to remove stale runtime shaping inputs at {:?}",
                        shaping_inputs_path
                    )
                })?;
            }
            publish_topology_runtime_status(
                config,
                source_generation,
                None,
                None,
                Some(effective_generation.as_str()),
                false,
                Some("Topology runtime did not produce shaping inputs.".to_string()),
            )?;
        }
    }

    Ok(())
}

fn topology_runtime_status_snapshot(
    config: &Config,
    source_generation: &str,
    shaping_generation: Option<&str>,
    shaping_content_generation: Option<&str>,
    effective_generation: Option<&str>,
    ready: bool,
    error: Option<String>,
) -> TopologyRuntimeStatusFile {
    let operator_view_path = topology_operator_view_path(config)
        .to_string_lossy()
        .to_string();
    TopologyRuntimeStatusFile {
        schema_version: 1,
        source_generation: source_generation.to_string(),
        shaping_generation: shaping_generation.unwrap_or_default().to_string(),
        shaping_content_generation: shaping_content_generation.unwrap_or_default().to_string(),
        effective_generation: effective_generation.unwrap_or_default().to_string(),
        ready,
        generated_unix: now_unix(),
        effective_state_path: topology_effective_state_path(config)
            .to_string_lossy()
            .to_string(),
        // Phase A §2 rename: the operator view is the current name. The
        // deprecated `effective_network_path` is dual-written for one release
        // so older readers keep working.
        operator_view_path: operator_view_path.clone(),
        effective_network_path: operator_view_path,
        shaping_inputs_path: topology_shaping_inputs_path(config)
            .to_string_lossy()
            .to_string(),
        error,
    }
}

/// Publishes topology runtime readiness for one source generation.
///
/// Side effects: writes `topology_runtime_status.json` in `config.lqos_directory`.
pub fn publish_topology_runtime_status(
    config: &Config,
    source_generation: &str,
    shaping_generation: Option<&str>,
    shaping_content_generation: Option<&str>,
    effective_generation: Option<&str>,
    ready: bool,
    error: Option<String>,
) -> Result<()> {
    let status = topology_runtime_status_snapshot(
        config,
        source_generation,
        shaping_generation,
        shaping_content_generation,
        effective_generation,
        ready,
        error,
    );
    if TopologyRuntimeStatusFile::load(config)
        .ok()
        .as_ref()
        .is_some_and(|current| current.semantic_equals_for_publish(&status))
    {
        return Ok(());
    }
    status.save(config).with_context(|| {
        format!(
            "Unable to publish topology runtime status at {:?}",
            topology_runtime_status_path(config)
        )
    })?;
    Ok(())
}

/// Publishes a failed topology runtime status for one source generation.
///
/// Side effects: writes `topology_runtime_status.json` in `config.lqos_directory`.
pub fn publish_topology_runtime_error_status(
    config: &Config,
    source_generation: &str,
    error: &str,
) -> Result<()> {
    publish_topology_runtime_status(
        config,
        source_generation,
        None,
        None,
        None,
        false,
        Some(error.to_string()),
    )
}
