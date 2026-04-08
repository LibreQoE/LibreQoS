# Recipe: Education / University Per-IP Shaping for Stable Real-Time Calls

Use this pattern when a school, college, or university wants to reduce Zoom/Teams/WebRTC freezing and lag during busy windows by shaping per IP.

## Fit

- Best for: campuses where endpoint fairness matters and client IP inventory can be refreshed on a schedule.
- Avoid when: the institution cannot maintain reliable IP-to-parent mapping updates.

## Why Per-IP in Education Networks

Per-IP shaping is usually the right default in education environments because:

1. student and staff traffic is highly mixed and bursty
2. real-time conferencing and bulk traffic often compete on the same uplinks
3. per-IP circuits reduce flow domination and improve fairness during peak contention

## LibreQoS Behavior Assumptions

This recipe assumes current LibreQoS behavior:

1. shaped circuits typically use `HTB` + `cake diffserv4`
2. undefined traffic is pass-through (not moved into an HTB default queue)
3. per-IP coverage quality directly affects how much traffic receives shaping

## Per-IP `ShapedDevices.csv` Design Patterns

Treat `ShapedDevices.csv` as generated data from your local automation, not a hand-maintained spreadsheet.

Recommended patterns:

1. one circuit per managed client IP
2. stable `Circuit ID` format so updates are deterministic (for example `<site>-<vlan>-<ip>`)
3. stable `Parent Node` mapping to operational bottleneck domains (for example building/floor/AP group)
4. explicit stale-entry policy for DHCP churn (remove IPs no longer present)
5. duplicate-IP and conflicting-parent checks before publishing updates
6. deterministic output ordering so diffs are clean and reviewable

Example row:

```text
Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment,sqm
UNI-BLDG-A-10.40.12.77,BLDG-A-10.40.12.77,UNI-BLDG-A-10.40.12.77,Host-10.40.12.77,BLDG-A,,10.40.12.77,,1,1,150,60,Education per-IP circuit,cake
```

## Script-Your-Own Sync Workflow (Meraki/UniFi/Custom)

Most education institutions should script this themselves because each network is unique.

Suggested workflow:

1. Collect source data from your systems (for example Meraki Dashboard API, UniFi Network API, DHCP/IPAM exports).
2. Normalize into a single intermediate model:
   - client IP
   - parent/bottleneck domain
   - optional role metadata (staff lab, student dorm, classroom, voice endpoint)
3. Generate deterministic `ShapedDevices.csv` output.
4. Run pre-publish checks:
   - duplicate IP detection
   - empty/missing parent checks
   - unexpected add/remove volume thresholds
5. Compare generated output to current production file and review the diff.
6. Publish updates on a fixed cadence with logging (for example every 5-15 minutes, based on DHCP churn and operational tolerance).
7. Keep last-known-good file for rapid rollback.

Operational guidance:

- Start simple. Reliable coverage beats over-complex enrichment.
- Prefer predictable, reversible sync jobs over ad-hoc manual edits.
- Alert on sync failures so gaps do not silently create pass-through traffic.

## DSCP and `diffserv4` Guidance for Conferencing

With `cake diffserv4`:

1. preserve trusted DSCP markings where your policy allows
2. if you remark traffic, keep rules narrow and intentional
3. avoid broad over-prioritization that starves normal campus traffic
4. remember unmarked traffic still works, but priority behavior depends on effective codepoints

See [HTB + fq-codel / CAKE](htb_fq_codel_cake.md) for full queueing details.

## Rollout Playbook

1. Select one pilot domain (for example one building or dorm).
2. Enable per-IP generated circuits for that domain only.
3. Observe busy-hour behavior and support-ticket trends.
4. Expand in phases to additional domains.
5. Keep phased rollback boundaries so you can revert one domain at a time if needed.

## Validation Checklist (UI-Only)

Validate in LibreQoS UI (no Linux `tc` CLI required):

1. No new errors or urgent health warnings after sync/apply.
2. Expected circuit counts appear for the pilot scope.
3. Dashboard views reflect the intended shaped domains.
4. Call-quality complaint rate trends down during peak windows.
5. No large unexplained growth in undefined/pass-through traffic indicators.

## Troubleshooting Matrix (UI-First)

| Symptom | Common cause | Check in UI | Corrective action |
|---|---|---|---|
| Zoom/Teams freezing at peak | Coverage gaps from missing IPs, so traffic bypasses shaping | Errors/warnings, circuit counts, affected-domain dashboards | Fix sync coverage, republish `ShapedDevices.csv`, validate counts |
| Audio robotic/choppy while throughput seems fine | Mixed-flow contention and priority mismatch | Domain/circuit behavior in peak windows | Refine DSCP policy and verify `cake diffserv4` assumptions |
| One dorm/building degrades others | Parent grouping does not match real contention domains | Parent-node distribution and impacted-domain views | Adjust parent mapping model in sync logic |
| Quality regresses after DHCP churn events | stale or duplicated entries in generated data | sudden count shifts after sync cycles | tighten stale cleanup and duplicate checks |
| Intermittent improvements only | sync cadence too slow for address churn | timing correlation between sync and incidents | increase sync cadence and add failure alerts |

## Related Pages

- [Deployment Recipes](recipes.md)
- [Advanced Configuration Reference](configuration-advanced.md)
- [HTB + fq-codel / CAKE](htb_fq_codel_cake.md)
- [Scale Planning and Topology Design](scale-topology.md)
- [Troubleshooting](troubleshooting.md)
