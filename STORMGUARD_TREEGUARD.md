# StormGuard and TreeGuard

## Current Relationship

StormGuard and TreeGuard overlap in intent, but not yet in implementation.

- StormGuard is a live runtime component that watches telemetry and changes top-level HTB ceilings dynamically.
- TreeGuard, in the current branch, is primarily config, defaults, dashboard, and UI/schema work.
- TreeGuard does not yet appear to have a backend worker in this repo comparable to `lqos_stormguard`.

That means the immediate reuse opportunity is not "replace StormGuard with TreeGuard", but "reuse the better TreeGuard control model and persistence contract to improve StormGuard".

## What StormGuard Does Today

StormGuard:

- loads a configured allowlist of site names from `[stormguard].targets`
- resolves those names into queue nodes and dependent child nodes
- monitors throughput, retransmits, and RTT once per second
- applies multiplicative HTB ceiling changes through the bakery
- keeps an in-memory override map in the bakery so limits can be replayed after queue rebuilds in the same process

Relevant code:

- `src/rust/lqos_stormguard/src/lib.rs`
- `src/rust/lqos_stormguard/src/config.rs`
- `src/rust/lqos_stormguard/src/site_state.rs`
- `src/rust/lqos_bakery/src/lib.rs`

## What TreeGuard Adds Conceptually

TreeGuard introduces a more operationally useful model:

- broad scope controls such as `all_nodes` and `all_circuits`
- explicit allowlists only when narrowing scope
- configurable dwell times, rate limits, and cooldowns
- explicit dry-run and enable toggles
- a persistence concept for scheduler-safe SQM overrides

Relevant code:

- `src/rust/lqos_config/src/etc/v15/treeguard.rs`
- `src/rust/lqosd/src/node_manager/static2/config_treeguard.html`

## Best Opportunities

### 1. Move StormGuard to persistent overrides

This is the biggest improvement.

Today StormGuard writes live HTB changes and the bakery stores them in an in-memory `stormguard_overrides` map. That lets the bakery replay them after queue rebuilds while `lqosd` remains alive, but it is still not a durable source of truth across restart.

TreeGuard already establishes the right direction: persist adaptive changes so scheduler and rebuild flows do not fight them.

Recommended direction:

- replace StormGuard's in-memory-only override map with a persisted override file or persisted override store
- replay persisted overrides at bakery rebuild time
- keep dry-run behavior unchanged
- expose override state in debug UI so operators can see what is active

This would directly address the historical "it is overwritten on scheduler run" problem more cleanly than the current in-memory replay.

## 2. Give StormGuard TreeGuard-style scope controls

StormGuard currently depends on a manual `targets` list of site names.

TreeGuard's scope model is better:

- apply to all eligible objects by default
- optionally narrow scope with an allowlist

Recommended direction:

- add `all_sites = true/false` to StormGuard
- keep `targets` only as an allowlist when `all_sites = false`
- consider a small exclusion list if operators want broad enablement with a few carve-outs

This would make StormGuard much easier to deploy at scale.

## 3. Move StormGuard policy constants into config

StormGuard currently hardcodes:

- increase/decrease multipliers
- cooldown timing
- some effective policy behavior thresholds

TreeGuard's config model is more release-friendly because operators can tune behavior without code changes.

Recommended direction:

- expose StormGuard multipliers in config
- expose cooldowns and rate limits in config
- expose optional per-direction behavior controls
- validate these values in `lqos_config` the same way TreeGuard now validates its ranges and relationships

This would reduce risk and make StormGuard easier to support.

## 4. Use TreeGuard-style circuit actions when StormGuard cannot safely touch HTB

StormGuard explicitly skips circuit-level queues that host a qdisc, because changing HTB at that level is unsafe.

TreeGuard's circuit model suggests a better fallback:

- when StormGuard detects instability on a queue it should not modify directly
- hand off a per-circuit mitigation action instead of doing nothing
- the obvious candidate is TreeGuard-style SQM switching such as CAKE to fq_codel for CPU savings or recovery behavior

This is likely the cleanest place where TreeGuard extends StormGuard rather than replaces it.

## 5. Build one shared adaptive-override substrate

Longer term, StormGuard and TreeGuard probably should not each own their own action plumbing.

Better structure:

- StormGuard remains the congestion/instability policy
- TreeGuard remains the topology/SQM policy
- both emit actions into one shared override layer
- the bakery owns persistence, replay, and application of overrides
- the UI/debug layer reports active overrides and why they were applied

That would avoid duplicated action paths and make adaptive behavior easier to reason about.

## Practical Recommendation Order

If we want the highest value with the least disruption:

1. Add persistent StormGuard overrides first.
2. Add TreeGuard-style scope controls to StormGuard.
3. Move StormGuard tuning constants into config.
4. Add a handoff path where StormGuard can trigger TreeGuard-style circuit actions for qdisc-hosting queues.
5. Only after that, consider unifying both systems under a single adaptive framework.

## One Caution

While reviewing StormGuard, the dependent-node discovery in `src/rust/lqos_stormguard/src/queue_structure.rs` looks worth re-checking before building more behavior on top of it. The descendant filter logic is compact and may be more fragile than intended.

