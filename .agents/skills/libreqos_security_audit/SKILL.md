---
name: libreqos_security_audit
description: Repo-local LibreQoS workflow for the Rust dependency step of security audit passes. Use when auditing LibreQoS before a release with cargo audit, cargo machete, cargo tree, CVE triage, and audit-file findings updates.
---

# LibreQoS Security Audit

Use this skill for the recurring Rust dependency step of LibreQoS security
audit passes in this repo. This skill does not cover the whole product security
audit; use additional focused checks for Python, web UI, packaging, live-host
configuration, secrets, authentication, and network-facing behavior.

## Scope

- Keep the audit repo-local. Do not install or update global skills.
- Start from the current repo checkout and respect dirty worktree boundaries.
- Write results into the requested audit file. If no file is named, ask before creating one.
- Distinguish security findings from maintenance warnings. "Not maintained" is not a security issue by itself.
- Distinguish reachable vulnerabilities from unused-feature or irrelevant-feature advisories. A vulnerability in a feature LibreQoS does not use is not a security finding by itself.

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

## Validation

- Re-read the audit section before returning; remove placeholders and vague conclusions.
- After changing any repo file, run the repo's anti-slop review path and fix non-zero slop before finishing.
