## Rust dependency audit: cargo audit and cargo machete

Date: 2026-05-15

Scope: Rust workspace under `src/rust/`, using `src/rust/Cargo.toml` and
`src/rust/Cargo.lock`.

Commands run:

- `cargo audit`
- `cargo machete`
- `cargo tree --locked --workspace --depth 1`

### Summary

- `cargo audit` scanned 549 locked crate dependencies.
- No CVE-class vulnerability was reported.
- One `rand` soundness advisory is present in the locked graph. Current repo
  usage touches affected APIs. The specific advisory trigger also requires a
  custom logger that calls `rand::thread_rng` / `rand::rng` during reseeding;
  no such logger setup was found in `src/rust/`.
- `cargo machete` found one likely unused dependency: `tokio` in
  `lqos_network_devices`.
- The remaining `cargo audit` warnings were maintenance-only notices and are
  not counted as security issues here.

### Findings

#### RUSTSEC-2026-0097 / GHSA-cq8v-f236-94qc: `rand` soundness advisory

Paths importing or using the affected dependency/API:

- `src/rust/lqosd/Cargo.toml` imports `rand = "0.8.5"`.
- `src/rust/lqosd/src/node_manager/auth.rs` uses
  `rand::thread_rng().fill_bytes(...)` for session-key generation.
- `src/rust/lqosd/src/node_manager/ws/single_user_channels.rs` uses
  `rand::random::<u64>()` for chatbot request IDs.
- `src/rust/lqos_probe/Cargo.toml` imports `rand = "0.8.5"`.
- `src/rust/lqos_probe/src/lib.rs` uses `rand::random` for ICMP ping IDs.
- `src/rust/lqosd/Cargo.toml` also imports `tungstenite`,
  `tokio-tungstenite`, and `axum-extra`, which appear in the `cargo audit`
  dependency tree above `rand 0.8.5`.

Short description:

`rand` versions including `0.8.5` have a soundness advisory involving
`rand::thread_rng` / `rand::rng`. The trigger requires the `log` and
`thread_rng` features, a custom logger, that logger calling the RNG, and the RNG
reseeding while called from the logger. LibreQoS has affected API calls, but the
audit did not find a custom `log::Log` implementation or `log::set_logger` path
in `src/rust/`, so the full trigger path was not found.

Recommended actions:

- Update direct `rand` usage to a patched release line, or replace the direct
  session-key generation path with an OS RNG API such as `rand_core::OsRng` /
  `getrandom`.
- After updating, run `cargo update -p rand --precise 0.8.6` if staying on
  the `0.8` line, then `cargo audit` and focused checks for `lqosd` and
  `lqos_probe`.
- Keep this as a release-tracked follow-up, but do not block release solely on
  the advisory unless a custom logger path is introduced or discovered.

#### Unused dependency: `tokio` in `lqos_network_devices`

Path:

- `src/rust/lqos_network_devices/Cargo.toml` imports `tokio = { workspace = true }`.

Short description:

`cargo machete` reports `tokio` as unused in `lqos_network_devices`. A source
search found no `tokio` references under `src/rust/lqos_network_devices/`.
This is not a security issue, but it is avoidable dependency surface.

Recommended actions:

- Remove the `tokio` dependency from `src/rust/lqos_network_devices/Cargo.toml`.
- Run `cargo check -p lqos_network_devices`.
- Re-run `cargo machete` to confirm the dependency list is clean.

### Non-security audit warnings

The following `cargo audit` results are maintenance-only warnings, not security
findings under this audit policy:

- `bincode 1.3.3` is unmaintained; imported by `lqosd`.
- `fxhash 0.2.1` is unmaintained; imported by `lqosd` and
  `lqos_network_devices`.
- `paste 1.0.15` is unmaintained; transitive through `default-net` /
  `netlink-packet-*`.
- `serde_cbor 0.11.2` is unmaintained; imported by several LibreQoS crates and
  `lqos_bus`.

These should be tracked as modernization work, not listed as release security
findings unless a separate vulnerability is reported for them.
