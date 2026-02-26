# NLNET Milestone 6 Plan — Intelligent Node Management (TreeGuard)

TreeGuard was previously referred to as “Autopilot” in early drafts.

Let's plan out a large feature.

My requirements:
* All new work MUST occur in a new branch, `intelligence_node_management`.
* Favor creating a new module `treeguard` inside `lqosd`.
* All functions must have RustDoc headers (without examples).
    * All RustDoc must document what a function does.
    * Function comments must indicate if a function is pure or not, and what side-effects it has.
    * If possible, constify pure functions.
* Prefer actors over mutexes.
* Prefer "let else" over nested "if let", and exit functions early after checking preconditions.
* Functions should be written defensively, validating inputs.
* Use custom `thiserror` implementations rather than `anyhow`.
* Explicitly handle failure cases and target keeping the system running.

## Deliverable (this file)

This document is a **decision-focused spec** for implementing NLNet Milestone 6, starting from the current state of the `develop` branch.

It also includes:
- an **open questions** section to confirm remaining decisions before coding, and
- a **junior-friendly implementation checklist** at the end.

## NLNet Requirement

This is an NLNet funded feature, so it's important we meet their stated goals:

Task: 6. Intelligent Node Management € 3 120

Milestone:
a. Monitor load levels and round-trip times of tracked backhaul links.
€ 720

Milestone:
b. If enabled, the monitor could “virtualize” a link that isn’t close to congested - and “unvirtualize” it if load becomes high and/or round-trip times begin to suffer.
€ 800

Milestone:
c. Similarly, customer circuits could utilize a low-impact shaping scheme when mostly idle and ramp up shaping as demand increases.
€ 800

Milestone:
d. The goal here is to tame CPU usage, making it easier to deploy LibreQoS on inexpensive hardware.

---

## Scope & goals (what we are building)

We are specifying a TreeGuard subsystem for **intelligent node management** that:

1) **Monitors** (milestone a)
- Load/utilization and RTT for operator-selected backhaul links.

2) **Virtualizes/unvirtualizes links** (milestone b)
- If enabled and safe, automatically mark selected `network.json` nodes as `virtual` (logical-only) to reduce shaping complexity/CPU.
- Automatically revert (unvirtualize) when load rises and/or RTT worsens.

3) **Adjusts per-circuit shaping** (milestone c)
- Use lower-impact shaping for CPU savings when safe.
- Chosen v1 direction: dynamic SQM switching (CAKE ↔ fq_codel) with guardrails and hysteresis.

4) **Tames CPU usage** (milestone d)
- Use CPU usage as an explicit control input: become more aggressive under CPU pressure and revert when CPU headroom returns.

### Explicitly out of scope
- Anything about the executive dashboard (it is already functional and live).
- Replacing the scheduler or rewriting the shaping pipeline.
- A “topology editor” beyond TreeGuard config + TreeGuard status/visibility.

---

## Glossary & definitions

- **Node / site**: a `network.json` entry.
- **Link**: the parent→child relationship in the `network.json` hierarchy. Practically, “virtualize a link” means “toggle `virtual` on the chosen node” so it’s omitted from the physical shaping tree.
- **Virtual node**: a `network.json` node marked as `virtual` / logical-only; it exists for monitoring/aggregation but is omitted from the physical HTB topology.
- **Basically idle**: very low utilization, sustained for at least **15 minutes** (sustained counters + smoothing; no flapping).
- **Backhaul link RTT (chosen for v1)**: passive RTT derived from Flowbee TCP timestamp RTT samples aggregated into the network tree (no ICMP probing required).

---

## Current-state integration points (develop branch)

We should leverage existing subsystems rather than reinvent:

- `lqosd` throughput monitor updates the in-memory network tree each tick:
  - per-node throughput and per-node RTT buffers
- Virtual nodes are already supported:
  - scheduler builds physical HTB tree omitting `virtual` nodes
- Overrides system exists:
  - `lqos_overrides.json` already supports `set_node_virtual` (scheduler applies it before shaping)
  - TreeGuard persists its changes via `lqos_overrides.treeguard.json` (merged with operator overrides when enabled)
- `queueingStructure.json` exists and is monitored:
  - contains circuit class ids (`class_major`, `up_class_major`, `class_minor`, parent handles, etc.)
- Bakery supports live updates:
  - `sqm_override` tokens include `"cake"`, `"fq_codel"`, `"none"`, and directional variants (`"down/up"`)
- `lqosd` already tracks CPU usage via a system stats actor.

---

## Hard requirement: persistence and ownership (avoid scheduler/TreeGuard fights)

### Persistence policy
Any TreeGuard decision that must survive scheduler runs MUST be persisted via **`lqos_overrides.treeguard.json`** (the TreeGuard overrides layer), not by directly editing `network.json` or `ShapedDevices.csv`.

This is required to prevent:
- scheduler “undoing” TreeGuard changes on the next run, and
- TreeGuard re-applying changes repeatedly (fights/flapping).

### Ownership policy (critical)
TreeGuard must not overwrite operator intent.

**Chosen v1 rule: explicit allowlists define TreeGuard ownership.**
- TreeGuard only reads/writes persistent changes for items in its allowlists.
- Operators should not manually edit TreeGuard overrides for allowlisted items. To regain manual control, remove the item from the allowlist (or disable TreeGuard).
- When an item is removed from an allowlist (or TreeGuard is disabled), TreeGuard must remove/undo its persisted changes for that item during reconciliation.

**No overrides-format changes in v1.**
- We will not add new keys/fields to `lqos_overrides.json` entries for ownership metadata.
- We will solve “scheduler/TreeGuard fights” by ensuring persistence happens via the overrides system, and by restricting TreeGuard control to explicit allowlists.

### Persistence matrix (what persists, how)

| TreeGuard action | Needs persistence? | Persistence mechanism |
|---|---:|---|
| Mark node virtual/unvirtual | Yes | `lqos_overrides.treeguard.json` network adjustment `set_node_virtual` (treeguard-owned) |
| Circuit SQM CAKE↔fq_codel switch | Yes (to avoid fights) | `lqos_overrides.treeguard.json` persistent device overlays that set `sqm_override` by `device_id` (scheduler applies overlays without replacing integration data) |
| Live apply of SQM switch | No (runtime) | Bakery live update (but must be consistent with persisted override) |
| Pure monitoring/reporting | No | N/A |

---

## Configuration (new config section + node manager config page)

Add a top-level config section: `[treeguard]`.

**Defaults (chosen for safety):**
- TreeGuard disabled by default.
- Enrollment default: **none** (explicit allowlist required).
- Dry-run enabled by default.
- CPU-aware behavior enabled by default.

**Chosen v1 enrollment model: explicit allowlists by default.**
- The config supports explicit `nodes = [...]` / `circuits = [...]` allowlists, plus convenience toggles:
  - `treeguard.links.all_nodes = true` (“all links”)
  - `treeguard.circuits.all_circuits = true` (“all circuits”)

### Proposed TOML shape (v1)
```toml
[treeguard]
enabled = false
dry_run = true
tick_seconds = 1

[treeguard.cpu]
mode = "cpu_aware"          # cpu_aware|traffic_rtt_only
cpu_high_pct = 75           # max CPU% to start CPU-saving actions
cpu_low_pct  = 55           # max CPU% to revert actions

[treeguard.links]
enabled = true
all_nodes = false               # if true, manage all non-root nodes in network.json
nodes = []                      # network.json node names
idle_util_pct = 2.0             # "very low utilization" (starting default; tune in production)
idle_min_minutes = 15
rtt_missing_seconds = 120       # treat missing RTT as unsafe after 2 minutes
unvirtualize_util_pct = 5.0     # "traffic starts to tick up" (starting default; tune in production)
min_state_dwell_minutes = 30
max_link_changes_per_hour = 4
reload_cooldown_minutes = 10

[treeguard.circuits]
enabled = true
all_circuits = false            # if true, manage all circuits found in ShapedDevices.csv
circuits = []                   # circuit IDs (strings, as in ShapedDevices.csv)
switching_enabled = true
independent_directions = true   # allow different SQM decisions for down vs up (directional sqm_override)
idle_util_pct = 2.0             # "very low utilization" (starting default; tune in production)
idle_min_minutes = 15
rtt_missing_seconds = 120       # treat missing RTT as unsafe after 2 minutes
upgrade_util_pct = 5.0          # "traffic starts to tick up" (starting default; tune in production)
min_switch_dwell_minutes = 30
max_switches_per_hour = 4
persist_sqm_overrides = true    # MUST be true if we want to avoid scheduler fights

[treeguard.qoo]
enabled = true
min_score = 80.0                # 0..100; if QoO is available and below this, do not take CPU-saving actions
```

---

## Monitoring (milestone a)

### Link utilization
For each enrolled node:
- compute per-direction utilization as percent of capacity:
  - throughput from the in-memory `NetworkJson` tree
  - capacity from `network.json` (Mbps)
- if capacity is missing/zero, treat utilization as unknown and **do not make changes** (but surface a dashboard warning).

### Link RTT (passive, chosen for v1)
For each enrolled node:
- use per-direction p95 RTT from the in-memory `NetworkJsonNode` RTT buffer.
- if RTT samples are missing for >= `rtt_missing_seconds` (2 minutes), treat RTT as unknown and unsafe:
  - do not virtualize, and
  - if currently virtualized and the link is not sustained-idle, plan to unvirtualize (subject to rate limits/cooldowns).

### CPU usage (milestone d)
TreeGuard uses:
- **max CPU% across cores** (not average) as the primary control signal.

### Smoothing / stability
TreeGuard must maintain:
- an EWMA for utilization and RTT, and
- sustained-duration counters (>= 15 minutes) for “basically idle”.

---

## Link virtualization behavior (milestone b)

### Eligibility
- Only enrolled nodes are eligible (default none unless `all_nodes = true`).
- Nodes explicitly `virtual` in base `network.json` are treated as operator intent and are not flipped by TreeGuard in v1.
- Nodes already managed by non-TreeGuard operator workflows should not be allowlisted (allowlist defines ownership).

### Decision rules (v1)
Virtualize only when ALL are true:
- sustained idle for `idle_min_minutes`:
  - `max(util_ewma_down, util_ewma_up) < idle_util_pct`
- QoO healthy:
  - if QoO is available, QoO >= `treeguard.qoo.min_score` (80) for the relevant direction(s)
- CPU pressure:
  - `cpu_max_pct >= cpu_high_pct`
- dwell time + rate limits permit change

Unvirtualize when ANY are true (with short confirmation window to avoid oscillation):
- `max(util_ewma_down, util_ewma_up) >= unvirtualize_util_pct`
- if QoO is available, QoO < `treeguard.qoo.min_score` (80) for the relevant direction(s)
- RTT becomes unknown for >= `rtt_missing_seconds` (2 minutes) while not sustained-idle

### Persistence & actuation (required)
- Persist desired `virtual` flag via `lqos_overrides.treeguard.json` using `set_node_virtual` (treeguard-owned entry).
- Trigger a controlled reload to apply topology changes (rate-limited, with backoff on failure).

---

## Circuit shaping behavior (milestone c)

### Chosen behavior: dynamic SQM switching (CAKE ↔ fq_codel)
When mostly idle, TreeGuard may downgrade SQM to reduce CPU/RAM overhead when guardrails permit:
- CAKE → fq_codel

When demand increases (or guardrails indicate risk), revert:
- fq_codel → CAKE

### Directionality (chosen v1)
SQM decisions are made **independently by direction** (download vs upload). Persistence uses directional `sqm_override` tokens (e.g., `"cake/fq_codel"`).

### QoO guard (chosen v1)
If QoO scores are available (0..100), TreeGuard should avoid CPU-saving actions when QoO is poor:
- if QoO < `treeguard.qoo.min_score`, do not downgrade SQM and do not virtualize links
- if RTT becomes unknown for >= 2 minutes while not sustained-idle, revert to the safer state (CAKE, non-virtual)

### Guardrails & anti-flap
- QoO must be >= `treeguard.qoo.min_score` (80) to allow downgrade (when QoO is available).
- Per-circuit dwell time (`min_switch_dwell_minutes`).
- Global per-hour switch rate limit (`max_switches_per_hour`).

### Applying the change (runtime)
- Apply immediately via Bakery live update using class ids from `queueingStructure.json`.

### Persistence (required to avoid scheduler fights)
If `persist_sqm_overrides = true`:
- Persist the SQM change via the existing overrides file (without format changes):
  - TreeGuard writes SQM overrides via `lqos_overrides.treeguard.json` **persistent devices** keyed by `device_id`.
  - Scheduler applies those overrides as **overlays** (patching only the SQM token) so integration-provided circuit/device data remains authoritative.
- This ensures the next scheduler run (which rebuilds from `network.json`/`ShapedDevices.csv`) does not revert the SQM selection.

---

## Operator awareness (node manager dashboard elements)

TreeGuard must never be a “silent optimizer”.

### Required UI elements
1) **TreeGuard configuration page**
- enable/disable, dry-run
- per-feature toggles (links/circuits)
- enrollment editor and thresholds

2) **TreeGuard Status dashlet**
- clear “TreeGuard is ON/OFF/DRY-RUN” indicator
- current CPU max%
- counts: enrolled links/circuits, currently virtualized links, circuits in fq_codel mode
- “last action” summary (what changed, when, why)

3) **TreeGuard Activity / Audit dashlet**
- show recent actions (virtualize/unvirtualize, SQM changes, persistence writes, reloads)
- show whether each action was:
  - dry-run only, or
  - applied live only, or
  - persisted via overrides (and therefore durable)
- include visible warnings when TreeGuard is blocked by:
  - not allowlisted, missing RTT, missing capacity, reload backoff, etc.

### Required warnings/notifications
- Whenever TreeGuard performs a persistent change (writes overrides), it should generate a user-visible notification:
  - e.g., banner/toast in node manager, and/or an entry in the existing urgent issues feed.
- If TreeGuard detects a fight risk (manual override conflicts), it must surface a warning and stop touching that entity.

---

## Overrides usage for TreeGuard (no format changes)

### Link virtualization persistence
Use existing network adjustments:
- `NetworkAdjustment::SetNodeVirtual { node_name, virtual: bool }`

TreeGuard writes/updates these only for enrolled nodes.

### Circuit SQM persistence (without changing overrides format)
We will not extend the overrides schema with new adjustment variants.

Instead, TreeGuard persists SQM decisions using the existing `persistent_devices` list (type `ShapedDevice`), keyed by `device_id`, with `sqm_override` set to the desired token (including directional tokens).

**Scheduler behavior requirement (later implementation):**
- When applying `persistent_devices`, treat them as **field overlays** rather than full-row replacements:
  - patch only the SQM token for matching `device_id` rows (and do not overwrite integration-derived fields like bandwidth, parent, IPs, names).
- This preserves integration authority while still making SQM overrides durable across scheduler runs.

---

## Testing & acceptance criteria (what “done” means later)

### Unit tests
- Link logic:
  - sustained idle → virtualize
  - util spike or QoO drop → unvirtualize
  - dwell time and rate limiting enforced
  - missing RTT while not sustained-idle causes safe unvirtualize
- Circuit SQM logic:
  - sustained idle + CPU allows saving + QoO good (if available) → fq_codel
  - utilization increase / CPU low / QoO bad (if available) / missing RTT while not sustained-idle → CAKE
  - dwell and rate limiting enforced
- Overrides ownership:
  - TreeGuard updates only treeguard-owned entries
  - removing from allowlists removes TreeGuard control

### Acceptance criteria by milestone
Milestone (a)
- For each enrolled link, TreeGuard can report utilization and RTT (p95) per direction with smoothing/stability.

Milestone (b)
- In non-dry-run mode, under CPU pressure, TreeGuard can persist virtual/unvirtual changes via overrides and apply them with controlled reloads.

Milestone (c)
- In non-dry-run mode, TreeGuard can switch SQM CAKE↔fq_codel with guardrails; persisted changes remain consistent across scheduler runs.

Milestone (d)
- Under realistic load, TreeGuard can reduce CPU usage without unacceptable RTT regression and without flapping.

---

## Defaults & tuning notes

Decisions locked in for v1:
- Enrollment is explicit allowlists by default (`nodes = [...]`, `circuits = [...]`), with optional `all_nodes` / `all_circuits` convenience toggles.
- Missing RTT for >= 2 minutes is treated as unsafe **when an entity is not sustained-idle** (block new CPU-saving actions; revert when applicable). On sustained-idle links/circuits, missing RTT is expected and does not block “idle wind-down” decisions.
- Unknown/missing capacity means **no changes** (warn in UI instead).
- QoO threshold default is **80**; no additional safety signals beyond utilization + RTT availability + QoO.
- All thresholds must be editable via config/UI; expect iterative tuning in real deployments.

---

## Open questions (tighten before coding)

These are the remaining decisions that can materially affect implementation complexity or behavior.

1) **QoO availability policy**
- If QoO is not available (no profile enabled / no `qoq_heatmap` allocated / UI shows `None`), should TreeGuard:
  - A) ignore QoO entirely and proceed using utilization + RTT freshness only, or
  - B) treat QoO as “unknown” and therefore block CPU-saving actions?
- Proposed v1 default: **A (QoO is an optional guard: enforce only when `Some(score)` is present).**

2) **Missing RTT behavior on idle**
- We currently say: “missing RTT for >= 120s is unsafe; block new CPU-saving actions; revert when applicable.”
- On truly idle links/circuits, RTT may naturally go missing (no TCP timestamp samples). Should we:
  - A) keep the strict rule (missing RTT triggers revert), or
  - B) only treat missing RTT as a *blocker* for taking new actions, but not as a reason to revert unless utilization is rising?
- Proposed v1 default: **B** (missing RTT is expected on sustained-idle links/circuits; do not force reverts while still sustained-idle).

3) **Override ownership when allowlisting**
- If an operator already has manual overrides for a node/circuit and then adds it to the TreeGuard allowlist, should TreeGuard:
  - A) take ownership immediately and overwrite as needed, or
  - B) refuse to manage and surface a UI warning until the operator clears conflicts?
- Proposed v1 default: **B** (avoid surprises; prevent fights).

4) **Override cleanup on disable / unallowlist**
- When TreeGuard is disabled or an entity is removed from allowlists, should TreeGuard:
  - A) always remove its persisted overrides (restoring integration/base behavior), or
  - B) leave overrides in place and require explicit operator cleanup?
- Proposed v1 default: **A** (ownership ends → overrides removed).

5) **User-visible notifications**
- Besides the Status + Activity dashlets, should we also emit:
  - A) a toast/banner when a persistent override write happens, and
  - B) an “urgent issue” entry for reload failures/backoff?
- Proposed v1 default: **A + B** (users must be aware of persistent actions and failures).

---

## Implementation guide (junior-friendly checklist)

This is the “do the work” guide. Every item is a concrete, testable step.

When you finish an item, change `[ ]` to `[x]`.

### 0) Branch + baseline

- [x] Create a new branch from `develop`: `git checkout develop && git pull && git checkout -b intelligence_node_management`.
- [x] Read this spec end-to-end: `NLNET_MILESTONE6_PLAN.md` (you should be able to explain TreeGuard in 2 minutes).
- [x] Build + run baseline tests before touching code:
  - `cd src/rust && cargo test --workspace`
  - (Optional) `cd src/rust && cargo check --workspace`

### 1) Add config schema (Rust, persisted in `/etc/lqos.conf`)

- [x] Create config structs for TreeGuard:
  - Add file: `src/rust/lqos_config/src/etc/v15/treeguard.rs` (new).
  - Export from: `src/rust/lqos_config/src/etc/v15/mod.rs` (add `mod treeguard;` near the other `mod ...;` lines and `pub use treeguard::*;` near the other `pub use ...` lines).
  - Required types (suggestion):
    - `TreeguardConfig { enabled, dry_run, tick_seconds, cpu, links, circuits, qoo }`
    - `TreeguardCpuConfig { mode, cpu_high_pct, cpu_low_pct }`
    - `TreeguardLinksConfig { enabled, nodes, idle_util_pct, idle_min_minutes, rtt_missing_seconds, unvirtualize_util_pct, min_state_dwell_minutes, max_link_changes_per_hour, reload_cooldown_minutes }`
    - `TreeguardCircuitsConfig { enabled, circuits, switching_enabled, independent_directions, rtt_missing_seconds, min_switch_dwell_minutes, max_switches_per_hour, persist_sqm_overrides }`
    - `TreeguardQooConfig { enabled, min_score }`
  - Derives: `Serialize`, `Deserialize`, `Clone`, `Debug`, `PartialEq`, `Allocative` (match other config structs).
  - Defaults MUST match this spec’s TOML block (see `NLNET_MILESTONE6_PLAN.md` “Proposed TOML shape (v1)”).

- [x] Add TreeGuard to the top-level config struct:
  - Edit: `src/rust/lqos_config/src/etc/v15/top_config.rs` (`pub struct Config`).
  - Add field: `pub treeguard: TreeguardConfig`.
  - Edit: `src/rust/lqos_config/src/etc/v15/top_config.rs` (`impl Default for Config`) to populate defaults.

- [x] Update the example config:
  - Edit: `src/rust/lqos_config/src/etc/v15/example.toml` (add a `[treeguard]` section near other top-level feature sections).

### 2) Node Manager: TreeGuard configuration page (UI + wiring)

- [x] Create the TreeGuard config HTML page:
  - Add file: `src/rust/lqosd/src/node_manager/static2/config_treeguard.html`.
  - Copy pattern from: `src/rust/lqosd/src/node_manager/static2/config_stormguard.html`.
  - Include form inputs for every config field in the TOML spec (including the allowlists).
  - Load script: `<script src="config_treeguard.js%CACHEBUSTERS%"></script>`.

- [x] Serve the new HTML page with templates/auth:
  - Edit: `src/rust/lqosd/src/node_manager/static_pages.rs:31` (`let html_pages = [`).
  - Add `"config_treeguard.html"` near the other config pages.

- [x] Add the TreeGuard config page to the config menu:
  - Edit: `src/rust/lqosd/src/node_manager/js_build/src/config/config_helper.js:274` (`renderConfigMenu`).
  - Add a menu item like `{ href: "config_treeguard.html", icon: "fa-rocket", text: "TreeGuard", id: "treeguard" }`.

- [x] Implement the TreeGuard config page JavaScript:
  - Add file: `src/rust/lqosd/src/node_manager/js_build/src/config_treeguard.js`.
  - Copy pattern from: `src/rust/lqosd/src/node_manager/js_build/src/config_stormguard.js` and `src/rust/lqosd/src/node_manager/js_build/src/config_queues.js`.
  - Must do:
    - Use `loadConfig(...)`/`saveConfig(...)` from `src/rust/lqosd/src/node_manager/js_build/src/config/config_helper.js:49` and `src/rust/lqosd/src/node_manager/js_build/src/config/config_helper.js:61`.
    - `loadConfig(() => { ... populate fields from window.config ... })`
    - Validate numeric inputs (no negatives; sensible bounds).
    - Save via `saveConfig(updatedConfig, ...)`.
  - Important: allowlists are authoritative. Provide simple “Add / remove” UI lists for:
    - `treeguard.links.nodes`
    - `treeguard.circuits.circuits`

- [x] Rebuild and copy the UI assets:
  - After adding new HTML: run `bash src/rust/lqosd/copy_files.sh` (copies static2 + builds JS + copies bundles).
  - During JS-only iteration: run `bash src/rust/lqosd/dev_build.sh` (builds JS + copies bundles).

### 3) Node Manager: TreeGuard dashboard elements (Status + Activity/Audit)

- [x] Add WS channels for TreeGuard:
  - Edit: `src/rust/lqosd/src/node_manager/ws/published_channels.rs:19` (`pub enum PublishedChannels`).
  - Add: `TreeGuardStatus`, `TreeGuardActivity`.

- [x] Define WS message payloads + events:
  - Edit: `src/rust/lqosd/src/node_manager/ws/messages.rs:276` (`pub enum WsResponse`).
  - Add new data structs above the enum (example types to design):
    - `TreeguardStatusData` (enabled/dry_run, cpu_max, counts, last_action_summary, warnings)
    - `TreeguardActivityEntry` (time, entity_type, entity_id, action, persisted, reason)
  - Add new enum variants:
    - `TreeGuardStatus { data: TreeguardStatusData }`
    - `TreeGuardActivity { data: Vec<TreeguardActivityEntry> }`

- [x] Add ticker(s) to publish TreeGuard data once per second:
  - Add file: `src/rust/lqosd/src/node_manager/ws/ticker/treeguard.rs` (new).
  - Edit: `src/rust/lqosd/src/node_manager/ws/ticker.rs`:
    - add `mod treeguard;` near the other `mod ...;` lines.
    - add `ticker_with_timeout("treeguard_status", ...)` and `ticker_with_timeout("treeguard_activity", ...)` inside the `join!(...)` list in `one_second_cadence`.

- [x] Add dashlets in JS:
  - Add files:
    - `src/rust/lqosd/src/node_manager/js_build/src/dashlets/treeguard_status.js`
    - `src/rust/lqosd/src/node_manager/js_build/src/dashlets/treeguard_activity.js`
  - Subscribe to channels:
    - `subscribeTo() { return ["TreeGuardStatus"]; }`
    - `subscribeTo() { return ["TreeGuardActivity"]; }`
  - Edit: `src/rust/lqosd/src/node_manager/js_build/src/dashlets/dashlet_index.js`:
    - import new dashlets near the other imports at the top (`src/rust/lqosd/src/node_manager/js_build/src/dashlets/dashlet_index.js:1`)
    - add menu items to `DashletMenu` (category suggestion: “TreeGuard”)
    - add `case "treeguardStatus": ...` and `case "treeguardActivity": ...` in `widgetFactory(...)`

### 4) TreeGuard module scaffolding (Rust, inside `lqosd`)

- [x] Create the new module directory:
  - Add folder: `src/rust/lqosd/src/treeguard/`
  - Add at minimum:
    - `src/rust/lqosd/src/treeguard/mod.rs` (public API + start function)
    - `src/rust/lqosd/src/treeguard/errors.rs` (`thiserror` types)
    - `src/rust/lqosd/src/treeguard/state.rs` (per-node/per-circuit state, last seen, dwell timers, EWMAs)
    - `src/rust/lqosd/src/treeguard/decisions.rs` (pure decision functions; document side effects = none)
    - `src/rust/lqosd/src/treeguard/actor.rs` (the actor loop; side effects documented)
    - `src/rust/lqosd/src/treeguard/overrides.rs` (write `lqos_overrides.treeguard.json`)
    - `src/rust/lqosd/src/treeguard/reload.rs` (rate-limited reload/backoff)
    - `src/rust/lqosd/src/treeguard/bakery.rs` (live apply SQM changes)
    - `src/rust/lqosd/src/treeguard/status.rs` (status snapshot + activity log for UI)

- [x] Register the module in `lqosd`:
  - Edit: `src/rust/lqosd/src/main.rs:21` (module list near `mod throughput_tracker;`).
  - Add: `mod treeguard;`

- [x] Start the TreeGuard actor at daemon startup:
  - Edit: `src/rust/lqosd/src/main.rs:245` where other subsystems start (near `throughput_tracker::spawn_throughput_monitor(...)`).
  - Start TreeGuard after:
    - config load
    - throughput tracker is running
    - queue structure monitor is running (TreeGuard will need `queueingStructure.json` for circuit class ids)

### 5) Implement telemetry sampling (links, circuits, CPU, QoO)

- [x] CPU sampling:
  - Use the already-running system stats actor started in `src/rust/lqosd/src/system_stats.rs:29` (`start_system_stats()`).
  - In TreeGuard, define “cpu_max_pct” as `max(SystemStats.cpu_usage)`.

- [x] Link/node sampling:
  - Read from `NETWORK_JSON` in `src/rust/lqosd/src/shaped_devices_tracker/netjson.rs:8` (global `NETWORK_JSON` lock).
  - Find nodes by name using `lqos_config::NetworkJson::get_index_for_name()` (`src/rust/lqos_config/src/network_json.rs:120`).
  - Capacity:
    - use `NetworkJsonNode.max_throughput` (Mbps); if 0/unknown, **do not make changes** and record a warning.
  - RTT missing detection:
    - use `RttBuffer.last_seen` (nanos since boot) (`src/rust/lqos_utils/src/rtt/rtt_buffer.rs:194`) and compare to `time_since_boot()` (imported in `src/rust/lqosd/src/throughput_tracker/mod.rs:29`).
    - if age >= `rtt_missing_seconds` (120s), treat RTT as missing/unsafe.

- [x] QoO sampling (guard rail):
  - For circuits, QoO is already computed in the throughput tracker:
    - reference: `src/rust/lqosd/src/shaped_devices_tracker/mod.rs:268` (`pub fn get_all_circuits()` builds `qoo` fields).
  - In TreeGuard, snapshot per-circuit QoO (down/up). **QoO is an optional guard**: enforce only when the value is `Some(score)`.
  - Use `treeguard.qoo.min_score = 80.0` as the “safe to optimize” threshold.

- [x] Circuit utilization sampling:
  - Aggregate per-circuit `bytes_per_second` from `THROUGHPUT_TRACKER.raw_data` (sum across devices/hosts per circuit hash).
  - Capacity:
    - derive per-circuit max down/up Mbps from ShapedDevices (max across devices in the circuit).
    - if 0/unknown, **do not make changes** and record a warning.

### 6) Implement decision logic (pure functions + state machines)

- [x] Implement EWMA + sustained-idle tracking:
  - “Basically idle” = utilization below `idle_util_pct` for >= `idle_min_minutes` (15 minutes) *after smoothing*.
  - Keep per-direction EWMAs + timers in `src/rust/lqosd/src/treeguard/state.rs`.

- [x] Link virtualization state machine (per allowlisted node):
  - Implement in `src/rust/lqosd/src/treeguard/decisions.rs` (pure).
  - Virtualize only if:
    - sustained idle (>= 15 minutes)
    - QoO (if available) >= 80 for relevant direction(s)
    - CPU pressure meets `cpu_high_pct`
  - Unvirtualize if:
    - utilization rises above `unvirtualize_util_pct`, OR
    - QoO (if available) < 80, OR
    - RTT becomes missing for >= 120s while not sustained-idle
  - Enforce:
    - dwell time (`min_state_dwell_minutes`)
    - rate limit (`max_link_changes_per_hour`)

- [x] Circuit SQM switching state machine (per allowlisted circuit, per direction):
  - Implement in `src/rust/lqosd/src/treeguard/decisions.rs` (pure).
  - Downgrade CAKE → fq_codel only if:
    - sustained idle for >= `idle_min_minutes` (per direction)
    - CPU pressure meets `cpu_high_pct` (or `traffic_rtt_only` mode is enabled)
    - QoO >= 80 (if available)
  - Revert fq_codel → CAKE if:
    - utilization rises above `upgrade_util_pct`, OR
    - CPU <= `cpu_low_pct`, OR
    - QoO < 80 (if available), OR
    - RTT missing for >= 120s while not sustained-idle
  - Persist decisions as directional tokens (`"down/up"`) because `independent_directions = true`.

### 7) Apply changes (persistence, live updates, reloads)

- [x] Persist link virtualization using overrides:
  - Implement in `src/rust/lqosd/src/treeguard/overrides.rs`.
  - Use `lqos_overrides::OverrideStore` layered APIs (`src/rust/lqos_overrides/src/overrides_file.rs`):
    - `OverrideStore::load_layer(OverrideLayer::Treeguard)`
    - `OverrideFile::set_network_node_virtual(...)`
    - `OverrideStore::save_layer(OverrideLayer::Treeguard, ...)`
  - Reconciliation rule:
    - TreeGuard must remove its `set_node_virtual` entries for nodes that are no longer allowlisted or when TreeGuard is disabled.

- [x] Persist circuit SQM changes **without changing overrides format**:
  - Implement in `src/rust/lqosd/src/treeguard/overrides.rs`.
  - Use `OverrideFile::add_persistent_shaped_device_return_changed(...)` to store **device overlays** keyed by `device_id` with `sqm_override` set, and save via the TreeGuard overrides layer.
  - For allowlisted circuits, write overlays for **every device_id in the circuit** so `LibreQoS.py` never sees mixed SQM tokens.
  - Reconciliation rule:
    - Remove overlays for devices that are no longer in allowlisted circuits or when TreeGuard is disabled.

- [x] Update scheduler override application to prevent data loss:
  - Edit: `src/scheduler.py`:
    - `merge_rows_replace_by_device_id(...)` (`src/scheduler.py:219`)
    - `apply_lqos_overrides()` (`src/scheduler.py:247`)
  - Change behavior so “persistent devices” are treated as **field overlays**, not full-row replacements:
    - Patch only the SQM column (row index 13) for matching `device_id`.
    - Do NOT overwrite integration-owned fields (names, IPs, rates, parent).

- [x] Apply circuit SQM changes live (Bakery):
  - Implement in `src/rust/lqosd/src/treeguard/bakery.rs`.
  - Source of class ids: `QUEUE_STRUCTURE` in `src/rust/lqos_queue_tracker/src/queue_structure/queing_structure_json_monitor.rs:14`.
  - Use `queueingStructure.json` parsed nodes (see `QueueNode` in `src/rust/lqos_queue_tracker/src/queue_structure/queue_node.rs:9`) to obtain:
    - `class_major`, `up_class_major`, `class_minor`, `parent_class_id`, `up_parent_class_id`, bandwidth mins/maxes
  - Send Bakery live updates via the existing bakery sender (see `BakeryCommands::AddCircuit` in `src/rust/lqos_bakery/src/commands.rs:132`).
  - Make the live command consistent with the persisted SQM token (directional if independent).

- [x] Reload when topology changes (link virtualization):
  - Implement in `src/rust/lqosd/src/treeguard/reload.rs`.
  - Use existing reload path:
    - `src/rust/lqosd/src/program_control.rs:4` calls `lqos_config::load_libreqos()`
    - `src/rust/lqos_config/src/program_control.rs:24` shells out to run `LibreQoS.py` + restarts `lqos_scheduler`
  - Enforce:
    - `reload_cooldown_minutes`
    - backoff on failure
    - user-visible warning (TreeGuard Activity/Audit dashlet + urgent issue if appropriate)

- [x] Ensure reload picks up overrides without scheduler:
  - Edit: `src/LibreQoS.py`
  - Apply `lqos_overrides.json` adjustments in-memory at shaping time:
    - persistent device overlays (SQM tokens by `deviceID`)
    - network adjustments (e.g., `set_node_virtual`)
  - Do not modify `ShapedDevices.csv` or `network.json` on disk.

### 8) Connect TreeGuard to the UI tickers (status + activity)

- [x] Implement status snapshot API in TreeGuard:
  - In `src/rust/lqosd/src/treeguard/status.rs`, provide functions like:
    - `treeguard_status_snapshot() -> TreeguardStatusData`
    - `treeguard_activity_snapshot() -> Vec<TreeguardActivityEntry>`
  - Store activity as a ring buffer (e.g., last 200 entries) in the TreeGuard actor.

- [x] Wire WS tickers to call these snapshot functions:
  - Implement in `src/rust/lqosd/src/node_manager/ws/ticker/treeguard.rs`.
  - Publish to:
    - `PublishedChannels::TreeGuardStatus` with `WsResponse::TreeGuardStatus { ... }`
    - `PublishedChannels::TreeGuardActivity` with `WsResponse::TreeGuardActivity { ... }`

### 9) Tests (do not skip — NLNet deliverable quality matters)

- [x] Add unit tests for TreeGuard decision logic:
  - Add tests under `src/rust/lqosd/src/treeguard/` (e.g., `decisions.rs` or `mod tests` files).
  - Must cover:
    - allowlist-only ownership behavior
    - 15-minute sustained idle behavior
    - RTT missing for 120s blocks/forces revert; QoO < 80 blocks/forces revert (when QoO is available)
    - independent down/up SQM switching and directional token formatting

- [x] Add tests for scheduler overlay behavior:
  - Add Python tests (if there is an existing test harness) or add a small focused test function near `src/scheduler.py` that can be run in CI/dev to ensure only SQM is patched.

- [x] Run the full workspace tests again:
  - `cd src/rust && cargo test --workspace`

### 10) Manual validation checklist (what to verify before calling it “done”)

- [ ] In dry-run mode, confirm TreeGuard dashboard shows “would change” actions but makes no persistent edits.
- [ ] Enable TreeGuard on a small allowlist (1–2 nodes, 1–2 circuits), confirm:
  - link virtualization persists via overrides and survives scheduler runs
  - circuit SQM changes persist via overrides and survive scheduler runs
  - down/up decisions can differ and appear correctly in UI
  - [ ] Confirm “unknown capacity” nodes never change but are clearly warned in UI.
  - [ ] Confirm missing RTT for >= 2 minutes causes safe reverts when not sustained-idle (and that users see it in the Activity/Audit dashlet).
