# Security Audit 1 Tasks

Date: 2026-05-15
Branch goal: move the current security branch from a solid B posture to an A-grade release posture.

Source notes reviewed:

- `SECURITY_AUDIT_2026_05_15_update1.md`
- `SECURITY_AUDIT_2026_05_15_DEEPSEEK.md`

## Scope Rules

- Assume the managed Caddy/HTTPS setup path is used. Do not spend this branch re-solving operator deployment exposure that is already handled by the Caddy setup path.
- Keep this list to repo-owned improvements that can be fixed, tested, or documented in this branch.
- Treat eBPF code size and verifier loadability as hard constraints. Prefer bounded checks, conservative pass behavior, counters, and tests over large parser expansions.
- Do not include OS, kernel, distro package, firewall, or live-host configuration findings in this branch task list.

## P0: A-Ready Blockers

### 1. Finish the remaining packet-capture stored-XSS cleanup in Node Manager

Status: Done. Packet-capture IP/address labels now use DOM text nodes via tested helpers; node_manager tests and build-contract validation passed on 2026-05-15.

Why:

DeepSeek's highest-risk finding is still visible in the source: the circuit packet-capture dropdown concatenates IP addresses into `innerHTML`. Even if the update note intended this class of issue to be fixed, this exact sink should be removed before calling the branch A-ready.

Paths:

- `src/rust/lqosd/src/node_manager/js_build/src/circuit.js`
- `src/rust/lqosd/src/node_manager/js_build/src/circuit_packet_capture_dom.mjs`
- `src/rust/lqosd/src/node_manager/js_build/src/circuit_packet_capture_dom.test.mjs`

Work:

- Replace the packet-capture dropdown construction with DOM text nodes, or add a shared escaping helper and apply it to the IP/address insertions.
- Review remaining `innerHTML` assignments that mix trusted markup with operator-controlled values. Keep static icon markup separate from untrusted text.
- Add a focused regression check for the packet-capture IP path using an IP-like string containing HTML metacharacters.

Done when:

- Packet-capture IP/address labels render through DOM text APIs, not `innerHTML`.
- Node Manager build-contract validation passes.

### 2. Clamp and warn on all remaining 32-bit telemetry narrowing

Status: Done. NetFlow 9 timestamps and LTS2/Insight client-side telemetry conversions now clamp with warnings while preserving current wire field widths; focused Rust tests and `cargo check -p lqosd` passed on 2026-05-15.

Why:

The update audit still calls out NetFlow 9 timestamps and LTS2 stats submission counters that narrow wider values without a documented clamp. This is not an unauthenticated exploit, but A-grade security should avoid silent wraparound in operational telemetry.

Paths:

- `src/rust/lqosd/src/throughput_tracker/flow_data/netflow9/protocol/header.rs`
- `src/rust/lqosd/src/throughput_tracker/stats_submission.rs`
- Insight/LTS2 shared protocol types are out of scope for this branch unless the other end is coordinated.

Work:

- Reuse or share the NetFlow 5 clamp helpers for NetFlow 9 `sys_uptime` and `unix_secs`.
- Add saturation helpers for LTS2 `u64 -> u32`, `u64 -> i32`, and RTT millisecond conversions.
- Preserve the current Insight/LTS2 wire field widths and clamp client side rather than widening protocol fields.
- Log low-rate warnings when saturation happens.
- Document any protocol limit that must remain 32-bit for compatibility.

Done when:

- Values above `u32::MAX`, `i32::MAX`, and non-finite RTT conversions are covered by tests and cannot silently wrap.
- Any required Insight/LTS2 compatibility decision is written down in the audit update.

### 3. Add an auth surface regression gate for Node Manager

Status: Done. Added the missing HTML route build-contract check, websocket same-origin guard, template-page block, and CORS regression test; existing cookie and read-only dashboard tests were rerun on 2026-05-15.

Why:

The branch fixed important auth behavior: no anonymous WebUI mode, CORS removed, session cookies hardened, durable session token no longer copied into websocket auth, and read-only dashboard writes denied. A-grade security needs a regression gate so these do not slip back.

Paths:

- `src/rust/lqosd/src/node_manager/auth.rs`
- `src/rust/lqosd/src/node_manager/run.rs`
- `src/rust/lqosd/src/node_manager/static_pages.rs`
- `src/rust/lqosd/src/node_manager/ws.rs`
- `src/rust/lqosd/src/node_manager/local_api/`

Work:

- Add tests or a build-contract check proving every shipped `.html` page is routed through the authenticated/template path, except explicit login/health/first-run pages.
- Add checks for session cookie flags: `HttpOnly`, `SameSite=Lax`, and conditional `Secure` when SSL/Caddy mode is active.
- Add checks that CORS remains absent or non-credentialed on Node Manager.
- Add read-only authorization regression tests for dashboard theme save/delete and any other state-changing websocket/local-api route touched in this branch.
- Check websocket upgrade `Origin` handling. If cross-origin cookie-authenticated upgrades are accepted, add an Origin allowlist or document and test why the current browser cookie behavior is sufficient.

Done when:

- Auth, cookie, CORS, route whitelist, and read-only write behavior are covered by repeatable tests or build-contract checks.

### 4. Remove panic and silent-failure paths on operator-controlled inputs

Status: Done. UISP subnet/device IP parsing now reports malformed data without panicking, malformed queue snapshots return parse errors, Heimdall/BPF/socket cleanup failures are logged, and XDP/TC detach fallbacks report failures while continuing; focused tests, compile checks, and clippy passed on 2026-05-15.

Why:

DeepSeek found several low-to-medium reliability/security issues where malformed operator files, imported API data, or cleanup failures panic or silently hide an operational failure. These are not direct remote compromise vectors, but they are exactly the sort of hardening work that separates a B from an A.

Paths:

- `src/rust/uisp_integration/src/ip_ranges.rs`
- `src/rust/uisp_integration/src/uisp_types/uisp_device.rs`
- `src/rust/lqos_queue_tracker/src/queue_structure/queue_node.rs`
- `src/rust/lqos_heimdall/src/watchlist.rs`
- `src/rust/lqos_sys/src/bpf_map.rs`
- `src/rust/lqos_sys/src/lqos_kernel.rs`
- `src/rust/lqos_bus/src/bus/unix_socket_server.rs`

Work:

- Replace config/API parse `unwrap()` calls with typed errors or warning-plus-skip behavior.
- Return or log parse errors for malformed `queueStructure.json` instead of silently creating an empty network.
- Log Heimdall watch insert/delete failures.
- Count or log BPF map delete failures during clear operations.
- Log XDP/TC detach fallback failures without changing live behavior.
- Log Unix socket cleanup failures before retry or bind failure.

Done when:

- Malformed operator-managed inputs produce actionable errors or warnings, not panics or silent empty state.
- Cleanup failures are visible in logs without causing unnecessary service disruption.

## P1: Strong A-Grade Hardening

### 5. Add low-cost eBPF backpressure visibility

Status: Done. Flow RTT ring-buffer output failures now increment the existing per-CPU pressure map and `lqosd` logs increases once per polling cycle; the BPF object built, focused Rust checks/clippy passed, and the eBPF load was confirmed on 2026-05-15.

Why:

The RTT `flowbee_events` ring-buffer path still ignores `bpf_ringbuf_output` failure. Heimdall now checks copy/output errors, but drop visibility should be consistent. This should be done with extreme care because recent eBPF changes already hit verifier and size limits.

Paths:

- `src/rust/lqos_sys/src/bpf/common/flows.h`
- `src/rust/lqos_sys/src/bpf/common/heimdall.h`
- `src/rust/lqos_sys/src/bpf/common/throughput.h`
- userspace readers that can expose low-rate pressure counters

Work:

- Add the smallest practical per-CPU drop counter for RTT ring-buffer failures.
- Avoid per-packet logging.
- Reuse existing map-pressure or insert-pressure reporting patterns if possible.
- Verify the BPF object still loads after every change.

Done when:

- Flow RTT event loss is observable under ring-buffer pressure.
- The eBPF program still builds and loads.

### 6. Expand malformed-traffic coverage without growing the hot path recklessly

Status: Done. IPv6 first extension headers/fragments now fail open before lookup or flow tracking on both XDP and TC fallback paths; direct IPv6 TCP/UDP remains lookup-eligible, stacked MPLS policy is covered by fixtures, the fragment policy is documented, focused and full `lqos_sys` tests/check/clippy passed, and eBPF load validation passed on 2026-05-15.

Why:

The branch fixed IPv4 header length, total length, version, and fragment handling, but the audit still points at IPv6 extension headers, IPv6 fragments, MPLS parser coverage, and fragment policy. Full parser expansion may be too expensive for XDP, so the task should be conservative.

Paths:

- `src/rust/lqos_sys/src/bpf/common/dissector.h`
- `src/rust/lqos_sys/src/bpf/common/dissector_tc.h`
- `src/rust/lqos_sys/src/bpf/lqos_kern.c`

Work:

- Add tests or verifier-focused fixtures for IPv6 packets with Hop-by-Hop, Routing, Destination Options, AH, and Fragment headers.
- Prefer "detect and pass without flow tracking" for IPv6 fragments and extension-header cases unless a small bounded parser fits the verifier and size budget.
- Add MPLS stacked-label tests for the current pass behavior.
- Document the fragment policy: what is shaped, what is passed, and why.

Done when:

- Malformed and extension-header traffic has explicit, tested behavior.
- No new eBPF size or verifier regression is introduced.

### 7. Move sensitive browser operational drafts out of durable localStorage

Status: Done. Network-mode drafts and pending-operation state now use tab-scoped `sessionStorage`, legacy durable values are migrated and removed, apply/confirm/revert/rollback/logout clear operational keys, harmless preferences remain durable, and node_manager storage/build-contract validation passed on 2026-05-15.

Why:

The update audit found no durable session tokens in browser storage, which is good. Remaining `localStorage` usage still includes operational drafts and pending network-mode state. On shared operator workstations, A-grade posture should reduce how much deployment detail survives logout or browser restart.

Paths:

- `src/rust/lqosd/src/node_manager/js_build/src/config_interface.js`
- dashboard preference storage under `src/rust/lqosd/src/node_manager/js_build/src/lq_js_common/dashboard/`
- template/login logout paths that can clear storage

Work:

- Classify each `localStorage` key as harmless preference, short-lived operational draft, or sensitive deployment detail.
- Move short-lived operational draft/pending-operation state to `sessionStorage` where practical.
- Clear sensitive draft keys on confirm, rollback, cancel, logout, and page unload where appropriate.
- Keep harmless user preferences such as theme and colorblind mode durable.

Done when:

- No interface, VLAN, pending operation, or deployment-change draft persists indefinitely in `localStorage`.
- Harmless preferences still work across reloads.

### 8. Add a Content-Security-Policy appropriate for the current UI

Status: Done. Authenticated templated pages and standalone login/first-run pages now emit an enforced CSP that keeps scripts self-hosted, preserves the existing inline template/style requirements, permits current websocket/API-docs/Insight tile needs, blocks framing, and is covered by Rust CSP tests plus `lqosd` check/clippy and node_manager build-contract validation on 2026-05-15.

Why:

CSP will not replace XSS fixes, but it reduces blast radius if a future trusted-HTML sink regresses. The current UI uses Bootstrap, FontAwesome, local scripts, and some inline bootstrap/theme snippets, so the policy should be practical rather than aspirational.

Paths:

- `src/rust/lqosd/src/node_manager/template.rs`
- `src/rust/lqosd/src/node_manager/static2/template.html`
- login and first-run pages if they bypass the main template

Work:

- Start with a report-only policy if needed.
- Prefer self-hosted scripts/styles and avoid enabling remote script sources.
- Keep any required inline allowances documented and minimized.
- Add a smoke test that the main UI pages still render.

Done when:

- A CSP header is present on authenticated UI pages without breaking the existing Node Manager.

### 9. Keep Caddy/API loopback behavior guarded, but do not duplicate setup work

Status: Done. Managed Caddy Caddyfile tests now assert loopback API/WebUI upstreams and runtime enable/disable tests continue to preserve `lqos_api` restarts; operator docs now identify `LQOS_API_LISTEN` as an advanced override that should stay unset on managed Caddy systems unless direct API exposure is intentional and separately protected.

Why:

The branch assumes Caddy is set up. The useful repo-owned hardening is to prevent accidental drift from the intended Caddy/loopback shape, not to treat Caddy setup itself as missing.

Paths:

- `src/rust/lqos_setup/src/ssl.rs`
- `src/rust/lqos_setup/src/web.rs`
- `src/rust/lqosd/src/node_manager/local_api/ssl.rs`
- sibling `lqos_api` listener code if coordinated in this branch

Work:

- Keep tests proving Caddy mode writes a loopback API listener and restarts the API service when the mode changes.
- If `LQOS_API_LISTEN` remains an advanced override, make the override noisy when it defeats Caddy loopback isolation.
- Document that the override is for advanced deployments and should not be set on managed Caddy systems.
- Do not make the first-run setup flow unusable for headless installs unless there is an explicit product decision.

Done when:

- Managed Caddy mode has tests and docs that preserve loopback API isolation.
- Any override that bypasses it is intentional and visible.

## P2: Audit Evidence and Release Gate

### 10. Close the audit with repeatable evidence

Why:

The final A grade should be based on proof, not just code inspection.

Work:

- Run the focused Rust checks for touched crates.
- Run Node Manager build-contract validation after frontend changes.
- Run `cargo audit`, `cargo machete`, and a dependency tree check after dependency or protocol changes.
- For eBPF changes, verify build and load behavior in the same way used to confirm the current eBPF fix.
- Update the audit document with exact commands, outcomes, and remaining accepted risks.

Done when:

- The branch has an evidence-backed security summary that explains why the remaining risks are accepted for an A grade.

## Deferred Unless Time Allows

- Do not retire `serde_cbor` in this security branch; the Insight/LTS2 protocol counterpart makes that too risky without coordinated work.
- Replace other unmaintained crates (`fxhash`, `bincode`, transitive `paste`) only when the migration is small or already planned. Track them as maintenance if they do not carry an active advisory.
- Add optional server-side redaction for read-only staff accounts as a product privacy feature. This is useful, but not required to preserve the intended trusted-support-staff model.
- Align userspace flow/host cache capacities with BPF LRU behavior if it can be done without destabilizing flow reporting. Treat this as observability correctness, not an A-blocking security issue.
