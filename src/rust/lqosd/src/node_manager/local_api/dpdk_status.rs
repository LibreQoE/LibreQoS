//! Read-only DPDK dataplane status assembly for the node manager.
//!
//! This surface is **control-plane only and read-only**: it never writes
//! configuration and never touches the dataplane. It assembles the fields the
//! Tuning → DPDK Dataplane panel renders from [`/proc/meminfo`],
//! `/etc/libreqos/bridge.env` + `/etc/libreqos/bridge.env.d/*.conf`, the
//! published `dataplane_transaction.json`, and the bridge's applied-state report
//! (the typed `bridge.conf` no longer carries an operator capacity key).
//!
//! Every field carries an explicit [`DpdkStatusKind`] so the UI can label it
//! honestly:
//!
//! * [`DpdkStatusKind::Running`] — live system truth (hugepages).
//! * [`DpdkStatusKind::Effective`] — the post-adjust value the PMD actually
//!   applied. When it is not readable from the control plane the value is left
//!   empty and a `note` explains why (honest-empty, never a fake value).
//! * [`DpdkStatusKind::Configured`] — the requested/derived configuration.
//!
//! The pure [`assemble`] function takes a [`DpdkStatusSources`] snapshot so the
//! field map and honest-empty behaviour are unit-testable without touching the
//! host filesystem.

use crate::node_manager::auth::LoginResult;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Where a rendered DPDK status value comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DpdkStatusKind {
    /// Live system truth (e.g. kernel hugepage counters).
    Running,
    /// The post-adjust value the device actually applied.
    Effective,
    /// The requested or derived configuration.
    Configured,
}

/// One read-only row of the DPDK status surface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DpdkStatusField {
    /// Stable field identifier (UI key).
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Display value; `None` when the source is absent (render as "Unavailable").
    pub value: Option<String>,
    /// Provenance class of the value.
    pub kind: DpdkStatusKind,
    /// Short description of the authoritative source.
    pub source: String,
    /// Optional caveat, e.g. why an effective value is unavailable.
    pub note: Option<String>,
}

/// The assembled read-only DPDK status payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DpdkStatus {
    /// Field rows, in display order.
    pub fields: Vec<DpdkStatusField>,
    /// Unix seconds when the snapshot was assembled.
    pub generated_at_unix: u64,
}

/// A filesystem/env snapshot the assembler reads. Constructed from the live
/// host by [`collect_dpdk_status`], or literally in tests.
#[derive(Clone, Debug, Default)]
pub struct DpdkStatusSources {
    /// Contents of `/proc/meminfo`, when readable.
    pub meminfo: Option<String>,
    /// Merged `bridge.env` + `bridge.env.d/*.conf` keys (later files win),
    /// with process-env fallback already folded in by the caller.
    pub bridge_env: BTreeMap<String, String>,
    /// §8: the DERIVED circuit capacity stamped on the published desired
    /// `dataplane_transaction.json`, when readable. The typed `bridge.conf`
    /// `circuit_capacity` key and the `LIBREQOS_DPDK_CIRCUIT_CAPACITY` env were
    /// deleted with the S8 fix; the transaction is the ONE source.
    pub desired_circuit_capacity: Option<u64>,
    /// §8: the bridge's last REPORTED applied capacity
    /// (`analytics_ipc::StateReport::applied_circuit_capacity`), noted by the
    /// broker on adopt. `None` = no report adopted yet.
    pub applied_circuit_capacity: Option<u64>,
}

/// Assemble the read-only DPDK status from a source snapshot.
pub fn assemble(sources: &DpdkStatusSources) -> DpdkStatus {
    let eal = eal_tokens(&sources.bridge_env);

    let mut fields = Vec::with_capacity(12);
    fields.push(hugepages_field(sources.meminfo.as_deref()));
    fields.push(pmd_path_field(&sources.bridge_env, &eal));
    fields.push(numa_field(&sources.bridge_env, &eal));
    fields.push(queues_field(&sources.bridge_env));
    fields.push(descriptors_field());
    fields.push(cake_arena_field());
    fields.push(circuit_capacity_field(sources));
    fields.push(worker_field(&sources.bridge_env));
    fields.extend(dynamic_cake_fields(&sources.bridge_env));
    fields.extend(completion_feed_fields());

    DpdkStatus {
        fields,
        generated_at_unix: now_unix(),
    }
}

/// The lqosd completion-feed consumer health (`lqos-74t.2.x` Q5): surfaced
/// read-only so the operator sees drained/exported/dropped/replay-lost/
/// overlap/failed-side-loss/lag + the degraded latch (the card §2.6 requirement).
fn with_degraded_note(mut field: DpdkStatusField, degraded: bool) -> DpdkStatusField {
    if degraded {
        field.note = Some(
            "DEGRADED: see completion_loss / completion_replay_lost / \
             completion_failed_side_loss (the feed is not healthy)"
                .to_string(),
        );
    }
    field
}

fn completion_feed_fields() -> Vec<DpdkStatusField> {
    let h = crate::bridge_analytics::completion_consumer::health();
    let row = |id: &str, label: &str, value: String| DpdkStatusField {
        id: id.to_string(),
        label: label.to_string(),
        value: Some(value),
        kind: DpdkStatusKind::Running,
        source: "completion_consumer::health() (lqosd)".to_string(),
        note: None,
    };
    vec![
        row("completion_ring_active", "Completion ring active", h.ring_active.to_string()),
        row("completion_handed_over", "Completion handover", h.handed_over.to_string()),
        row("completion_drained", "Completions drained", h.drained.to_string()),
        row("completion_exported", "Completions exported", h.exported.to_string()),
        row("completion_loss", "Completion loss", h.completion_loss.to_string()),
        row(
            "completion_overwritten",
            "Completion overwritten (informational)",
            h.overwritten.to_string(),
        ),
        row("completion_replay_lost", "Completions replay-lost", h.replay_lost.to_string()),
        row("completion_overlap", "Completion handover overlap", h.overlap.to_string()),
        row("completion_failed_side_loss", "Completion failed-side loss", h.failed_side_loss.to_string()),
        with_degraded_note(row("completion_degraded", "Completion feed degraded", h.degraded.to_string()), h.degraded),
    ]
}

/// Read the live host sources and assemble the status snapshot.
/// Side effects: reads `/proc/meminfo`, `/etc/libreqos/bridge.env(.d)`, the
/// configured `bridge.conf`, and a fixed set of process-environment fallbacks.
/// Never writes anything.
pub fn collect_dpdk_status() -> DpdkStatus {
    assemble(&read_sources())
}

/// Assemble for the node-manager websocket handler. Admin-only.
pub fn dpdk_status_data(login: LoginResult) -> Result<DpdkStatus, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(collect_dpdk_status())
}

// --- live source reading -----------------------------------------------------

const BRIDGE_ENV_PATH: &str = "/etc/libreqos/bridge.env";
const BRIDGE_ENV_DIR: &str = "/etc/libreqos/bridge.env.d";
const DEFAULT_PMD_PATH: &str = "/usr/libexec/libreqos/pmds";

const ENV_KEYS_WITH_FALLBACK: &[&str] = &[
    "LIBREQOS_DPDK_WORKERS",
    "LIBREQOS_DPDK_RX_QUEUES",
    "LIBREQOS_DPDK_TX_QUEUES",
    "LIBREQOS_DPDK_EAL_ARGS",
    "LIBREQOS_DPDK_DYNAMIC_CAKE",
    "LIBREQOS_DPDK_DYNAMIC_CAKE_TICK_MS",
    "LIBREQOS_DPDK_DYNAMIC_CAKE_DISARM_QUIET_MS",
    "LIBREQOS_DPDK_DYNAMIC_CAKE_FLIP_BUDGET",
];

fn read_sources() -> DpdkStatusSources {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok();
    let bridge_env = read_bridge_env();

    DpdkStatusSources {
        meminfo,
        bridge_env,
        desired_circuit_capacity: desired_circuit_capacity_from_tx(),
        applied_circuit_capacity: lqos_topology::applied_circuit_capacity().map(u64::from),
    }
}

/// Read `bridge.env` + its sorted `bridge.env.d/*.conf` drop-ins, folding in the
/// process env only for keys the files did not carry. One reader for both the
/// DPDK status panel and the CPU/Workers `autobalance.configured` field.
fn read_bridge_env() -> BTreeMap<String, String> {
    let mut bridge_env = BTreeMap::new();
    if let Ok(contents) = std::fs::read_to_string(BRIDGE_ENV_PATH) {
        merge_env_file(&mut bridge_env, &contents);
    }
    if let Ok(entries) = std::fs::read_dir(BRIDGE_ENV_DIR) {
        let mut paths: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        paths.sort();
        for path in paths {
            if path.extension().and_then(|ext| ext.to_str()) == Some("conf")
                && let Ok(contents) = std::fs::read_to_string(&path)
            {
                merge_env_file(&mut bridge_env, &contents);
            }
        }
    }
    // libreqos-lqosd.service also sources bridge.env; fold the process env in
    // only for keys the files did not carry.
    for key in ENV_KEYS_WITH_FALLBACK {
        if !bridge_env.contains_key(*key)
            && let Ok(value) = std::env::var(key)
            && !value.is_empty()
        {
            bridge_env.insert((*key).to_string(), value);
        }
    }
    bridge_env
}

/// `lqos-74t.118.2`: the CONFIGURED bridge autobalance posture from
/// `bridge.env`. `None` = the key is absent/unparseable; runtime ACTIVE state is
/// not exposed by the bridge (log-only), so the API reports `runtime: null`.
pub(crate) fn configured_autobalance() -> Option<bool> {
    const KEY: &str = "LIBREQOS_DPDK_PLACEMENT_AUTOBALANCE";
    let value = read_bridge_env()
        .get(KEY)
        .cloned()
        .or_else(|| std::env::var(KEY).ok())?;
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// §8: the DERIVED capacity the emitter stamped on the published desired
/// transaction (`dataplane_transaction.json`). Read-only; `None` before the
/// first publish.
fn desired_circuit_capacity_from_tx() -> Option<u64> {
    let config = lqos_config::load_config().ok()?;
    let path = lqos_config::topology_dataplane_transaction_path(&config);
    let bytes = std::fs::read(path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("circuit_capacity")
        .and_then(serde_json::Value::as_u64)
}

fn merge_env_file(map: &mut BTreeMap<String, String>, contents: &str) {
    for raw in contents.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            if !key.is_empty() {
                map.insert(key.to_string(), unquote(value.trim()));
            }
        }
    }
}

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();
    if trimmed.len() >= 2
        && ((bytes[0] == b'"' && bytes[trimmed.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[trimmed.len() - 1] == b'\''))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

// --- field assembly ----------------------------------------------------------

fn field(
    id: &str,
    label: &str,
    value: Option<String>,
    kind: DpdkStatusKind,
    source: &str,
    note: Option<&str>,
) -> DpdkStatusField {
    DpdkStatusField {
        id: id.to_string(),
        label: label.to_string(),
        value,
        kind,
        source: source.to_string(),
        note: note.map(str::to_string),
    }
}

fn hugepages_field(meminfo: Option<&str>) -> DpdkStatusField {
    let parsed = meminfo.and_then(parse_hugepages);
    let note = parsed
        .is_none()
        .then_some("Could not read hugepage counters from /proc/meminfo.");
    field(
        "hugepages",
        "Hugepages",
        parsed,
        DpdkStatusKind::Running,
        "/proc/meminfo (HugePages_Total / HugePages_Free)",
        note,
    )
}

fn parse_hugepages(meminfo: &str) -> Option<String> {
    let mut total = None;
    let mut free = None;
    let mut size_kb = None;
    for line in meminfo.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let first = value
            .split_whitespace()
            .next()
            .and_then(|v| v.parse::<u64>().ok());
        match key {
            "HugePages_Total" => total = first,
            "HugePages_Free" => free = first,
            "Hugepagesize" => size_kb = first,
            _ => {}
        }
    }
    let total = total?;
    let free = free?;
    match size_kb {
        Some(size) => Some(format!("{total} total, {free} free ({size} KiB pages)")),
        None => Some(format!("{total} total, {free} free")),
    }
}

fn pmd_path_field(env: &BTreeMap<String, String>, eal: &[String]) -> DpdkStatusField {
    let from_eal = eal_flag_value(eal, "-d");
    let value = from_eal
        .or_else(|| env.get("RTE_EAL_PMD_PATH").cloned())
        .unwrap_or_else(|| DEFAULT_PMD_PATH.to_string());
    field(
        "pmd_path",
        "PMD path",
        Some(value),
        DpdkStatusKind::Configured,
        "bridge.env (LIBREQOS_DPDK_EAL_ARGS -d)",
        None,
    )
}

fn numa_field(_env: &BTreeMap<String, String>, eal: &[String]) -> DpdkStatusField {
    let mut found = Vec::new();
    for flag in ["--socket-mem", "--socket-limit", "--lcores", "-l"] {
        if let Some(value) = eal_flag_value(eal, flag) {
            found.push(format!("{flag} {value}"));
        } else if eal.iter().any(|token| token == flag) {
            found.push(flag.to_string());
        }
    }
    let (value, note) = if found.is_empty() {
        (
            None,
            Some("No NUMA/socket flags in the EAL arguments; EAL defaults apply."),
        )
    } else {
        (Some(found.join(", ")), None)
    };
    field(
        "numa",
        "NUMA / socket layout",
        value,
        DpdkStatusKind::Configured,
        "bridge.env (LIBREQOS_DPDK_EAL_ARGS)",
        note,
    )
}

fn queues_field(env: &BTreeMap<String, String>) -> DpdkStatusField {
    let rx = env.get("LIBREQOS_DPDK_RX_QUEUES").filter(|v| !v.is_empty());
    let tx = env.get("LIBREQOS_DPDK_TX_QUEUES").filter(|v| !v.is_empty());
    let workers = env.get("LIBREQOS_DPDK_WORKERS").filter(|v| !v.is_empty());
    let (value, note) = match (rx, tx) {
        (Some(rx), Some(tx)) => (Some(format!("{rx} RX / {tx} TX")), None),
        _ => match workers {
            Some(workers) => (
                Some(format!("{workers} RX / {workers} TX (defaults to workers)")),
                None,
            ),
            None => (
                None,
                Some(
                    "LIBREQOS_DPDK_RX_QUEUES / TX_QUEUES unset; bridge_wrapper defaults them to the worker count.",
                ),
            ),
        },
    };
    field(
        "queues",
        "RX / TX queues",
        value,
        DpdkStatusKind::Configured,
        "bridge.env.d via bridge_wrapper (LIBREQOS_DPDK_RX_QUEUES / TX_QUEUES)",
        note,
    )
}

fn descriptors_field() -> DpdkStatusField {
    field(
        "descriptors",
        "RX / TX descriptors",
        None,
        DpdkStatusKind::Effective,
        "bridge_launch (hard-coded 1024 / 1024, then PMD-clamped)",
        Some(
            "Configured as 1024 / 1024 at launch, then clamped to the port maximum by \
             rte_eth_dev_adjust_nb_rx_tx_desc. The effective post-clamp value is only \
             known to the running bridge and is not exposed to the control plane.",
        ),
    )
}

fn cake_arena_field() -> DpdkStatusField {
    // NOTE: this is the compiled engine-default constant, not a live read of a
    // configured `CakeConfig` (there is no operator override at HEAD). It will
    // not track a change to the compiled default; it is reported as CONFIGURED
    // engine-default-constant, never as a live capability read.
    field(
        "cake_arena",
        "CAKE arena (max_packets_per_tin)",
        Some("2048 packets (engine default; per-tin 0 → arena size)".to_string()),
        DpdkStatusKind::Configured,
        "cake_core::CakeConfig compiled constant (max_packets = 2048; not read live)",
        Some(
            "The per-tin cap defaults to 0, which resolves to the arena size (max_packets = 2048). This is a compiled engine default, not a live capability read.",
        ),
    )
}

fn circuit_capacity_field(sources: &DpdkStatusSources) -> DpdkStatusField {
    // §8 (`docs/lqos_conf_single_source_card.md`): `circuit_capacity` is DERIVED
    // by the emitter and threaded on the desired transaction; the bridge REPORTS
    // its applied value (`StateReport.applied_circuit_capacity`). The typed
    // `bridge.conf` key and the `LIBREQOS_DPDK_CIRCUIT_CAPACITY` env were
    // DELETED — never render their old default (17).
    match (
        sources.applied_circuit_capacity,
        sources.desired_circuit_capacity,
    ) {
        (Some(applied), Some(desired)) if applied != desired => field(
            "circuit_capacity",
            "Circuit capacity",
            Some(applied.to_string()),
            DpdkStatusKind::Effective,
            "StateReport.applied_circuit_capacity (bridge-reported applied)",
            Some(&format!(
                "The desired transaction derives {desired}; the bridge reports {applied} applied \
                 (the world grows on the next transaction)."
            )),
        ),
        (Some(applied), _) => field(
            "circuit_capacity",
            "Circuit capacity",
            Some(applied.to_string()),
            DpdkStatusKind::Effective,
            "StateReport.applied_circuit_capacity (bridge-reported applied)",
            None,
        ),
        (None, Some(desired)) => field(
            "circuit_capacity",
            "Circuit capacity",
            Some(desired.to_string()),
            DpdkStatusKind::Configured,
            "dataplane_transaction.json circuit_capacity (derived at emit)",
            Some("No applied report adopted yet; showing the derived desired capacity."),
        ),
        (None, None) => field(
            "circuit_capacity",
            "Circuit capacity",
            None,
            DpdkStatusKind::Effective,
            "dataplane_transaction.json / StateReport.applied_circuit_capacity",
            Some(
                "No transaction published yet; the capacity is DERIVED by the emitter \
                 (never an operator key).",
            ),
        ),
    }
}

fn worker_field(env: &BTreeMap<String, String>) -> DpdkStatusField {
    let value = env
        .get("LIBREQOS_DPDK_WORKERS")
        .filter(|v| !v.is_empty())
        .cloned();
    // The editable `[queues] workers` is rendered into this environment by
    // `lqos_setup` (the installer / a port re-bind), NOT at runtime, so the
    // boot value here and a fresh UI edit can legitimately differ until then.
    let note = match &value {
        Some(_) => {
            "Boot value from the bridge environment. The editable `[queues] workers` is rendered into it by `lqos_setup` (a port re-bind), not at runtime."
        }
        None => {
            "Unset in the bridge environment; `lqos_setup` derives it. The editable `[queues] workers` is rendered here by the installer, not at runtime."
        }
    };
    field(
        "workers",
        "Worker count (boot env — LIBREQOS_DPDK_WORKERS)",
        value,
        DpdkStatusKind::Configured,
        "bridge.env.d via bridge_wrapper (LIBREQOS_DPDK_WORKERS)",
        Some(note),
    )
}

fn dynamic_cake_fields(env: &BTreeMap<String, String>) -> Vec<DpdkStatusField> {
    let source = "bridge.env / engine default (LIBREQOS_DPDK_DYNAMIC_CAKE*)";
    let enabled = env
        .get("LIBREQOS_DPDK_DYNAMIC_CAKE")
        .map(|v| parse_env_bool(v, true))
        .unwrap_or(true);
    let tick = env
        .get("LIBREQOS_DPDK_DYNAMIC_CAKE_TICK_MS")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(1_000)
        .max(1);
    let quiet = env
        .get("LIBREQOS_DPDK_DYNAMIC_CAKE_DISARM_QUIET_MS")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(120_000);
    let flip = env
        .get("LIBREQOS_DPDK_DYNAMIC_CAKE_FLIP_BUDGET")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(256)
        .max(1);
    vec![
        field(
            "dynamic_cake_enabled",
            "Dynamic CAKE",
            Some(if enabled { "Enabled" } else { "Disabled" }.to_string()),
            DpdkStatusKind::Configured,
            source,
            None,
        ),
        field(
            "dynamic_cake_tick_ms",
            "Dynamic CAKE tick",
            Some(format!("{tick} ms")),
            DpdkStatusKind::Configured,
            source,
            None,
        ),
        field(
            "dynamic_cake_quiet_ms",
            "Dynamic CAKE disarm quiet window",
            Some(format!("{quiet} ms")),
            DpdkStatusKind::Configured,
            source,
            None,
        ),
        field(
            "dynamic_cake_flip_budget",
            "Dynamic CAKE flip budget",
            Some(format!("{flip} per direction per tick")),
            DpdkStatusKind::Configured,
            source,
            None,
        ),
    ]
}

fn parse_env_bool(raw: &str, default: bool) -> bool {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => default,
    }
}

// --- EAL argument helpers ----------------------------------------------------

fn eal_tokens(env: &BTreeMap<String, String>) -> Vec<String> {
    env.get("LIBREQOS_DPDK_EAL_ARGS")
        .map(|raw| raw.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

fn eal_flag_value(tokens: &[String], flag: &str) -> Option<String> {
    let mut iter = tokens.iter();
    while let Some(token) = iter.next() {
        if token == flag {
            return iter.next().cloned();
        }
        if let Some(rest) = token.strip_prefix(flag)
            && !rest.is_empty()
            && !rest.starts_with('-')
        {
            return Some(rest.to_string());
        }
    }
    None
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn find<'a>(status: &'a DpdkStatus, id: &str) -> &'a DpdkStatusField {
        status
            .fields
            .iter()
            .find(|f| f.id == id)
            .unwrap_or_else(|| panic!("missing field {id}"))
    }

    #[test]
    fn field_map_is_complete_and_kinded() {
        let sources = DpdkStatusSources {
            meminfo: Some(
                "HugePages_Total: 1024\nHugePages_Free: 512\nHugepagesize: 2048 kB\n".to_string(),
            ),
            bridge_env: env(&[
                (
                    "LIBREQOS_DPDK_EAL_ARGS",
                    "-d /usr/libexec/libreqos/pmds --socket-mem 1024",
                ),
                ("LIBREQOS_DPDK_WORKERS", "4"),
                ("LIBREQOS_DPDK_RX_QUEUES", "4"),
                ("LIBREQOS_DPDK_TX_QUEUES", "4"),
            ]),
            // R2b: `bridge.conf`/env are no longer a capacity source; only the
            // derived desired + the reported applied capacity.
            desired_circuit_capacity: Some(4096),
            applied_circuit_capacity: None,
        };
        let status = assemble(&sources);

        let ids: Vec<&str> = status.fields.iter().map(|f| f.id.as_str()).collect();
        for required in [
            "hugepages",
            "pmd_path",
            "numa",
            "queues",
            "descriptors",
            "cake_arena",
            "circuit_capacity",
            "workers",
            "dynamic_cake_enabled",
            "dynamic_cake_tick_ms",
            "dynamic_cake_quiet_ms",
            "dynamic_cake_flip_budget",
        ] {
            assert!(ids.contains(&required), "missing {required}: {ids:?}");
        }

        assert_eq!(find(&status, "hugepages").kind, DpdkStatusKind::Running);
        assert_eq!(
            find(&status, "hugepages").value.as_deref(),
            Some("1024 total, 512 free (2048 KiB pages)")
        );
        assert_eq!(
            find(&status, "pmd_path").value.as_deref(),
            Some("/usr/libexec/libreqos/pmds")
        );
        assert_eq!(
            find(&status, "numa").value.as_deref(),
            Some("--socket-mem 1024")
        );
        assert_eq!(
            find(&status, "queues").value.as_deref(),
            Some("4 RX / 4 TX")
        );
        // R2b: the derived desired value is shown (no bridge.conf/env source).
        assert_eq!(
            find(&status, "circuit_capacity").value.as_deref(),
            Some("4096")
        );
        assert_eq!(
            find(&status, "circuit_capacity").kind,
            DpdkStatusKind::Configured
        );
        assert_eq!(find(&status, "workers").value.as_deref(), Some("4"));
        assert_eq!(find(&status, "descriptors").kind, DpdkStatusKind::Effective);
        assert_eq!(find(&status, "cake_arena").kind, DpdkStatusKind::Configured);
    }

    /// R2b: the DPDK-status capacity shows the bridge's REPORTED applied value
    /// (Effective), falls back to the DERIVED desired (Configured), and is
    /// honest-empty when neither exists — never the deleted env/17.
    #[test]
    fn circuit_capacity_reports_applied_then_derived_honestly() {
        let applied = assemble(&DpdkStatusSources {
            desired_circuit_capacity: Some(2048),
            applied_circuit_capacity: Some(4096),
            ..Default::default()
        });
        let field = find(&applied, "circuit_capacity");
        assert_eq!(field.value.as_deref(), Some("4096"));
        assert_eq!(field.kind, DpdkStatusKind::Effective);
        assert!(
            field.note.as_deref().is_some_and(|n| n.contains("2048")),
            "the mismatch note must name the derived desired: {:?}",
            field.note
        );

        let desired = assemble(&DpdkStatusSources {
            desired_circuit_capacity: Some(1024),
            applied_circuit_capacity: None,
            ..Default::default()
        });
        let field = find(&desired, "circuit_capacity");
        assert_eq!(field.value.as_deref(), Some("1024"));
        assert_eq!(field.kind, DpdkStatusKind::Configured);
    }

    #[test]
    fn descriptors_are_honest_empty_effective() {
        let status = assemble(&DpdkStatusSources::default());
        let descriptors = find(&status, "descriptors");
        assert_eq!(descriptors.kind, DpdkStatusKind::Effective);
        assert_eq!(
            descriptors.value, None,
            "effective descriptors must not be faked"
        );
        assert!(
            descriptors
                .note
                .as_deref()
                .is_some_and(|n| n.contains("1024 / 1024"))
        );
    }

    #[test]
    fn absent_sources_are_labeled_not_faked() {
        let status = assemble(&DpdkStatusSources::default());
        let hugepages = find(&status, "hugepages");
        assert_eq!(hugepages.kind, DpdkStatusKind::Running);
        assert_eq!(hugepages.value, None);
        assert!(hugepages.note.is_some());

        let queues = find(&status, "queues");
        assert_eq!(queues.value, None);
        assert!(queues.note.is_some());

        let numa = find(&status, "numa");
        assert_eq!(numa.value, None);
        assert!(numa.note.is_some());
    }

    #[test]
    fn defaults_are_reported_when_bridge_sources_are_absent() {
        let status = assemble(&DpdkStatusSources::default());
        assert_eq!(
            find(&status, "pmd_path").value.as_deref(),
            Some(DEFAULT_PMD_PATH)
        );
        // R2b: no transaction + no report -> honest-empty, NEVER the deleted
        // compiled default (17).
        let capacity = find(&status, "circuit_capacity");
        assert_eq!(capacity.value, None, "capacity must be honest-empty");
        assert_eq!(capacity.kind, DpdkStatusKind::Effective);
        assert!(
            capacity
                .note
                .as_deref()
                .is_some_and(|note| note.contains("DERIVED")),
            "the note must name the derived source: {:?}",
            capacity.note
        );
        assert_eq!(
            find(&status, "dynamic_cake_enabled").value.as_deref(),
            Some("Enabled")
        );
        assert_eq!(
            find(&status, "dynamic_cake_tick_ms").value.as_deref(),
            Some("1000 ms")
        );
        assert_eq!(
            find(&status, "dynamic_cake_quiet_ms").value.as_deref(),
            Some("120000 ms")
        );
        assert_eq!(
            find(&status, "dynamic_cake_flip_budget").value.as_deref(),
            Some("256 per direction per tick")
        );
    }

    #[test]
    fn dynamic_cake_env_overrides() {
        let sources = DpdkStatusSources {
            bridge_env: env(&[
                ("LIBREQOS_DPDK_DYNAMIC_CAKE", "0"),
                ("LIBREQOS_DPDK_DYNAMIC_CAKE_TICK_MS", "500"),
                ("LIBREQOS_DPDK_DYNAMIC_CAKE_DISARM_QUIET_MS", "60000"),
                ("LIBREQOS_DPDK_DYNAMIC_CAKE_FLIP_BUDGET", "64"),
            ]),
            ..Default::default()
        };
        let status = assemble(&sources);
        assert_eq!(
            find(&status, "dynamic_cake_enabled").value.as_deref(),
            Some("Disabled")
        );
        assert_eq!(
            find(&status, "dynamic_cake_tick_ms").value.as_deref(),
            Some("500 ms")
        );
        assert_eq!(
            find(&status, "dynamic_cake_quiet_ms").value.as_deref(),
            Some("60000 ms")
        );
        assert_eq!(
            find(&status, "dynamic_cake_flip_budget").value.as_deref(),
            Some("64 per direction per tick")
        );
    }

    #[test]
    fn env_file_parsing_handles_comments_quotes_and_export() {
        let mut map = BTreeMap::new();
        merge_env_file(
            &mut map,
            "# comment\nexport LIBREQOS_DPDK_WORKERS=8\nLIBREQOS_DPDK_RX_QUEUES=\"4\" # trailing\nBAD LINE\n",
        );
        assert_eq!(
            map.get("LIBREQOS_DPDK_WORKERS").map(String::as_str),
            Some("8")
        );
        assert_eq!(
            map.get("LIBREQOS_DPDK_RX_QUEUES").map(String::as_str),
            Some("4")
        );
        assert!(!map.contains_key("BAD LINE"));
    }

    #[test]
    fn eal_flag_value_reads_attached_and_detached_forms() {
        assert_eq!(
            eal_flag_value(&["-d".into(), "/pmds".into()], "-d").as_deref(),
            Some("/pmds")
        );
        assert_eq!(
            eal_flag_value(&["-d/pmds".into()], "-d").as_deref(),
            Some("/pmds")
        );
        assert_eq!(
            eal_flag_value(&["--lcores".into(), "0-3".into()], "--lcores").as_deref(),
            Some("0-3")
        );
        assert_eq!(
            eal_flag_value(&["-l".into(), "0-3".into()], "-l").as_deref(),
            Some("0-3")
        );
    }
}
