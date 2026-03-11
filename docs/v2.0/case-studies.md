# Case Studies (Anonymized)

This page collects qualitative, anonymized operator stories for adoption guidance.

Anonymization policy used here:

- Geography is kept at region/continent level only.
- Subscriber counts are shown as bands.
- Product/integration names may be included.
- No uniquely identifying network details are included.

## Story 1: Regional WISP Standardized on UISP Integration

- Region: North America
- Scale band: 1,000-5,000 subscribers
- Deployment pattern: [WISP/FISP integration recipe](recipes-wisp-fisp-integration.md)

Situation:
- Frequent subscriber plan changes were creating drift between intended and active shaping behavior.

Approach:
- Adopted built-in UISP integration as durable source of truth.
- Standardized on integration-owned `ShapedDevices.csv` with explicit overwrite policy.
- Started with moderate hierarchy depth before considering deeper topology.

Outcome:
- Fewer manual corrections after plan changes.
- Faster onboarding for operations staff.
- More predictable queue behavior after recurring sync cycles.

## Story 2: Maritime Operator Stabilized Quality on Variable WAN

- Region: global routes across multiple ocean regions
- Scale band: 500-1,000 active client endpoints
- Deployment pattern: [Maritime StormGuard recipe](recipes-maritime-stormguard.md)

Situation:
- WAN capacity variability caused recurring quality swings during peak periods.

Approach:
- Modeled vessel traffic under a single top-level `Ship` node.
- Enabled StormGuard in dry-run, then moved to live bounded adjustments.
- Monitored debug/status views during busy windows.

Outcome:
- Better quality resilience during congestion events.
- Clearer operational visibility into adaptive limit decisions.
- Safer change process through staged dry-run rollout.

## Story 3: Hospitality Network Shifted to Per-Device Fairness

- Region: Europe
- Scale band: 500-1,000 rooms / 1,000-5,000 device endpoints
- Deployment pattern: [Hospitality per-device recipe](recipes-hospitality.md)

Situation:
- Shared room-level shaping led to fairness complaints in high-occupancy periods.

Approach:
- Moved to per-device circuit mapping for managed address pools.
- Kept hierarchy shallow and parent naming stable.
- Tracked memory and queue/class pressure before broader rollout.

Outcome:
- Improved perceived fairness across concurrently active guest devices.
- Better troubleshooting granularity at support desk level.
- Clearer capacity planning signals for peak occupancy periods.

## Related Pages

- [Deployment Recipes](recipes.md)
- [System Requirements](requirements.md)
- [Scale Planning and Topology Design](scale-topology.md)
