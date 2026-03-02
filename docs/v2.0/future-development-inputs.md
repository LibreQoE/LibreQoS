# Future Development Inputs

This page summarizes recurring operator feedback inputs reviewed for LibreQoS, supported by NLNet deliverable work.

## Scope and Method

Review window:

- March 1, 2024 through March 1, 2026 (UTC)

Input sources:

- GitHub issues from the LibreQoS repository
- Community support channels

Reviewed volume:

- 139 GitHub issues
- 10,275 community-channel messages

## Recurring Themes

These themes are intended to help prioritize future work. They are not release commitments.

## 1) UI clarity and operator confidence

Example symptoms:

- Blank or partially empty data views
- Missing context when diagnosing state
- Inconsistent status cues across screens

Representative issues:

- [#922 - Flowmap doesn't render](https://github.com/LibreQoE/LibreQoS/issues/922)
- [#921 - ASN Explorer dropdowns are empty](https://github.com/LibreQoE/LibreQoS/issues/921)
- [#920 - Tree Overview shows blank on low-traffic boxes](https://github.com/LibreQoE/LibreQoS/issues/920)
- [#831 - Dashboard goes blank when used by same user in multiple locations](https://github.com/LibreQoE/LibreQoS/issues/831)

## 2) Integration behavior and source-of-truth safety

Example symptoms:

- Confusion around overwrite ownership of shaping files
- Integration matching edge cases
- Unclear defaults during integration onboarding

Representative issues:

- [#860 - always_overwrite_network_json default behavior confusion](https://github.com/LibreQoE/LibreQoS/issues/860)
- [#899 - UISP: always_overwrite_network_json=false and missing ShapedDevices.csv](https://github.com/LibreQoE/LibreQoS/issues/899)
- [#845 - UISP: multi-services with same site name](https://github.com/LibreQoE/LibreQoS/issues/845)
- [#699 - UISP: trailing spaces break matching](https://github.com/LibreQoE/LibreQoS/issues/699)

## 3) Onboarding and early deployment friction

Example symptoms:

- First-run setup breakpoints
- Startup/configuration workflow confusion
- Installer/default behavior mismatches

Representative issues:

- [#859 - Broken default ShapedDevices.csv from setup tool](https://github.com/LibreQoE/LibreQoS/issues/859)
- [#858 - Config tool webusers flow break](https://github.com/LibreQoE/LibreQoS/issues/858)
- [#728 - Default installation bridge-mode mismatch](https://github.com/LibreQoE/LibreQoS/issues/728)
- [#667 - Add YAML creation to setup installer](https://github.com/LibreQoE/LibreQoS/issues/667)

## 4) Scale/topology guardrails and proactive warnings

Example symptoms:

- Queue depth pressure in complex hierarchies
- Overflow-risk visibility needs
- Parent-node hygiene and topology clarity concerns

Representative issues:

- [#913 - Tree verbosity and HTB depth pressure](https://github.com/LibreQoE/LibreQoS/issues/913)
- [#801 - Visible warning for TC ID overflow](https://github.com/LibreQoE/LibreQoS/issues/801)
- [#856 - Improve no-parent circuit warnings](https://github.com/LibreQoE/LibreQoS/issues/856)
- [#560 - htb too many events under load](https://github.com/LibreQoE/LibreQoS/issues/560)

## 5) Performance and hardware-fit tuning

Example symptoms:

- Core utilization behavior on heterogeneous CPU platforms
- Memory growth concerns
- Throughput/headroom fit on different hardware classes

Representative issues:

- [#928 - Detect e-cores and avoid shaping load there](https://github.com/LibreQoE/LibreQoS/issues/928)
- [#651 - Memory growth even when gather_stats is false](https://github.com/LibreQoE/LibreQoS/issues/651)
- [#578 - Resource footprint for 100 Gbit fiber links](https://github.com/LibreQoE/LibreQoS/issues/578)
- [#526 - Performance improvements during reloading](https://github.com/LibreQoE/LibreQoS/issues/526)

## Candidate Directions Under Evaluation

1. Improve guided diagnostics and empty-state clarity in WebUI.
2. Make source-of-truth ownership and overwrite behavior more explicit.
3. Expand integration pre-flight validation and edge-case handling.
4. Strengthen scale guardrails and early warning ergonomics.
5. Expand deployment runbooks for common architecture patterns.
6. Improve performance-fit guidance by hardware and topology profile.

## Out of Scope

- This page is a planning input summary, not a roadmap commitment.
- Inclusion of an issue does not guarantee a release target or implementation date.

## Related Pages

- [Deployment Recipes](recipes.md)
- [Case Studies (Anonymized)](case-studies.md)
- [Troubleshooting](troubleshooting.md)
