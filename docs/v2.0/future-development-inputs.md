# Future Development Inputs

This page summarizes recurring operator feedback patterns reviewed for LibreQoS, supported by NLNet deliverable work.

## Scope and Method

Review window:

- March 1, 2024 through March 1, 2026 (UTC)

Input sources:

- GitHub issues from the LibreQoS repository
- Zulip channels:
  - `6-Community-Help-Chat`
  - `1-general`
  - `17-LibreQoS-Support-(Requires-Insight)`
- Additional operator support streams used for technical pattern validation

Method update:

- Message-level review was performed on raw Zulip message text (not topic names alone).
- Findings were normalized into recurring symptom patterns and sanitized for public documentation.

Reviewed volume:

- 139 GitHub issues
- 10,275 Zulip messages

## Recurring Themes

These themes are intended to help prioritize future work. They are not release commitments.

## 1) Source-of-truth safety and integration data hygiene

Example symptoms:

- Duplicate IP assignments causing reload failures or partial shaping
- Parent-node mismatches and invalid topology references
- Confusion around overwrite ownership for `network.json` and `ShapedDevices.csv`
- Edge cases in CRM/NMS matching and stale assignment state

Representative issues:

- [#860 - always_overwrite_network_json default behavior confusion](https://github.com/LibreQoE/LibreQoS/issues/860)
- [#899 - UISP: always_overwrite_network_json=false and missing ShapedDevices.csv](https://github.com/LibreQoE/LibreQoS/issues/899)
- [#845 - UISP: multi-services with same site name](https://github.com/LibreQoE/LibreQoS/issues/845)
- [#699 - UISP: trailing spaces break matching](https://github.com/LibreQoE/LibreQoS/issues/699)

## 2) Startup reliability and onboarding friction

Example symptoms:

- Scheduler/service startup failures after reboot/update
- Missing dependencies, file ownership mismatches, or service ordering races
- First-run install and mode-selection confusion (bridge/single-interface assumptions)
- Setup workflow breakpoints that reduce operator confidence early

Representative issues:

- [#859 - Broken default ShapedDevices.csv from setup tool](https://github.com/LibreQoE/LibreQoS/issues/859)
- [#858 - Config tool webusers flow break](https://github.com/LibreQoE/LibreQoS/issues/858)
- [#728 - Default installation bridge-mode mismatch](https://github.com/LibreQoE/LibreQoS/issues/728)
- [#667 - Add YAML creation to setup installer](https://github.com/LibreQoE/LibreQoS/issues/667)

## 3) Topology/path modeling, scale guardrails, and operator control

Example symptoms:

- Deep hierarchy pressure and queue complexity under scale
- Dashboard "stuck/loading" or confusing UI states tied to topology validity problems
- Need for stronger validation and warnings for parent/path correctness
- Multi-edge and failover environments requiring explicit operator-controlled path intent

Representative issues:

- [#913 - Tree verbosity and HTB depth pressure](https://github.com/LibreQoE/LibreQoS/issues/913)
- [#856 - Improve no-parent circuit warnings](https://github.com/LibreQoE/LibreQoS/issues/856)
- [#801 - Visible warning for TC ID overflow](https://github.com/LibreQoE/LibreQoS/issues/801)
- [#920 - Tree Overview shows blank on low-traffic boxes](https://github.com/LibreQoE/LibreQoS/issues/920)

## 4) Performance fit, hardware profile, and runtime stability

Example symptoms:

- Throughput shortfalls on unsupported NICs or low single-thread CPUs
- Reload-related instability under high churn
- Memory-growth concerns and high-scale capacity planning pressure
- MTU/encapsulation mismatches that mimic shaping faults

Representative issues:

- [#928 - Detect e-cores and avoid shaping load there](https://github.com/LibreQoE/LibreQoS/issues/928)
- [#651 - Memory growth even when gather_stats is false](https://github.com/LibreQoE/LibreQoS/issues/651)
- [#578 - Resource footprint for 100 Gbit fiber links](https://github.com/LibreQoE/LibreQoS/issues/578)
- [#526 - Performance improvements during reloading](https://github.com/LibreQoE/LibreQoS/issues/526)

## Candidate Directions Under Evaluation

1. Add stronger pre-flight validation for integration-managed deployments.
2. Improve source-of-truth ownership visibility and overwrite safety cues.
3. Harden startup reliability and installer default-path checks.
4. Expand topology linting and parent/path correctness guardrails.
5. Improve operator-facing diagnostics for "loading/blank" UI states.
6. Extend hardware-fit and peak-load guidance by deployment profile.

## Out of Scope

- This page is a planning input summary, not a roadmap commitment.
- Inclusion of an issue does not guarantee a release target or implementation date.

## Related Pages

- [Deployment Recipes](recipes.md)
- [Case Studies (Anonymized)](case-studies.md)
- [Troubleshooting](troubleshooting.md)
