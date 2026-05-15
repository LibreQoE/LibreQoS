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

## Network control-plane audit

Date: 2026-05-15

Scope assumptions:

- The Caddy / SSL / TLS option is installed.
- Linux, Ubuntu, kernel, and distribution package vulnerabilities are out of
  scope for this section.
- The control interface is in scope. The two bridge interfaces, whether XDP or
  Linux bridge backed by eBPF, are out of scope for this section.
- An open listener is not a vulnerability by itself. This section looks for
  exploitability, unauthorized access, credential exposure, authorization
  bypass, or remotely triggerable failure on the control plane.
- The sibling `../../lqos_api/` repo is included because it is exposed behind
  the managed Caddy configuration.

Files and directories reviewed:

- `docs/v2.0/https-caddy.md`
- `docs/v2.0/api.md`
- `src/rust/lqos_setup/src/ssl.rs`
- `src/rust/lqos_setup/src/web.rs`
- `src/rust/lqosd/src/node_manager/run.rs`
- `src/rust/lqosd/src/node_manager/auth.rs`
- `src/rust/lqosd/src/node_manager/local_api.rs`
- `src/rust/lqosd/src/node_manager/static_pages.rs`
- `src/rust/lqos_config/src/authentication.rs`
- `../../lqos_api/src/main.rs`
- `../../lqos_api/src/web.rs`
- `../../lqos_api/src/web_security.rs`
- `../../lqos_api/README.md`

### Summary

- The managed Caddy configuration disables the Caddy admin API, proxies the
  WebUI to `127.0.0.1:9123`, and proxies `/api/v1/*` to `127.0.0.1:9122`.
- The WebUI runtime listener is configured to move to loopback for the Caddy
  path. That loopback listener is not a finding.
- The sibling `lqos_api` service still binds directly to `:::9122`. If that
  port is reachable on the control interface, authenticated API traffic can
  bypass the Caddy/TLS path. Runtime reachability was not verified in this
  code audit.
- Three control-plane findings and one reachability-unknown exposure are listed
  below. Public API documentation and the explicit anonymous read-only demo mode
  are recorded as observations, not findings by themselves.

### Findings

#### Reachability unknown: direct `lqos_api` listener can bypass the Caddy/TLS path

Path:

- `../../lqos_api/src/main.rs`

Short description:

`lqos_api` binds its HTTP server to `:::9122`. The managed Caddy configuration
proxies API traffic to `127.0.0.1:9122`, but the API process itself also remains
able to listen on all interfaces unless deployment firewalling blocks it.

Exposure / threat:

The API binds all interfaces while Caddy proxies the intended HTTPS path to
localhost. If port `9122` is reachable on the control interface, a client can
send the `x-bearer` credential over direct HTTP instead of the Caddy-protected
HTTPS path. This audit verified the code-level listener and Caddy upstream, but
did not verify runtime firewall or socket exposure on an installed host.

Recommended actions:

- Make the `lqos_api` listen address configurable and default it to
  `127.0.0.1:9122` when the Caddy option is installed.
- Update the Caddy/setup integration and API documentation so remote operators
  use only the HTTPS `/api/v1/` path.
- Add install-time firewall guidance or service hardening that blocks direct
  control-interface access to `9122` when Caddy is enabled.

#### Malformed `x-bearer` header can panic API authentication

Path:

- `../../lqos_api/src/web_security.rs`

Short description:

The API authentication middleware calls `header.to_str().unwrap()` while
processing the unauthenticated `x-bearer` header.

Exposure / threat:

A remote client can send a malformed header value that fails UTF-8 conversion.
Authentication should reject that request, but the current code can panic while
handling unauthenticated input. Even if Axum/Tokio limits the blast radius to a
request task or connection, this is a remotely triggerable control-plane failure
path.

Recommended actions:

- Replace the `unwrap()` with explicit error handling that returns
  `401 Unauthorized` for invalid or missing bearer headers.
- Keep malformed authentication input on the same path as other auth failures:
  no panic, no stack trace, and no different response body that helps probing.
- After fixing, add a small test for a non-UTF-8 or otherwise invalid
  `x-bearer` value.

#### WebUI and local API use credentialed permissive CORS

Paths:

- `src/rust/lqosd/src/node_manager/run.rs`
- `src/rust/lqosd/src/node_manager/static_pages.rs`
- `src/rust/lqosd/src/node_manager/local_api.rs`
- `src/rust/lqosd/src/node_manager/auth.rs`

Short description:

The WebUI and local API install `CorsLayer::very_permissive()`. In the
tower-http version locked by the workspace, that mirrors the request origin and
allows credentials. The WebUI session uses a `User-Token` cookie with
`SameSite=Lax`, but without `Secure` or `HttpOnly`.

Exposure / threat:

The reviewed code does not show a supported cross-origin WebUI client. For a
cookie-authenticated control-plane UI, reflecting arbitrary origins while
allowing credentials grants browser read access to origins outside the WebUI's
own origin whenever the browser sends the `User-Token` cookie. `SameSite=Lax`
limits common unrelated-site subresource requests, but this policy is still
broader than the reviewed WebUI needs. The local API and WebUI should not grant
credentialed CORS to arbitrary origins without a documented client need.

Recommended actions:

- Remove CORS from same-origin WebUI/local API routes unless a concrete
  supported cross-origin client requires it.
- If cross-origin access is required, restrict allowed origins to configured
  operator hosts and avoid credentialed wildcard or origin-mirroring behavior.
- Add origin or CSRF checks for state-changing browser routes.
- Set session cookies with `Secure` when served behind HTTPS and `HttpOnly`
  unless browser JavaScript truly needs to read the cookie.

#### WebUI login lacks rate limiting

Paths:

- `src/rust/lqosd/src/node_manager/auth.rs`
- `src/rust/lqos_config/src/authentication.rs`

Short description:

The WebUI login path checks passwords with Argon2id for current hashes and
upgrades older SHA-256 hashes after successful login. The reviewed code did not
show per-IP, per-account, or global throttling for repeated failed login
attempts to the public `/doLogin` route.

Exposure / threat:

Under the Caddy setup, the login form is reachable through the control-plane
HTTPS entrypoint. An unauthenticated client can repeatedly submit passwords to
`/doLogin`; the server-side password hash is strong, but the reviewed code does
not throttle repeated failures before each verification attempt.

Recommended actions:

- Add rate limiting or exponential backoff for failed `/doLogin` attempts,
  keyed by source address and username.
- Log repeated failures in a way operators can act on without logging submitted
  passwords.

### Observations / not findings

- `src/rust/lqos_setup/src/ssl.rs` renders a managed Caddyfile with
  `admin off`, WebUI upstream `127.0.0.1:9123`, and API upstream
  `127.0.0.1:9122`.
- `docs/v2.0/https-caddy.md` documents moving the WebUI runtime listener to
  `127.0.0.1:9123` when HTTPS is enabled.
- `../../lqos_api/src/web.rs` merges Swagger UI at `/api-docs`. This exposes
  endpoint shape, not credentials or control actions, and is not counted as a
  vulnerability by itself.
- `allow_anonymous` is an explicit read-only public/demo mode in the WebUI
  authentication configuration. It is not counted as a finding when the operator
  intentionally enables that mode.
- `src/rust/lqos_setup/src/web.rs` binds the setup web service to
  `0.0.0.0:9123` and uses a setup token. Because this section assumes the
  runtime Caddy option is already installed, first-run setup exposure is left
  for a later setup/lifecycle audit unless a concrete bypass is found.
