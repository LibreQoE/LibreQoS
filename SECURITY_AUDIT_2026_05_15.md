## Introduction

This audit is a basic pre-release security review of LibreQoS as of
2026-05-15. It focuses on dependency advisories, Rust dependency hygiene,
control-plane exposure, bridged-interface and eBPF malformed-traffic handling,
panic/error/type-loss paths, and Node Manager privacy, authentication, and XSS
risk.

The review is static unless a section explicitly says otherwise. It does not
cover Ubuntu/Linux distribution vulnerabilities, live firewall state, NIC driver
behavior, or private deployment-specific controls that are not visible in this
checkout.

## Executive summary

- Overall grade: **C**. LibreQoS has several strong security foundations, but
  the current release state includes multiple actionable control-plane and WebUI
  issues that should be fixed or explicitly risk-accepted before release.
- No CVE-class Rust dependency vulnerability was confirmed as reachable during
  the dependency audit. The `rand` soundness advisory has been remediated by
  updating the locked 0.8 dependency line to `rand 0.8.6`, and one likely unused
  dependency should be removed.
- The Caddy/TLS path is generally well-directed for the WebUI, but the sibling
  API service can still bind directly on all interfaces, malformed bearer headers
  can panic API authentication, WebUI/local API CORS is too broad, and WebUI
  login lacks rate limiting.
- The eBPF packet parsers show good verifier-conscious bounds checking for
  common encapsulation paths, but malformed traffic can still create operational
  DoS or visibility loss through packet-rate debug logging, IPv4 fragment/header
  edge cases, and non-LRU map exhaustion.
- The panic/type-loss review found request-time panic paths and unchecked numeric
  narrowing that can lose operational data in queue stats and NetFlow export.
- Node Manager has the highest concentration of release-relevant WebUI risk:
  anonymous/read-only paths can retrieve raw subscriber and topology data,
  operator-controlled strings reach `innerHTML`, the session cookie is readable
  by JavaScript, and read-only users can write dashboard themes that can become
  stored XSS.
- Recommended release priority is to first fix WebUI XSS/session-cookie exposure,
  server-side anonymization for anonymous/demo mode, API direct-listener/TLS
  bypass risk, malformed auth-header panic, and production packet-path debug
  logging.

## Rust dependency audit

Date: 2026-05-15

Scope: Rust workspace under `src/rust/`, using `src/rust/Cargo.toml` and
`src/rust/Cargo.lock`.

Commands run:

- `cargo audit`
- `cargo machete`
- `cargo tree --locked --workspace --depth 1`
- `cargo update -p rand@0.8.5 --precise 0.8.6`
- `cargo tree --locked -p lqosd -e features`
- `cargo check -p lqos_probe -p lqosd`

### Summary

- `cargo audit` scanned 549 locked crate dependencies.
- No CVE-class vulnerability was reported.
- The previous `rand` soundness advisory no longer appears in `cargo audit`
  after updating `rand 0.8.5` to `rand 0.8.6`.
- Direct `rand` imports now disable default features and re-enable only `std`
  and `std_rng`, which are required for the current `thread_rng` and `random`
  calls. The resolved graph still enables `rand/default` through upstream
  `tungstenite` and `cookie` dependencies, so feature minimization is limited
  to LibreQoS' direct imports unless those upstream crates are patched or
  upgraded.
- `cargo machete` found one likely unused dependency: `tokio` in
  `lqos_network_devices`.
- The remaining `cargo audit` warnings were maintenance-only notices and are
  not counted as security issues here.

### Dependency findings and triage notes

#### Resolved: RUSTSEC-2026-0097 / GHSA-cq8v-f236-94qc `rand` soundness advisory

Paths importing or using the affected dependency/API:

- `src/rust/lqosd/Cargo.toml` imports
  `rand = { version = "0.8.6", default-features = false, features = ["std", "std_rng"] }`.
- `src/rust/lqosd/src/node_manager/auth.rs` uses
  `rand::thread_rng().fill_bytes(...)` for session-key generation.
- `src/rust/lqosd/src/node_manager/ws/single_user_channels.rs` uses
  `rand::random::<u64>()` for chatbot request IDs.
- `src/rust/lqos_probe/Cargo.toml` imports
  `rand = { version = "0.8.6", default-features = false, features = ["std", "std_rng"] }`.
- `src/rust/lqos_probe/src/lib.rs` uses `rand::random` for ICMP ping IDs.
- `src/rust/lqosd/Cargo.toml` also imports `tungstenite`,
  `tokio-tungstenite`, and `axum-extra`, which appear in the `cargo audit`
  dependency tree above `rand 0.8.6`.

Short description:

`rand` versions including `0.8.5` have a soundness advisory involving
`rand::thread_rng` / `rand::rng`. The trigger requires the `log` and
`thread_rng` features, a custom logger, that logger calling the RNG, and the RNG
reseeding while called from the logger. LibreQoS has affected API calls, but the
audit did not find a custom `log::Log` implementation or `log::set_logger` path
in `src/rust/`, so the full trigger path was not found.

Remediation:

- Updated the locked 0.8 line with
  `cargo update -p rand@0.8.5 --precise 0.8.6`.
- Minimized LibreQoS' direct `rand` feature requests to `std` and `std_rng`.
- Re-ran `cargo audit`; the `rand` advisory is no longer reported.
- Re-ran `cargo check -p lqos_probe -p lqosd`; both crates compile with the
  updated dependency.

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

The WebUI and local API install `CorsLayer::very_permissive()`, a permissive
credentialed CORS policy. The WebUI session uses a `User-Token` cookie with
`SameSite=Lax`, but without `Secure` or `HttpOnly`.

Exposure / threat:

I did not find a documented cross-origin WebUI client in the reviewed code. For
a cookie-authenticated control-plane UI, permissive credentialed CORS grants
browser read access to origins outside the WebUI's own origin whenever the
browser sends the `User-Token` cookie. `SameSite=Lax` limits common
unrelated-site subresource requests, but this policy is still broader than the
reviewed WebUI needs. The local API and WebUI should not grant credentialed
CORS to arbitrary origins without a documented client need.

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

## Bridged interface / eBPF malformed-traffic audit

Date: 2026-05-15

Scope assumptions:

- The two bridged interfaces are in scope for this section, including XDP,
  TC/eBPF, pinned maps, ring-buffer events, and userspace consumers of eBPF
  output.
- The control interface and Caddy/API/WebUI exposure are covered by the prior
  section and are out of scope here.
- Linux, Ubuntu, kernel, and NIC driver vulnerabilities are out of scope. This
  section reviews LibreQoS packet parser behavior, map pressure, debug logging,
  and userspace handling of eBPF output.
- No live XDP/TC attach-detach, pinned-map cleanup, or packet fuzzing was
  performed. Findings are based on static review of in-repo code.

Files and directories reviewed:

- `src/rust/lqos_sys/src/bpf/common/debug.h`
- `src/rust/lqos_sys/src/bpf/common/dissector.h`
- `src/rust/lqos_sys/src/bpf/common/dissector_tc.h`
- `src/rust/lqos_sys/src/bpf/common/flows.h`
- `src/rust/lqos_sys/src/bpf/common/heimdall.h`
- `src/rust/lqos_sys/src/bpf/common/lpm.h`
- `src/rust/lqos_sys/src/bpf/common/throughput.h`
- `src/rust/lqos_sys/src/bpf/common/maximums.h`
- `src/rust/lqos_sys/src/lqos_kernel.rs`
- `src/rust/lqosd/src/throughput_tracker/flow_data/flow_analysis/kernel_ringbuffer.rs`
- `src/rust/lqosd/src/throughput_tracker/tracking_data.rs`
- `src/rust/lqos_heimdall/src/perf_interface.rs`
- `src/rust/lqos_heimdall/src/timeline.rs`
- `src/rust/lqos_heimdall/src/pcap.rs`

Review searches:

- `rg "bpf_debug\\(|frag_off|ihl|tot_len|doff|BPF_MAP_TYPE_HASH|BPF_MAP_TYPE_PERCPU_HASH|BPF_MAP_TYPE_LRU|MAX_FLOWS|MAX_TRACKED_IPS|bpf_ringbuf_output|bpf_probe_read_kernel" src/rust/lqos_sys/src src/rust/lqosd/src/throughput_tracker src/rust/lqos_heimdall/src`

### Summary

- The XDP/TC packet dissectors use explicit `data_end` bounds checks and bounded
  VLAN/MPLS loops. The review did not find memory-unsafe packet reads in the
  normal parser path.
- Parser failures in `xdp_prog` return `XDP_PASS`; parser failures in
  `tc_iphash_to_cpu` return `TC_ACT_OK`. Unknown non-IP traffic therefore
  passes unshaped in the reviewed XDP/TC paths.
- Five malformed-traffic / resource-exhaustion findings are listed below.
  Packet-rate `bpf_trace_printk` is reachable through the `bpf_debug` macro
  from malformed short UDP/ICMP paths.
- Several findings affect observability and flow analysis more than packet
  forwarding. They can still break LibreQoS operationally by hiding current
  flow/host state, filling pinned maps, or burning CPU on bridged-interface
  traffic.

### Findings

#### Malformed packets can trigger packet-rate BPF trace logging

Paths:

- `src/rust/lqos_sys/src/bpf/common/debug.h`
- `src/rust/lqos_sys/src/bpf/common/dissector.h`
- `src/rust/lqos_sys/src/bpf/common/flows.h`
- `src/rust/lqos_sys/src/bpf/common/throughput.h`

Short description:

`bpf_debug(...)` expands directly to `bpf_trace_printk(...)`. Some call sites
are behind `VERBOSE` or `TRACING`, but several error paths reachable from packet
handling are not. Examples include truncated UDP/ICMP headers in
`dissector.h` and map insertion failures in `flows.h` and `throughput.h`.

Exposure / threat:

An attacker on a bridged interface can send malformed or high-cardinality
traffic that repeatedly hits these error paths. `bpf_trace_printk` is expensive
and writes into the kernel tracing path; at packet rate this can consume CPU and
trace-buffer bandwidth on the shaping host. Once map-pressure findings below
are triggered, failed insertions can also create a second packet-rate logging
path.

Recommended actions:

- Compile `bpf_debug` to a no-op unless `VERBOSE` or `TRACING` is explicitly
  enabled.
- Replace packet-rate error logging with counters in a bounded BPF map that
  userspace can poll at a low rate.
- Treat any remaining `bpf_trace_printk` call in XDP/TC packet paths as a debug
  build feature, not production behavior.

#### IPv4 fragments and invalid IPv4 header lengths can pollute flow tracking

Path:

- `src/rust/lqos_sys/src/bpf/common/dissector.h`

Short description:

The XDP dissector verifies that an IPv4 header-sized region is present, then
uses `iph->ihl * 4` to locate TCP, UDP, or ICMP headers. The reviewed code does
not reject `ihl < 5`, does not verify the IPv4 total length against the captured
packet bounds, and does not skip L4 snooping for fragmented IPv4 packets.

Exposure / threat:

Malformed IPv4 packets can make the dissector derive L4 ports and TCP flags
from bytes that are not a valid L4 header. When those bytes make the apparent
TCP data offset large enough, the TCP timestamp parser can also run against
fragment payload rather than a real TCP options area. On the TCP path, this can
seed or update Flowbee records, retransmit counters, and RTT sampling inputs
with attacker-chosen fragment payload bytes.

UDP and ICMP fragments can also be interpreted as flow traffic if enough
payload bytes are present. Those paths can create or update UDP/ICMP Flowbee
entries from fragment payload instead of a valid UDP or ICMP header.

Recommended actions:

- Validate IPv4 `version == 4`, `ihl >= 5`, and `l3offset + ihl * 4 <= data_end`
  before any L4 header lookup.
- Validate IPv4 total length enough to ensure the parsed L4 header is inside the
  IPv4 packet, not just inside the received frame.
- Skip L4 snooping and Flowbee updates for IPv4 fragments with non-zero fragment
  offset or `MF` set. Continue IP-level LPM shaping if desired.
- Add a small packet corpus for malformed IPv4 IHL values, truncated TCP/UDP,
  and fragmented IPv4 packets.

#### UDP/ICMP spray can fill the non-LRU Flowbee map

Paths:

- `src/rust/lqos_sys/src/bpf/common/flows.h`
- `src/rust/lqos_sys/src/bpf/common/maximums.h`
- `src/rust/lqosd/src/throughput_tracker/tracking_data.rs`

Short description:

`flowbee` is a pinned `BPF_MAP_TYPE_HASH` with `MAX_FLOWS` entries. The UDP and
ICMP handlers create a new Flowbee entry whenever no entry exists, even when the
IP mapping result has `tc_handle == 0`. TCP has a guard for non-SYN packets with
no mapping, but UDP and ICMP do not have the same shaped-traffic guard.

Exposure / threat:

Traffic with many spoofed IPs and ports can fill `flowbee` with unshaped UDP or
ICMP entries. When the map is full, later legitimate flow insertions fail and
LibreQoS loses current flow, RTT, retransmit, and QoE visibility for real
traffic. Each failed insert also reaches a `bpf_debug` path, which can amplify
the logging DoS above.

Recommended actions:

- Do not create UDP/ICMP Flowbee entries when `tc_handle == 0`, unless a
  documented feature requires unshaped flow visibility.
- Consider changing `flowbee` to an LRU map, or add a bounded admission policy
  for unshaped UDP/ICMP.
- Expose Flowbee map pressure and insertion failures to userspace as counters,
  not trace logs.
- Add tests or a packet-replay harness that confirms unshaped UDP/ICMP sprays do
  not evict or block shaped TCP flow visibility.

#### Spoofed IP spray can fill the per-host traffic map

Paths:

- `src/rust/lqos_sys/src/bpf/common/throughput.h`
- `src/rust/lqos_sys/src/bpf/common/maximums.h`

Short description:

`map_traffic` is declared as `BPF_MAP_TYPE_PERCPU_HASH` with
`MAX_TRACKED_IPS = 128000`. The comment says the map is LRU, but the declared
type is not an LRU map. `track_traffic` inserts a host counter for every parsed
IP host key, including unshaped traffic with `tc_handle == 0`.

Exposure / threat:

An attacker can send traffic with many spoofed source or destination IPs through
the bridge and fill the host counter map. Once full, new legitimate host
counters fail to insert, unknown-IP and per-host throughput visibility becomes
misleading, and every failed insertion can hit `bpf_debug("Failed to insert
flow")`.

Recommended actions:

- Change the map type to an LRU variant if eviction of old host counters is the
  intended behavior, or correct the comment and add explicit map-pressure
  handling.
- Avoid inserting unshaped hosts into `map_traffic` unless operator-facing
  unknown-IP visibility requires it.
- Add userspace counters for insert failures and map occupancy so operators can
  distinguish real quiet periods from map exhaustion.

#### Heimdall packet capture copies a fixed 128 bytes and ignores event-backpressure errors

Paths:

- `src/rust/lqos_sys/src/bpf/common/heimdall.h`
- `src/rust/lqos_heimdall/src/perf_interface.rs`
- `src/rust/lqos_heimdall/src/pcap.rs`

Short description:

When Heimdall analysis mode is enabled for a watched IP, the eBPF path copies
`PACKET_OCTET_SIZE` bytes from the packet start into each event and sends the
event through `heimdall_events`. The copy length is fixed at 128 bytes, the
return from `bpf_probe_read_kernel` is ignored, and the return from
`bpf_ringbuf_output` is ignored.

Exposure / threat:

This path is conditional on Heimdall watch mode, so it is not a default
bridging exposure. When enabled, short or malformed watched packets can produce
zero-padded or incomplete packet dumps without any signal to userspace. High
rate watched traffic can also fill the ring buffer. The current eBPF path
ignores the `bpf_ringbuf_output` return value, and the reviewed userspace path
has a collected-event counter and missed-tick warning but no surfaced
ring-buffer drop counter for Heimdall captures.

Recommended actions:

- Clamp the packet-copy length to the available packet length and
  `PACKET_OCTET_SIZE`.
- Check the return values from `bpf_probe_read_kernel` and `bpf_ringbuf_output`
  and increment bounded counters for copy failures and dropped events.
- Surface Heimdall copy-failure and ring-buffer drop counters through the
  existing Heimdall/lqosd status path when packet capture is active.

### Observations / not findings

- VLAN, PPPoE, and MPLS parsing uses bounded loops and `data_end` checks before
  walking encapsulation headers. This limits parser runtime on stacked headers.
- Unknown non-IP traffic passes unshaped by design, preserving ARP, STP, IS-IS,
  and similar bridge traffic.
- IPv6 extension headers and IPv6 fragments are not deeply parsed for Flowbee
  L4 metrics. The reviewed path still performs IP-level LPM mapping for
  unsupported protocols, so this is an observability gap rather than a shaping
  bypass in the reviewed code.
- Flowbee RTT ring-buffer userspace handling validates event size before copying
  and uses a bounded in-process queue with coalesced wakeups.
- Runtime packet fuzzing, pinned-map occupancy checks, and live bridge-interface
  reachability were not performed in this stage.

## Panic, error-handling, and type-loss audit

Date: 2026-05-15

Scope:

- Runtime Rust under `src/rust/`, current Python entrypoints under `src/`, and
  sibling `../../lqos_api/src/` because the API is deployed behind Caddy.
- Tests, generated output, vendored bindings, and historical Python copies were
  excluded unless a runtime path referenced them.
- This was a static source review. No live service, XDP/TC attach-detach, or
  packet replay was performed.

Review searches:

- `rg -n "\bpanic!\(|\.unwrap\(|\.expect\(|unreachable!\(|todo!\(|unimplemented!\(|assert!\(|from_raw_parts|transmute|unsafe \{|as (u8|u16|u32|usize|i8|i16|i32|f32)|unwrap_or_default\(|except Exception|except:|pass$" src/rust src --glob '*.py' ../../lqos_api/src`
- `rg -n "as u32|as u16|as f32|partial_cmp\(.*\)\.unwrap|to_str\(\)\.unwrap|parse\(\)\.unwrap|try_into\(\)\.unwrap" src/rust ../../lqos_api/src`
- `rg -n "except Exception|except:|pass$|sys.exit|int\(|float\(" src --glob '*.py' --glob '!LibreQoS-old.py' --glob '!LibreQoS-ancient.py' --glob '!LibreQoS.py.new'`

### Summary

- Four confirmed findings are listed below: one authenticated request-time panic,
  one request/websocket error-handling panic pattern, one queue-stat type-loss
  issue, and one NetFlow export type-loss issue.
- Two reachability-unknown hardening items are listed separately: API bandwidth
  float narrowing and RTT percentile sorting on floats.
- The malformed `x-bearer` panic at `../../lqos_api/src/web_security.rs:32` was
  already recorded in the network control-plane section and is not duplicated as
  a new finding here.
- I did not find a confirmed memory-unsafe unsoundness issue in the reviewed
  runtime paths. The high-volume unsafe hits were mostly FFI wrappers, generated
  libbpf bindings, or callbacks that check event size before `from_raw_parts`.

### Findings

#### Authenticated packet-capture download can panic on missing capture file

Paths:

- `src/rust/lqosd/src/node_manager/local_api.rs:44`
- `src/rust/lqosd/src/node_manager/local_api/packet_analysis.rs:29`
- `src/rust/lqosd/src/node_manager/local_api/packet_analysis.rs:35`

Short description:

The authenticated `/local-api/pcapDump/:id` route calls
`n_second_pcap(id).expect(...)` and later `ServeFile::try_call(...).expect(...)`.
An invalid, expired, or removed packet-capture session can panic the request task
instead of returning a normal HTTP error.

Exposure / threat:

This route is behind the WebUI auth layer, so it is not an unauthenticated remote
panic. A logged-in user, stale browser request, or automation using an old capture
ID can still trigger request-time failure on the control plane. If the panic
poisons shared state in the surrounding server path, the blast radius could be
larger than one failed download.

Recommended actions:

- Return `404 Not Found` or `410 Gone` when `n_second_pcap(id)` cannot resolve a
  capture file.
- Convert `ServeFile::try_call` errors into a bounded `5xx` response and log the
  underlying path/error once.
- Add a focused route test for a missing capture ID and for a removed capture
  file.

#### Flow-explorer websocket handlers panic when time sources fail

Paths:

- `src/rust/lqosd/src/node_manager/local_api/flow_explorer.rs:42`
- `src/rust/lqosd/src/node_manager/local_api/flow_explorer.rs:45`
- `src/rust/lqosd/src/node_manager/local_api/flow_explorer.rs:104`
- `src/rust/lqosd/src/node_manager/local_api/flow_explorer.rs:107`
- `src/rust/lqosd/src/node_manager/local_api/flow_explorer.rs:117`
- `src/rust/lqosd/src/node_manager/local_api/flow_explorer.rs:120`
- `src/rust/lqosd/src/node_manager/ws.rs:665`
- `src/rust/lqosd/src/node_manager/ws.rs:674`
- `src/rust/lqosd/src/node_manager/ws.rs:683`

Short description:

The flow-explorer timeline builders use `expect(...)` on `time_since_boot()` and
`unix_now()`. These functions are called from websocket message handlers for ASN,
country, and protocol timelines.

Exposure / threat:

The direct trigger is an operating-system time retrieval failure, not attacker
controlled input. Still, once the condition exists, any authenticated websocket
request for these timeline views can panic instead of returning an empty/error
payload. This is incorrect request-time error handling on a control-plane feature.

Recommended actions:

- Return `Result<Vec<FlowTimeline>, Error>` from the timeline builders and send a
  structured websocket error when the clock calls fail.
- Reuse a small helper that computes boot time once with explicit logging.
- Add a focused unit test for the transport conversion path and a websocket test
  for the error response if the time helper is injectable.

#### Queue tracker silently narrows kernel qdisc counters

Paths:

- `src/rust/lqos_queue_tracker/src/queue_types/tc_cake.rs:102`
- `src/rust/lqos_queue_tracker/src/queue_types/tc_cake.rs:117`
- `src/rust/lqos_queue_tracker/src/queue_types/tc_cake.rs:187`
- `src/rust/lqos_queue_tracker/src/queue_types/tc_cake.rs:198`
- `src/rust/lqos_queue_tracker/src/queue_types/tc_cake.rs:206`
- `src/rust/lqos_queue_tracker/src/queue_types/tc_fq_codel.rs:61`
- `src/rust/lqos_queue_tracker/src/queue_types/tc_fq_codel.rs:62`
- `src/rust/lqos_queue_tracker/src/queue_types/tc_htb.rs:49`
- `src/rust/lqos_queue_tracker/src/queue_types/tc_htb.rs:50`
- `src/rust/lqos_queue_tracker/src/queue_types/tc_mq.rs:37`
- `src/rust/lqos_queue_tracker/src/queue_types/tc_mq.rs:38`

Short description:

The qdisc JSON parsers read `tc -s -j` values as `u64` and then cast many packet,
drop, backlog, flow, and memory counters to `u32` or `u16` with `as`. Rust's
integer narrowing casts wrap modulo the destination type, so large counters can
silently become smaller values.

Exposure / threat:

Busy shapers can exceed 32-bit packet/drop counters during normal operation. A
traffic flood can make this happen faster. Wrapped queue stats can hide drops,
mislead capacity and QoE views, and produce incorrect data for downstream
operator or Insight decisions. This is data loss rather than memory corruption.

Recommended actions:

- Keep kernel counters as `u64` through the queue tracker, bus messages, API
  serialization, and UI consumers unless the kernel field is truly bounded.
- Where a protocol/UI contract must stay narrower, use `try_from` with explicit
  clamp-and-warn behavior instead of unchecked `as` casts.
- Add qdisc parser fixtures with values above `u32::MAX` and `u16::MAX`.

#### NetFlow 5 export can wrap flow counts, byte counts, and timestamps

Paths:

- `src/rust/lqosd/src/throughput_tracker/flow_data/netflow5/mod.rs:69`
- `src/rust/lqosd/src/throughput_tracker/flow_data/netflow5/protocol.rs:83`
- `src/rust/lqosd/src/throughput_tracker/flow_data/netflow5/protocol.rs:84`
- `src/rust/lqosd/src/throughput_tracker/flow_data/netflow5/protocol.rs:85`
- `src/rust/lqosd/src/throughput_tracker/flow_data/netflow5/protocol.rs:86`
- `src/rust/lqosd/src/throughput_tracker/flow_data/netflow5/protocol.rs:96`
- `src/rust/lqosd/src/throughput_tracker/flow_data/netflow5/protocol.rs:97`
- `src/rust/lqosd/src/throughput_tracker/flow_data/netflow5/protocol.rs:119`
- `src/rust/lqosd/src/throughput_tracker/flow_data/netflow5/protocol.rs:120`

Short description:

The NetFlow 5 exporter narrows accumulator length, packet counters, byte counters,
and nanosecond timestamps to `u16` or `u32` with unchecked casts. One direction
converts timestamps to milliseconds before narrowing; the reverse record narrows
nanoseconds directly.

Exposure / threat:

NetFlow export is optional, but when enabled it can silently emit wrapped or
inconsistent accounting for long-lived or high-throughput flows. External
collectors may then undercount traffic, mis-order flow times, or make billing and
abuse-analysis decisions from corrupted export records.

Recommended actions:

- Split NetFlow batches before the record count exceeds the protocol limit and
  avoid unchecked `usize -> u16` casts.
- Convert timestamps consistently to the expected NetFlow units before narrowing.
- For NetFlow 5's inherently 32-bit fields, clamp with a warning or emit delta
  records before counters exceed protocol capacity.
- Prefer NetFlow 9/IPFIX-style export for counters that need wider fields.

### Reachability unknown / hardening items

#### API site-speed changes narrow unbounded `f64` request values to `f32`

Paths:

- `../../lqos_api/src/api/network_json.rs:126`
- `../../lqos_api/src/api/network_json.rs:127`
- `../../lqos_api/src/api/network_json.rs:128`
- `../../lqos_api/src/api/network_json.rs:129`
- `../../lqos_api/src/api/network_json.rs:230`
- `../../lqos_api/src/api/network_json.rs:234`
- `../../lqos_api/src/api/network_json.rs:237`
- `../../lqos_api/src/api/network_json.rs:241`
- `../../lqos_api/src/api/network_json.rs:711`
- `../../lqos_api/src/api/network_json.rs:714`
- `../../lqos_api/src/api/network_json.rs:717`
- `../../lqos_api/src/api/network_json.rs:720`

Short description:

The API accepts site-speed values as `Option<f64>`, writes them into
`network.json`, and then narrows values to `f32` for live bus commands and queue
mapping reads. The reviewed code did not show finite/range validation before the
narrowing casts.

Exposure / threat:

The route is bearer-authenticated, so this is not unauthenticated input. A
credentialed caller can submit extremely large or nonsensical bandwidth values
that round or become non-finite when narrowed, depending on downstream handling.
The live command path may then diverge from the JSON value. I did not verify
whether downstream bus receivers reject these values.

Recommended actions:

- Validate site-speed request values as finite, positive, and within explicit
  LibreQoS-supported Mbps bounds before writing JSON or sending live commands.
- Keep one numeric type across API, config, bus, and bakery code where practical.
- Add API tests for huge, negative, zero, fractional, and boundary bandwidth
  values.

#### API transit conversion can panic if RTT samples contain NaN

Path:

- `../../lqos_api/src/transit_types.rs:389`

Short description:

`NetworkJsonTransit::from` sorts RTT samples with
`partial_cmp(...).unwrap()`. `partial_cmp` returns `None` for NaN, which makes the
conversion panic if a NaN reaches the RTT vector.

Exposure / threat:

The current in-repo RTT producers reviewed here mostly derive RTT values from
durations, which should be finite. I did not find a clear external input path to
inject NaN into this vector, so this is marked reachability unknown. If a NaN
does enter the telemetry state, several API endpoints that serialize
`NetworkJsonTransit` can panic while preparing a response.

Recommended actions:

- Filter non-finite RTT samples before sorting, or sort with `f32::total_cmp`.
- Add a small conversion test with `[10.0, f32::NAN]` to prove the API response
  path does not panic.

### Observations / not findings

- `../../lqos_api/src/web_security.rs:32` remains a confirmed malformed-header
  panic, but it was already recorded in the network control-plane audit section.
- Unsafe callback paths such as `src/rust/lqos_heimdall/src/perf_interface.rs:70`
  through `src/rust/lqos_heimdall/src/perf_interface.rs:77` check event size
  before creating a byte slice. This is not counted as a new unsoundness finding.
- Broad Python exception handling exists in runtime files, including
  `src/LibreQoS.py:2361`, `src/LibreQoS.py:2364`, and `src/LibreQoS.py:2549`.
  The sampled paths are shaping-input tolerance or planner-weight fallback
  behavior. They should be cleaned up for diagnosability, but I did not find a
  concrete security impact in this stage.

## Node Manager privacy, auth, and XSS audit

Date: 2026-05-15

Scope:

- Node Manager HTTP routing, static page routing, auth/session handling,
  websocket request handling, local API data providers, frontend source under
  `src/rust/lqosd/src/node_manager/js_build/src/`, and the templated static HTML
  shell.
- Generated bundles, vendored JavaScript, static images, and source maps were
  excluded from detailed XSS review unless a source file pointed to the behavior.
- This was a static source review. No browser exploit proof-of-concept was run.

Review searches:

- `rg -n "localStorage|sessionStorage|document\.cookie|innerHTML|outerHTML|insertAdjacentHTML|eval\(|Function\(|onclick=|onerror=|sanitize|DOMPurify|redact|redaction|redactable|allow_anonymous|auth_layer|route_layer|LoginResult|ReadOnly|Admin|Denied" src/rust/lqosd/src/node_manager docs/v2.0`
- `rg -n 'innerHTML\s*=.*(\+|`)|simpleRowHtml\(|href=.*\+|data-[^=]+=|textContent|innerText' src/rust/lqosd/src/node_manager/js_build/src --glob '*.js'`
- `rg -n "ShapedDevice|network_json|CircuitById|AllShapedDevices|NetworkJson|Search|UnknownIps|CircuitDirectory|device_name|circuit_name|mac|ipv4|ipv6|comment" src/rust/lqosd/src/node_manager`

### Summary

- Four confirmed findings are listed below: raw PII exposure in anonymous/read-only
  surfaces, multiple DOM XSS sinks for operator/customer fields, JS-readable
  session cookies, and dashboard-theme writes/XSS reachable by read-only users.
- I did not find a websocket path that serves data before the handshake completes:
  the server sends `Hello`, requires `HelloReply`, rejects denied tokens, and
  closes on other pre-handshake messages.
- Static HTML pages and `/local-api/*` are routed through `auth_layer`. The
  fallback static file server remains a hardening concern, but the reviewed
  current `static2` files are static assets or pages already listed in the
  authenticated router.
- No session token or API key was found in `localStorage`. The higher-risk browser
  storage issue is that the session cookie is readable by JavaScript so the
  websocket client can copy it into the handshake.

### Findings

#### Anonymous/read-only mode exposes raw subscriber and topology data without server-side anonymization

Paths:

- `src/rust/lqos_config/src/authentication.rs:487`
- `src/rust/lqos_config/src/authentication.rs:498`
- `src/rust/lqosd/src/node_manager/auth.rs:421`
- `src/rust/lqosd/src/node_manager/auth.rs:431`
- `src/rust/lqosd/src/node_manager/ws.rs:2199`
- `src/rust/lqosd/src/node_manager/ws.rs:2207`
- `src/rust/lqosd/src/node_manager/local_api/config.rs:528`
- `src/rust/lqosd/src/node_manager/local_api/config.rs:541`
- `src/rust/lqosd/src/node_manager/local_api/shaped_devices_page.rs:118`
- `src/rust/lqosd/src/node_manager/local_api/shaped_devices_page.rs:123`
- `src/rust/lqosd/src/node_manager/local_api/shaped_devices_page.rs:131`
- `docs/v2.0/components.md:123`
- `docs/v2.0/components.md:126`
- `src/rust/lqosd/src/node_manager/js_build/src/helpers/redact.js:48`

Short description:

The documented anonymous option grants unauthenticated users `ReadOnly` access.
Read-only websocket requests include raw `network.json` and all shaped devices.
Those payloads include circuit names/IDs, device names/IDs, parent node names,
MACs, comments, and IPv4/IPv6 addresses. The current redaction feature is a
client-side display blur/font mode, documented as not modifying source data.

Exposure / threat:

Anonymous read-only mode is intended for demos, but demos are exactly where PII
redaction matters. Anyone who can reach a node with anonymous mode enabled can
request raw payloads directly over the websocket, regardless of whether the UI
redaction toggle is on. Read-only authenticated users also receive the same raw
data, so redaction is not a privacy boundary.

Recommended actions:

- Add a server-side anonymized/demo payload mode for anonymous access. Replace
  subscriber/circuit/device names, IDs, IPs, MACs, and comments before sending
  websocket responses.
- Consider field-level filtering for normal `ReadOnly` users where full
  subscriber/device identifiers are not needed.
- Keep browser redaction as a screenshot tool, but do not rely on it for public
  demo privacy.
- Add tests that anonymous mode cannot retrieve raw `AllShapedDevices`,
  `NetworkJson`, circuit search, or circuit detail identifiers.

#### Operator-controlled names reach `innerHTML` without escaping

Paths:

- `src/rust/lqosd/src/node_manager/js_build/src/template.js:797`
- `src/rust/lqosd/src/node_manager/js_build/src/template.js:799`
- `src/rust/lqosd/src/node_manager/js_build/src/template.js:801`
- `src/rust/lqosd/src/node_manager/js_build/src/circuit.js:2164`
- `src/rust/lqosd/src/node_manager/js_build/src/dashlets/top10_downloaders.js:97`
- `src/rust/lqosd/src/node_manager/js_build/src/dashlets/worst10_downloaders.js:97`
- `src/rust/lqosd/src/node_manager/js_build/src/dashlets/worst10_retransmits.js:101`
- `src/rust/lqosd/src/node_manager/js_build/src/config_users.js:101`
- `src/rust/lqosd/src/node_manager/js_build/src/config_users.js:104`
- `src/rust/lqosd/src/node_manager/js_build/src/helpers/builders.js:45`

Short description:

Several frontend paths concatenate circuit names, device names, site names,
dashboard/user fields, or other operator-controlled values into `innerHTML`. Some
nearby code has local `escapeHtml` helpers, but these sinks do not consistently
use them. `simpleRowHtml` is a reusable helper that writes its argument directly
to `td.innerHTML`.

Exposure / threat:

Circuit/device/site names can come from operator files or external integrations.
A malicious or compromised integration record containing HTML can execute script
when a user searches, opens a circuit page, views dashboard top lists, or opens
the user management page. Because the WebUI controls shaping and configuration,
stored XSS should be treated as a control-plane compromise path, not a cosmetic
bug.

Recommended actions:

- Replace these sinks with DOM construction and `textContent` for untrusted text.
  Keep icons as separately created `<i>` elements.
- Ban `simpleRowHtml` for untrusted data. Use `simpleRow` or an escaped helper
  with a name that makes trust explicit.
- Escape attribute values separately from text nodes and validate URL protocols
  before assigning links.
- Add frontend tests or a small DOM fixture using a circuit name like
  `<img src=x onerror=alert(1)>` to confirm it renders as text everywhere.

#### WebUI session token is readable by JavaScript and copied into websocket auth

Paths:

- `src/rust/lqosd/src/node_manager/auth.rs:266`
- `src/rust/lqosd/src/node_manager/auth.rs:269`
- `src/rust/lqosd/src/node_manager/js_build/src/pubsub/ws.js:66`
- `src/rust/lqosd/src/node_manager/js_build/src/pubsub/ws.js:78`

Short description:

The `User-Token` session cookie is created with `SameSite=Lax`, but it is not
marked `HttpOnly` or `Secure`. The frontend websocket client reads
`document.cookie` and sends the session token in the websocket handshake.

Exposure / threat:

Any XSS in Node Manager can read and exfiltrate the signed session token. That
turns DOM injection into session theft, and an admin session can perform
configuration and shaping actions. `Secure` should also be set when the Caddy/TLS
option is active so the browser will not send the cookie over plain HTTP.

Recommended actions:

- Move websocket authentication to the server-side `Cookie` header during the
  upgrade, or issue a short-lived websocket nonce that is not the durable session
  token.
- Set `HttpOnly` on the session cookie once the websocket no longer needs
  JavaScript cookie access.
- Set `Secure` whenever Node Manager is served through HTTPS/Caddy, with a
  documented local-development exception if needed.
- Add an XSS regression test that verifies the session token is not available via
  `document.cookie`.

#### Read-only websocket users can write dashboard themes, enabling stored XSS

Paths:

- `src/rust/lqosd/src/node_manager/ws.rs:354`
- `src/rust/lqosd/src/node_manager/ws.rs:371`
- `src/rust/lqosd/src/node_manager/ws.rs:378`
- `src/rust/lqosd/src/node_manager/local_api/dashboard_themes.rs:35`
- `src/rust/lqosd/src/node_manager/local_api/dashboard_themes.rs:60`
- `src/rust/lqosd/src/node_manager/local_api/dashboard_themes.rs:97`
- `src/rust/lqosd/src/node_manager/js_build/src/lq_js_common/dashboard/dashboard.js:1024`

Short description:

The websocket dashboard-theme save/get/delete handlers do not check for
`LoginResult::Admin`. `dashboard_themes` only normalizes `/` and `\` in the theme
filename, stores the provided `name`, and later returns it for display. The saved
layout list writes `d.name` into `innerHTML`.

Exposure / threat:

Any authenticated read-only user, and any anonymous user when demo mode is
enabled, can save a dashboard layout with a malicious name. When an admin or
operator opens the saved-layout picker, that name can execute as stored XSS and
steal the JS-readable session token described above.

Recommended actions:

- Require `LoginResult::Admin` for `DashletSave` and `DashletDelete`, or document
  and constrain why read-only users should be allowed to write shared layouts.
- Escape `d.name` when rendering saved layouts, preferably with `textContent`.
- Validate dashboard theme names server-side with an allowlist of printable
  characters and a length limit.
- Add websocket tests for read-only denial and an XSS fixture for saved layout
  names.

### Hardening observations / not findings

- `src/rust/lqosd/src/node_manager/ws.rs:311` through
  `src/rust/lqosd/src/node_manager/ws.rs:329` require the websocket `HelloReply`
  before processing normal requests. This review did not find a pre-handshake
  data response.
- `src/rust/lqosd/src/node_manager/static_pages.rs:114` through
  `src/rust/lqosd/src/node_manager/static_pages.rs:117` apply auth and templates
  to the listed HTML pages, and `src/rust/lqosd/src/node_manager/local_api.rs:66`
  applies `auth_layer` to `/local-api`. `src/rust/lqosd/src/node_manager/run.rs:85`
  still has an unauthenticated fallback `ServeDir`; keep it restricted to static
  assets and avoid adding data files or unlisted HTML pages under `static2`.
- `src/rust/lqosd/src/node_manager/local_api/config.rs:341` through
  `src/rust/lqosd/src/node_manager/local_api/config.rs:349` require admin access
  and redact integration secrets before returning the main config view.
- `src/rust/lqosd/src/node_manager/js_build/src/config_interface.js:4`,
  `src/rust/lqosd/src/node_manager/js_build/src/config_interface.js:5`,
  `src/rust/lqosd/src/node_manager/js_build/src/config_interface.js:376`, and
  `src/rust/lqosd/src/node_manager/js_build/src/config_interface.js:424` store
  network-mode drafts and pending operation IDs in `localStorage`. I did not find
  credentials there, but interface names, VLANs, and operation IDs are
  operationally sensitive on shared browsers. Prefer `sessionStorage` or clear
  these keys on logout, confirm, and rollback.

## Conclusion

Overall security grade: **C**.

LibreQoS is not starting from a bad place. The review found no confirmed
reachable CVE-class Rust dependency issue, the managed Caddy setup moves the
intended WebUI path toward loopback-backed TLS, Node Manager has a real
authentication/role model, eBPF packet parsing uses many explicit `data_end`
checks and bounded encapsulation loops, and several sensitive admin config values
are already redacted before they reach the browser. Those are meaningful
strengths, and they reduce the chance that the next release ships with a simple
dependency or parser-memory-safety failure.

The weaknesses are concentrated around control-plane hardening and browser trust
boundaries. The biggest current risks are WebUI stored/DOM XSS, a JavaScript
readable session token, raw PII exposure in anonymous/read-only demo paths, an
API direct-listener path that can bypass the intended Caddy/TLS route if exposed,
request-time panics on malformed or stale inputs, and packet-path behavior that
can lose observability or consume CPU under malformed/high-cardinality bridged
traffic.

The release posture should be: acceptable foundations, but not yet polished
security hygiene. Fixing the Node Manager XSS/session/anonymization issues and
the API/control-plane panic/listener issues would move the project materially
toward a **B**. Addressing the eBPF map-pressure/debug-logging findings, adding
rate limiting, and cleaning up numeric type-loss paths would make the security
state much more comfortable for routine release cadence.
