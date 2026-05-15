# LibreQoS Security Audit Update 2 - 2026-05-15

## Introduction

This update re-conducts the 2026-05-15 LibreQoS security review after the hardening work completed on the security branch. It supersedes the posture described in `SECURITY_AUDIT_2026_05_15.md` and updates the follow-up findings in `SECURITY_AUDIT_2026_05_15_update1.md`.

The review scope followed the LibreQoS security-audit skill and focused on:

- Rust dependency posture in `src/rust`.
- Network control-plane exposure in LibreQoS, Node Manager, setup/Caddy integration, and the read-only sibling `../../lqos_api`.
- Bridged-interface and eBPF malformed-traffic behavior.
- Panic, error-handling, and numeric type-loss risks in security-relevant paths.
- Node Manager authentication, authorization, privacy, and browser-side injection risks.

The audit assumes the documented Caddy/HTTPS option is installed and configured by the operator. Host firewall posture, kernel CVEs, Ubuntu package state, Caddy internals, and live-host network policy are outside this repo-level review.

## Executive Summary

**Current security grade: A**

LibreQoS is now in a strong security posture for a release candidate. The branch closed the high-impact risks from the original audit without changing the Insight/LTS2 protocol or retiring `serde_cbor`. The remaining items are accepted maintenance or deployment risks rather than active release-blocking vulnerabilities.

The original audit graded LibreQoS at **C** because it found several concrete risks: excessive API exposure in common HTTPS mode, browser-side token handling, insufficient login throttling, eBPF malformed-packet verifier/runtime risks, unchecked LTS2 numeric narrowing, and Node Manager injection/privacy gaps. `update1` moved the project to **B** after initial remediation. This update raises the grade to **A** because the security branch now has:

- Loopback defaults and tests for the Caddy/API mode.
- Hardened Node Manager authentication, session, authorization, CSP, and packet-capture DOM handling.
- eBPF verifier-safe packet parsing, bounded maps, map-pressure reporting, and successful real load validation.
- LTS2/Insight client-side numeric clamps while preserving the existing protocol.
- Focused regression tests and branch evidence for the mitigations.

No new critical or high security findings were confirmed in this pass.

## Rust Dependency Audit

Commands run from `src/rust`:

- `cargo audit`
- `cargo machete`
- `cargo tree --locked --workspace --depth 1`

`cargo audit` reported no active vulnerability findings. It reported four allowed maintenance warnings:

- `bincode 1.3.3` is unmaintained.
- `fxhash 0.2.1` is unmaintained.
- `paste 1.0.15` is unmaintained.
- `serde_cbor 0.11.2` is unmaintained.

These are maintenance concerns, not confirmed exploitable findings in this review. `serde_cbor` is intentionally retained because the LTS2/Insight bus protocol must remain compatible with deployed peer software. Replacing it would require coordinated changes on both sides and is out of scope for this branch.

`cargo machete` found no unused Rust dependencies. The shallow workspace dependency tree did not show a new suspicious dependency expansion caused by the security work.

### Findings

No dependency issue found in this pass requires blocking the release.

### Accepted Risks

- `serde_cbor` remains an accepted compatibility dependency until LTS2/Insight can coordinate a protocol migration.
- Other unmaintained crates should stay on the maintenance backlog, but they do not currently prevent an A security grade.

## Network Control-Plane Audit

The Caddy setup path now uses loopback upstreams for HTTPS mode:

- Runtime secure Node Manager listener: `127.0.0.1:9123`.
- API upstream behind Caddy: `127.0.0.1:9122`.
- Caddy proxies API requests to the loopback API upstream and the web UI to the loopback web upstream.
- Tests verify that the generated Caddyfile does not expose API upstreams as `:9122`, `0.0.0.0`, or `[::]`.

The read-only sibling `lqos_api` now has a safer listen-address policy:

- Direct/non-Caddy mode still defaults to `:::9122`.
- Caddy mode defaults to `127.0.0.1:9122`.
- `LQOS_API_LISTEN` remains an explicit advanced override.
- The API authentication middleware handles malformed `x-bearer` headers without panicking.

The remaining direct-listen default is an intentional operational mode. It is not graded as a vulnerability by itself because listener exposure depends on the deployment mode and network policy. In the documented Caddy/HTTPS path, the API is loopback-bound behind Caddy.

### Findings

No high-impact control-plane exposure remains in the documented Caddy/HTTPS deployment path.

### Accepted Risks

- Operators can still override `LQOS_API_LISTEN` into an exposed address. That is a deliberate escape hatch and should remain documented as advanced configuration.
- Direct mode remains more exposed than Caddy mode by design. Operators using it should protect the API with host/network policy.

## Bridged-Interface and eBPF Audit

The eBPF side has been hardened materially since the original review:

- IPv4 parsing validates version, IHL, total length, packet bounds, fragmentation, and minimum TCP/UDP/ICMP header presence before classifying.
- IPv6 extension headers that would require deeper parsing are passed unshaped instead of being force-classified incorrectly.
- Traffic and flow maps now use bounded LRU map types where appropriate.
- Host-map pressure and flow-event ring-buffer output failures are tracked and surfaced to userspace.
- Heimdall packet capture now uses verifier-visible size clamps and checks helper/ring-buffer return values.
- eBPF debug logging compiles out unless verbose/tracing builds enable it.
- The operator confirmed that the eBPF program loaded successfully with no verifier validation errors.

This is a large improvement from the original branch state. It also keeps the implementation realistic for eBPF: the program remains constrained by verifier complexity and instruction/state limits, so deeper packet handling is deliberately avoided when it would create more risk than benefit.

### Findings

No current eBPF verifier, malformed-packet, or unbounded-map security finding remains open from this review.

### Accepted Risks

- Some IPv6 extension-header traffic is deliberately passed rather than deeply parsed. That is a conservative security choice and should be documented as behavior, not treated as a defect.
- Future eBPF additions must remain small and verifier-conscious. The earlier E2BIG failure shows there is little room for broad in-kernel logic.

## Panic, Error-Handling, and Type-Loss Audit

The previous security-relevant type-loss risks have been addressed:

- NetFlow 9 uptime and timestamp fields are clamped before conversion to protocol-sized fields.
- LTS2/Insight metric submissions clamp client-side before narrowing to existing protocol integer fields.
- Non-finite RTT and rate values are handled before submission.
- Cake drops, cake marks, circuit retransmits, site retransmits, and related counters use bounded conversions.

The review still found legacy `unwrap`/`expect` usage in tests, importer assumptions, and some non-hot-path setup or integration code. Those are not equivalent to the original packet/control-plane risks. The security-relevant hot paths reviewed here now favor clamping, checked parsing, logged failures, or explicit fallback behavior.

### Findings

No current release-blocking panic or numeric narrowing finding remains in the reviewed packet, control-plane, and LTS2 submission paths.

### Accepted Risks

- Some legacy importer paths still assume internally consistent UISP graph data. Those should be cleaned up incrementally, but they were not confirmed as remotely exploitable security bugs in this pass.
- The LTS2/Insight protocol still carries narrower fields than some local counters. Client-side clamping is the correct mitigation until a coordinated protocol revision is possible.

## Node Manager Privacy, Auth, and XSS Audit

Node Manager is now in a substantially stronger browser and session posture:

- Login cookies are `HttpOnly`, `SameSite=Lax`, scoped to `/`, and marked `Secure` when the request is secure.
- No websocket token is stored in browser storage.
- Anonymous Node Manager access is denied.
- Read-only users are blocked from write-capable websocket actions.
- Repeated login failures are rate-limited and logged.
- Security headers are applied to templated and standalone pages.
- A Content Security Policy is now present for Node Manager pages.
- Packet-capture DOM rendering has regression tests proving malicious address strings are rendered as text, not executable markup.
- Dashboard theme updates validate known themes and require admin privileges.
- Network-mode state moved to session storage, and legacy local-storage state is cleared on logout.

Some UI code still uses `innerHTML` for static or internally formatted markup, so continued care is needed when adding new dynamic content. The specific user-controlled packet-capture path that motivated the original XSS concern is now covered by tests.

### Findings

No current Node Manager authentication, authorization, token-storage, or packet-capture XSS finding remains open from this review.

### Accepted Risks

- The CSP still permits inline behavior needed by the current UI. Tightening it further would be a worthwhile future hardening step, but it is not required for the A grade.
- Read-only users can still see operational data by design. That is a product authorization decision, not a bypass.

## Improvements Made Since `SECURITY_AUDIT_2026_05_15.md`

- Updated `rand` and cleaned up the original dependency concern without adding unused workspace dependencies.
- Removed the unnecessary direct `tokio` dependency from `lqos_config`.
- Removed permissive Node Manager CORS behavior.
- Added Node Manager login rate limiting and failed-login logging.
- Hardened Node Manager cookies with `HttpOnly`, `SameSite=Lax`, scoped path, and conditional `Secure`.
- Removed browser-stored websocket token exposure.
- Denied anonymous Node Manager access by default.
- Added read-only websocket authorization regression coverage for write-capable actions.
- Added Node Manager security headers and CSP coverage.
- Fixed packet-capture DOM rendering to avoid user-controlled HTML injection, with regression tests.
- Restricted dashboard theme changes to validated known themes and admin users.
- Moved network-mode state from persistent local storage to session storage and clears legacy state on logout.
- Added Caddy/HTTPS loopback defaults for Node Manager and API upstreams.
- Added Caddyfile tests preventing accidental exposed API upstream generation.
- Added API listen-address logic in `lqos_api` that defaults to loopback when Caddy mode is detected.
- Hardened malformed `x-bearer` handling in `lqos_api` so invalid headers do not panic.
- Kept the LTS2/Insight protocol compatible and clamped unsafe values client-side.
- Added NetFlow 9 timestamp and uptime clamps.
- Added LTS2/Insight metric clamps for rates, RTT, retransmits, cake drops, and cake marks.
- Compiled eBPF debug logging out of normal builds.
- Added IPv4 malformed-packet checks for version, IHL, total length, fragments, and minimum transport headers.
- Added conservative IPv6 extension-header pass-through behavior.
- Converted traffic and flow maps to bounded LRU map types where appropriate.
- Added map-pressure and ring-buffer failure accounting for eBPF/user-space visibility.
- Added Heimdall capture-size and helper-return checks.
- Resolved the eBPF verifier issues and confirmed successful eBPF load with no validation errors.
- Added focused tests and branch evidence across the security-sensitive changes.

## Conclusion

LibreQoS now earns an **A** for this repo-level security review. The branch has closed the original high-impact findings while preserving operational compatibility with Insight/LTS2 and avoiding risky protocol churn.

The project is not "done with security"; no real system is. The remaining work is backlog hardening: plan a future protocol migration away from unmaintained serialization once both ends can change, keep tightening CSP as the UI allows, continue removing legacy importer panics, and be cautious with any future eBPF expansion. None of those items block the current security branch from being considered release-grade.
