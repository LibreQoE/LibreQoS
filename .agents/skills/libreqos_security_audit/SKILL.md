---
name: libreqos_security_audit
description: Repo-local LibreQoS workflow for release security audit passes. Use when auditing LibreQoS with cargo audit, cargo machete, cargo tree, CVE triage, network control-plane exposure review, bridged-interface/eBPF malformed-traffic review, panic/error-handling/type-loss review, node_manager privacy/auth/XSS review, and audit-file findings updates.
---

# LibreQoS Security Audit

Use this skill for recurring LibreQoS security audit passes in this repo. It
covers the Rust dependency baseline, network control-plane exposure review,
bridged-interface/eBPF malformed-traffic review, and panic/error-handling/type
loss review, and node_manager privacy/auth/XSS review. Use additional focused
checks for Python, packaging, live-host configuration, secrets, and
authentication flows outside these scopes.

## Scope

- Keep the audit repo-local. Do not install or update global skills.
- Start from the current repo checkout and respect dirty worktree boundaries.
- Write results into the requested audit file. If no file is named, ask before creating one.
- Distinguish security findings from maintenance warnings. "Not maintained" is not a security issue by itself.
- Distinguish reachable vulnerabilities from unused-feature or irrelevant-feature advisories. A vulnerability in a feature LibreQoS does not use is not a security finding by itself.
- Treat an open listener as evidence, not a vulnerability by itself. Record a
  security finding only when the listener exposes exploitability, credential or
  session leakage, unauthenticated access beyond the intended demo/public mode,
  authorization bypass, brute-force exposure, remotely triggerable panic/DoS, or
  a control action reachable without the expected protection.

## Step 1: Rust Dependency Audit

Run from `src/rust/` unless the user asks for a narrower crate:

```text
cargo audit
cargo machete
cargo tree --locked --workspace --depth 1
```

Use the depth-1 tree only for a quick workspace dependency summary. For every
advisory or suspicious dependency, run an inverse tree query for the exact crate
and version when Cargo can resolve it, such as `cargo tree -i rand --locked` or
`cargo tree -i rand@0.8.5 --locked`. If Cargo cannot resolve the package ID,
use `cargo audit`'s dependency tree plus `rg` searches through manifests and
source files.

If sandboxing blocks Cargo registry or advisory-db access, rerun the blocked command with approval instead of switching to stale data.

### Reachability Rubric

Treat an advisory as a security finding when at least one of these is true:

- LibreQoS directly imports the vulnerable crate and calls the affected API.
- A current workspace crate exposes the vulnerable behavior through a LibreQoS
  runtime surface:
  `lqosd` node-manager HTTP/websocket handlers, `lqos_bus` message handling,
  `lqos_config` parsing for `network.json` / `ShapedDevices.csv` / config
  files, `lqos_network_devices` runtime access to shaped-device and topology
  state, `lqos_topology` / `lqos_topology_compile` topology projection,
  `lqos_probe` active probing, `lqos_overrides`, `uisp_integration`,
  `lqos_python`, `lqos_setup`, `lqos_netplan_helper`, `lqos_sys`
  TC/XDP/eBPF interaction, queue/bakery crates, or support-tool archive/input
  handling.
- The advisory applies to default features or enabled features in
  `Cargo.toml` / `Cargo.lock`, and no code-level usage check is needed for the
  vulnerable behavior to be present.

Expected evidence searches include the affected crate name in `Cargo.toml`
files, affected function/type names in `src/rust/**`, relevant feature names in
manifests and `Cargo.lock`, and logger/auth/network/file-parsing entry points
when the advisory depends on runtime conditions.

Treat an advisory as not currently security-relevant when all of these are true:

- The affected crate is present only through an unused optional feature,
  build-only path, test-only path, or API surface LibreQoS does not call.
- Feature flags and source search support that conclusion.
- The audit note records what was checked.

If the evidence is incomplete, record it as "reachability unknown" and list the
specific follow-up needed. Do not call it safe.

### Triage Rules

For every `cargo audit` result:

- Record CVE/RUSTSEC/GHSA security advisories as findings only when they meet the reachability rubric above.
- For each security finding, include:
  - the advisory ID
  - the repo-relative path to the `Cargo.toml` that imports the vulnerable crate
  - repo-relative source paths that call the affected API or make the dependency reachable, when applicable
  - a short vulnerability description
  - recommended actions
- Do not list maintenance-only advisories as security findings. Put them in a separate non-security notes section.
- If an advisory depends on specific feature flags, runtime configuration, or API calls, verify those paths with `rg` before writing the conclusion.
- If reachability is unclear, use "reachability unknown" and say exactly what was checked and what remains unknown.

For `cargo machete` results:

- Record unused dependencies separately from security findings.
- Include the repo-relative manifest path and dependency name.
- Recommend removal plus the smallest focused validation command. Use a real
  crate name in the audit note, for example `cargo check -p
  lqos_network_devices`.

For `cargo tree`:

- Use depth-1 output only for a quick workspace summary.
- Use inverse tree queries to identify direct and transitive import paths for findings.
- Keep dependency-surface summaries short and tied to an audit decision.

### Audit Section Checklist

The audit-file section must include:

- Heading: `Rust dependency audit`.
- Date, scope, and exact commands run.
- Summary bullets covering dependency count, security findings, unused
  dependencies, and non-security warnings.
- One subsection per security finding with advisory ID, affected crate, import
  paths, source-use paths where applicable, short description, reachability
  decision, and recommended actions.
- A separate unused-dependencies subsection for `cargo machete` output.
- A separate non-security warnings subsection for unmaintained or informational
  advisories.
- No placeholders, no `...`, and no unresolved `<angle-bracket>` tokens.

## Step 2: Network Control-Plane Exposure Audit

Use this step when the audit turns to external threats over the LibreQoS control
plane.

### Scope Assumptions

- Assume the operator installed the Caddy / SSL / TLS option.
- Treat Linux, Ubuntu, kernel, and distribution package vulnerabilities as out
  of scope because LibreQoS cannot fix them in-repo.
- Treat the control interface as in scope. Anything listening or reachable on
  the control interface should be reviewed.
- Treat the two bridge interfaces, whether XDP or Linux bridge backed by eBPF,
  as out of scope for this section.
- Include the sibling `../../lqos_api/` repo as read-only audit context when it
  is present, because the API is exposed behind Caddy.
- Do not edit `../../lqos_api/` unless the user explicitly authorizes
  cross-repo changes.

### Evidence To Gather

Review these surfaces first:

```text
src/rust/lqos_setup/src/ssl.rs
src/rust/lqos_setup/src/web.rs
src/rust/lqosd/src/node_manager/
docs/v2.0/https-caddy.md
docs/v2.0/api.md
../../lqos_api/src/
../../lqos_api/README.md
```

Use `rg` searches for listener addresses, routes, middleware, auth checks,
cookie settings, CORS, Caddy upstreams, and panic-prone request handling:

```text
rg "bind\\(|TcpListener|listen|reverse_proxy|Caddy|CorsLayer|very_permissive|allow_anonymous|SameSite|Cookie|x-bearer|route_layer|unwrap\\(" src/rust docs ../../lqos_api
```

For each reachable service or route, identify:

- listener address and port
- whether Caddy proxies it and whether the direct port remains reachable
- authentication and authorization mechanism
- unauthenticated routes and whether they expose control, data, or only health/docs
- state-changing routes and their protection
- cookie flags, CORS policy, CSRF/origin checks, and session behavior
- rate limits or backoff for login/API authentication attempts
- request paths where malformed unauthenticated input can panic

### Triage Rules

Count these as likely security findings when evidence supports them:

- a direct control-plane HTTP listener bypasses the expected Caddy/TLS path
  for authenticated API traffic
- a route that changes state or exposes sensitive operational data is reachable
  without the expected auth, except for the explicit public/demo read-only mode
- an auth or request middleware can panic on unauthenticated remote input
- credentialed CORS, cookie flags, or missing CSRF/origin checks let another
  browser origin use an operator session
- login or bearer-token checks lack reasonable throttling for a network-exposed
  control-plane service

Do not count these as findings by themselves:

- Caddy or LibreQoS listening on a port when the route is protected as intended
- Caddy serving HTTPS for the WebUI/API path
- public API documentation that exposes only endpoint shape and no secret or
  state-changing capability
- `allow_anonymous` when the operator intentionally enabled the documented
  public/demo read-only mode
- first-run setup exposure when the setup token and lifecycle are being audited
  separately, unless the current section finds a concrete bypass

### Audit Section Checklist

The audit-file section must include:

- Heading: `Network control-plane audit`.
- Date, scope assumptions, and exact files or directories reviewed.
- Summary bullets separating findings from observations.
- One subsection per finding with the repo-relative path, short description,
  exposure/threat, and recommended actions.
- A separate observations / not-findings subsection for open listeners or public
  docs that are intentional and not vulnerable by themselves.
- No placeholders, no `...`, and no unresolved `<angle-bracket>` tokens.

## Step 3: Bridged Interface / eBPF Malformed-Traffic Audit

Use this step when the audit turns to bridged interfaces and the eBPF datapath.
This step is about malformed-packet handling, DoS, map exhaustion, ring-buffer
backpressure, packet-rate debug logging, and userspace handling of eBPF events.
It is not about the control interface.

Review these BPF-specific surfaces first:

```text
src/rust/lqos_sys/src/bpf/lqos_kern.c
src/rust/lqos_sys/src/bpf/common/debug.h
src/rust/lqos_sys/src/bpf/common/dissector.h
src/rust/lqos_sys/src/bpf/common/dissector_tc.h
src/rust/lqos_sys/src/bpf/common/flows.h
src/rust/lqos_sys/src/bpf/common/heimdall.h
src/rust/lqos_sys/src/bpf/common/lpm.h
src/rust/lqos_sys/src/bpf/common/throughput.h
src/rust/lqos_sys/src/bpf/common/maximums.h
src/rust/lqos_sys/src/lqos_kernel.rs
src/rust/lqosd/src/throughput_tracker/
src/rust/lqos_heimdall/src/
```

Use this search as the starting point:

```text
rg "bpf_debug\\(|frag_off|ihl|tot_len|doff|BPF_MAP_TYPE_HASH|BPF_MAP_TYPE_PERCPU_HASH|BPF_MAP_TYPE_LRU|MAX_FLOWS|MAX_TRACKED_IPS|bpf_ringbuf_output|bpf_probe_read_kernel|data_end|SKB_OVERFLOW|metadata|queue_mapping" src/rust/lqos_sys/src src/rust/lqosd/src/throughput_tracker src/rust/lqos_heimdall/src
```

For each packet path, identify the concrete behavior for:

- malformed Ethernet, VLAN, PPPoE, MPLS, IPv4, IPv6, TCP, UDP, and ICMP input
- IPv4 `ihl`, total length, and fragmentation checks before L4 parsing
- IPv6 extension headers and fragments
- bounded-loop limits for stacked headers and TCP options
- unshaped or spoofed traffic creating pinned-map entries
- map type, max entries, LRU behavior, and insert-failure behavior
- `bpf_trace_printk` / `bpf_debug` calls reachable from bridged traffic
- ring-buffer size checks, backpressure, drop counters, and userspace panics
- metadata paths where malformed packets can become unexpected drops

Count a finding when malformed, spoofed, or high-cardinality traffic can cause
packet-rate expensive work, non-LRU map exhaustion, bogus flow/RTT/retransmit
state, unexpected packet drops, userspace panic, or unreported loss of capture
events. Do not count verifier-enforced memory safety, unknown non-IP traffic
that merely fails open, or untested live reachability claims as findings by
themselves.

## Step 4: Panic, Error-Handling, and Type-Loss Audit

Use this step when the audit turns to code paths that can panic, hide errors, or
silently lose data. Include the sibling `../../lqos_api/src/` when it is present
because it is part of the deployed control-plane surface, but do not edit that
repo unless the user explicitly authorizes cross-repo changes.

Start with these searches, then inspect only runtime-reachable code. Exclude
tests, fixtures, generated output, vendored bindings, and historical copies such
as `LibreQoS-old.py` unless the user explicitly puts them in scope.

```text
rg -n "\\bpanic!\\(|\\.unwrap\\(|\\.expect\\(|unreachable!\\(|todo!\\(|unimplemented!\\(|assert!\\(|from_raw_parts|transmute|unsafe \\{|as (u8|u16|u32|usize|i8|i16|i32|f32)|unwrap_or_default\\(|except Exception|except:|pass$" src/rust src --glob '*.py' ../../lqos_api/src
rg -n "as u32|as u16|as f32|partial_cmp\\(.*\\)\\.unwrap|to_str\\(\\)\\.unwrap|parse\\(\\)\\.unwrap|try_into\\(\\)\\.unwrap" src/rust ../../lqos_api/src
rg -n "except Exception|except:|pass$|sys.exit|int\\(|float\\(" src --glob '*.py' --glob '!LibreQoS-old.py' --glob '!LibreQoS-ancient.py' --glob '!LibreQoS.py.new'
```

For each candidate, identify:

- file name and exact line number
- whether the code is request-time, packet-time, config/import-time, startup-only,
  test-only, or generated/vendor code
- whether an external user, bridged-interface packet, operator-managed file, or
  internal telemetry value can trigger the path
- whether the impact is panic/DoS, poisoned shared state, incorrect rejection,
  silent fallback, lossy conversion, wrapped counters, non-finite float handling,
  or misleading operational data

Count a finding when evidence supports one of these:

- a request, websocket message, packet event, or operator-managed file can panic
  a runtime task instead of returning an error
- malformed input can poison or permanently break shared runtime state
- error handling silently continues with a different shaping, auth, or telemetry
  result that an operator would not see
- numeric conversion narrows kernel counters, flow counters, bandwidth values, or
  timestamps in a way that can wrap, saturate unexpectedly, become non-finite, or
  otherwise lose operational data
- an unsafe block reads caller-provided memory without a size check or serializes
  uninitialized padding bytes

Do not count these as findings by themselves:

- `unwrap` / `expect` in tests, examples, benchmarks, one-shot setup validation,
  or process startup where failure stops boot cleanly
- unsafe FFI wrappers that validate sizes and keep pointer lifetimes local
- protocol fields that are intentionally narrower when the code checks range or
  logs/clamps loss before export
- broad Python exception handling that only preserves backwards-compatible
  tolerance and does not change shaping/auth/security behavior

The audit-file section must include:

- Heading: `Panic, error-handling, and type-loss audit`.
- Date, scope, and exact searches or files reviewed.
- Summary bullets separating confirmed findings, reachability-unknown items, and
  observations/not-findings.
- One subsection per finding with the repo-relative `path:line`, short
  description, exposure/threat, and recommended actions.
- No placeholders, no `...`, and no unresolved `<angle-bracket>` tokens.

## Step 5: Node Manager Privacy, Auth, and XSS Audit

Use this step when reviewing node_manager for missing anonymization of PII,
missing authentication or authorization on data access, browser-storage exposure,
and XSS. Review source files, not generated bundles, unless a generated artifact
is the only shipped source for that behavior.

Start with these surfaces:

```text
src/rust/lqosd/src/node_manager/run.rs
src/rust/lqosd/src/node_manager/static_pages.rs
src/rust/lqosd/src/node_manager/auth.rs
src/rust/lqosd/src/node_manager/ws.rs
src/rust/lqosd/src/node_manager/ws/messages.rs
src/rust/lqosd/src/node_manager/local_api/
src/rust/lqosd/src/node_manager/js_build/src/
src/rust/lqosd/src/node_manager/static2/template.html
docs/v2.0/node-manager-ui.md
docs/v2.0/components.md
```

Use these searches as a starting point:

```text
rg -n "localStorage|sessionStorage|document\\.cookie|innerHTML|outerHTML|insertAdjacentHTML|eval\\(|Function\\(|onclick=|onerror=|sanitize|DOMPurify|redact|redaction|redactable|allow_anonymous|auth_layer|route_layer|LoginResult|ReadOnly|Admin|Denied" src/rust/lqosd/src/node_manager docs/v2.0
rg -n "innerHTML\\s*=.*(\\+|`)|simpleRowHtml\\(|href=.*\\+|data-[^=]+=|textContent|innerText" src/rust/lqosd/src/node_manager/js_build/src --glob '*.js'
rg -n "ShapedDevice|network_json|CircuitById|AllShapedDevices|NetworkJson|Search|UnknownIps|CircuitDirectory|device_name|circuit_name|mac|ipv4|ipv6|comment" src/rust/lqosd/src/node_manager
```

For each candidate, identify:

- the exact `path:line`
- whether access is unauthenticated, anonymous read-only, authenticated read-only,
  or admin-only
- whether the exposed data includes subscriber/customer identifiers, circuit
  names/IDs, device names/IDs, IPs, MACs, comments, topology names, tickets, or
  integration secrets
- whether redaction happens server-side, in the transport payload, or only in the
  browser display
- whether browser storage persists credentials, session tokens, topology drafts,
  dashboard layouts, interface names, VLANs, or other operational data
- whether untrusted strings are inserted with `innerHTML`, HTML tooltips,
  attributes, inline handlers, or URLs without escaping and protocol validation

Count a finding when evidence supports one of these:

- anonymous/demo/read-only access can retrieve raw PII or sensitive operational
  data with no server-side anonymization
- a route, websocket request, local API, static fallback, or file-serving path
  exposes data without the expected auth boundary
- state-changing websocket/local API behavior is available to read-only or
  anonymous users without a documented reason
- operator/customer/integration-controlled strings can reach `innerHTML` or HTML
  attributes without escaping
- an XSS would expose a session token, API key, config secret, localStorage value,
  or pending control-plane operation
- localStorage retains sensitive topology/configuration data beyond the browser
  session or logout without a clear need

Do not count these as findings by themselves:

- static JS/CSS/images served without auth when they contain no operator data,
  credentials, or secrets
- client-side redaction that is documented as screenshot/demo display redaction,
  unless the same mode is used as the privacy boundary for anonymous/public
  access
- admin-only config views that already redact integration secrets before sending
  them to the browser
- `innerHTML` used only for fixed icons, fixed Bootstrap markup, or escaped values

The audit-file section must include:

- Heading: `Node Manager privacy, auth, and XSS audit`.
- Date, scope, exact searches or files reviewed, and any excluded generated/vendor
  output.
- Summary bullets separating confirmed findings, hardening observations, and
  not-findings.
- One subsection per finding with repo-relative `path:line`, short description,
  exposure/threat, and recommended actions.
- A short localStorage/sessionStorage/cookie note, even when no sensitive
  localStorage token is found.
- No placeholders, no `...`, and no unresolved `<angle-bracket>` tokens.

## Validation

- Re-read the audit section before returning; remove placeholders and vague conclusions.
- After changing any repo file, run the repo's anti-slop review path and fix non-zero slop before finishing.
