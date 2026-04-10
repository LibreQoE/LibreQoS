# `lqos_network_devices`

Centralized, shared access to LibreQoS topology inputs:

- `ShapedDevices.csv` (and `ShapedDevices.insight.csv`)
- `network.json` (and `network.insight.json` / `network.effective.json`)

## Runtime model

In daemon mode, this crate starts:

- A single-thread actor that owns reload/apply commands.
- A config-directory watcher that coalesces filesystem events and requests reloads.

Callers access state through public accessor functions; the underlying channel sender is kept in a
private static.

## Public API (high level)

- `start_daemon_mode(hooks)`: starts actor + directory watcher.
- `shaped_devices_snapshot()`: `Arc<ConfigShapedDevices>` snapshot.
- `shaped_device_hash_cache_snapshot()`: hash→index cache snapshot (device/circuit hash lookups).
- `with_network_json_read(|net_json| ...)`: read-only access to in-memory `NetworkJson`.
- `with_network_json_write(|net_json| ...)`: mutable access to in-memory `NetworkJson` (used by runtime counters).
- `request_reload_shaped_devices(reason)` / `request_reload_network_json(reason)`: ask actor to reload from disk.
- `apply_shaped_devices_snapshot(reason, shaped)`: publish a caller-provided shaped-devices snapshot via actor.
- `swap_shaped_devices_snapshot(reason, shaped_arc)`: replace shaped-devices snapshot without actor (intended for tests).

## Notes for `lqosd`

`lqosd` starts daemon mode and provides hooks to invalidate derived snapshots/caches on updates.
Runtime throughput tracking mutates the in-memory `NetworkJson` via `with_network_json_write`.

## Access & update catalog

Current (2026-04) call sites and update patterns:

- Daemon startup: `lqosd` calls `start_daemon_mode()` once at boot and provides `DaemonHooks`.
- Shaped-device updates:
  - Directory watcher requests reload when `ShapedDevices.csv` or `ShapedDevices.insight.csv` changes.
  - Node manager admin edits write `ShapedDevices.csv` and immediately publish the new snapshot via
    `apply_shaped_devices_snapshot()`.
- Network topology updates:
  - Directory watcher requests reload when `network.json`, `network.insight.json`, or
    `network.effective.json` changes.
  - Node manager admin edits write `network.json` and request reload via `request_reload_network_json()`.
- Hot-path reads (in-memory snapshots): node manager REST endpoints, throughput tracking, Insight/LTS2
  control-channel snapshotting, and shaped-device/circuit views.
- One-shot disk loads: tools/sidecars (e.g. `lqos_python`, `lqos_stormguard`, support tooling) should
  use `load_shaped_devices()` / `load_network_json()` rather than reaching into config modules directly.
