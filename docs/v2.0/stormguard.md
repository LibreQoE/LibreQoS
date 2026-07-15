# StormGuard

StormGuard is LibreQoS' adaptive queue-adjustment subsystem for congestion and quality events.

> **Important Scope Warning**
> StormGuard is intended for specific use cases, such as controlling congestion on variable-bandwidth WAN links (for example maritime networks), or a small number of access points with highly variable capacity.
> It is not intended to manage dozens or hundreds of nodes at the same time.

## What StormGuard Does

- Monitors real-time signals (throughput, RTT/loss-related metrics, and saturation context).
- Applies bounded adjustments to configured node limits to protect quality under stress.
- Exposes status/debug data in the WebUI (Node Manager).

StormGuard adaptive site-speed changes are stored in the StormGuard override layer. They are not written back into `network.json`.

## Configuration

StormGuard is configured in `/etc/lqos.conf` under `[stormguard]`.

Common keys:

- `enabled`: turns StormGuard on/off.
- `dry_run`: calculate decisions without applying live queue changes.
- `targets`: list of top-level node names to manage.
- `minimum_download_percentage`: minimum floor ratio for download limits.
- `minimum_upload_percentage`: minimum floor ratio for upload limits.
- `log_file`: optional CSV output path for decision/change telemetry.

Example:

```toml
[stormguard]
enabled = true
dry_run = true
log_file = "/var/log/stormguard.csv"
targets = ["SITE_A", "SITE_B"]
minimum_download_percentage = 0.5
minimum_upload_percentage = 0.5
```

If you are testing, start with `dry_run = true` so you can observe decisions before allowing live limit changes.

Disabling StormGuard, or changing an active deployment back to `dry_run = true`, restores its managed queues to their configured rates and ceilings and removes StormGuard's persisted adaptive overrides. Operator-managed overrides are not changed. On startup, this cleanup can run before Bakery finishes normal queue initialization, but only for live classes that match StormGuard's persisted ownership record and the current shaping-tree generation. Cleanup waits during a full reload and retains its ownership record until Bakery confirms the restoration.

## UI and Debugging

- WebUI provides a dedicated StormGuard dashboard tab plus status and debug views.
- The StormGuard dashboard tab is intended to answer "what is StormGuard doing right now?" with:
  - summary cards for watched, cooling-down, and recently changed sites
  - a site list that works for single-site and multi-site watched sets
  - a selected-site detail panel explaining current limits, last actions, and why StormGuard is holding or changing rates
  - a recent activity feed for quick operator triage
- The StormGuard debug page shows:
  - current effective limits
  - evaluation metrics
  - rule/decision context
- The Network Tree shows a contextual **StormGuard** tab while StormGuard is enabled or cleanup/degraded state remains. Select a watched node to see its current download/upload limits, bounds, strategy, cooldown, decision reason, last outcome, and a browser-local five-minute graph. The tab explains when the selected node is not managed and links to StormGuard configuration for edits.

Runtime health is reported as one of these states:

- `disabled`: StormGuard is off and no cleanup remains.
- `initializing`: configuration, topology, or Bakery dependencies are not ready yet.
- `dry_run`: decisions are being evaluated without live queue changes.
- `live`: acknowledged live adjustments are allowed.
- `cleanup_pending`: owned queue state still needs restoration.
- `degraded`: an error prevents normal evaluation or cleanup; inspect the displayed last error and service log.

## Diagnostic Log

When `log_file` is configured, StormGuard appends one semicolon-delimited row per watched site and direction each second. The first field is a schema version. The version 1 header is:

```text
schema_version;timestamp_unix_ms;site;direction;mode;strategy;queue_mbps;min_mbps;max_mbps;throughput_mbps;throughput_ma_mbps;retransmit_fraction;retransmit_ma;passive_rtt_ms;active_ping_rtt_ms;active_ping_target;active_ping_weight;effective_rtt_ms;rtt_ma_ms;baseline_rtt_ms;delay_ms;passive_rtt_flow_count;decision_score;candidate_action;candidate_target_mbps;decision_reason;decision_blocker;state;cooldown_remaining_secs;last_attempt_action;last_attempt_target_mbps;last_attempt_outcome;last_attempt_unix_ms;last_attempt_error;rtt_source
```

Unavailable values are empty fields. The file is appended across daemon restarts, with a header written only for a new or empty file. At 64 MiB, StormGuard rotates the file to `<log_file>.1`; the previous `.1` is replaced, so at most one backup is retained.

Application outcomes distinguish `applied`, `dry_run`, `skipped`, and `failed`. A failed adjustment does not change StormGuard's current limit or start its cooldown, so it remains eligible for retry.

Use this during rollout validation.

## Safe Rollout Pattern

1. Enable StormGuard with `dry_run = true`.
2. Observe behavior for multiple peak periods.
3. Validate there are no undesirable limit oscillations.
4. Switch `dry_run = false`.
5. Continue monitoring after each major topology/integration change.

## Troubleshooting

If StormGuard behavior seems incorrect:

```bash
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqosd --since "30 minutes ago"
```

Also verify:

- target node names still match your current `network.json` hierarchy
- integration updates have not renamed key nodes/APs
- your minimum percentage floors are reasonable for expected traffic profiles
- `network.json` still reflects your planned/source-of-truth site rates if you are investigating an unexpected StormGuard reduction
- `log_file` path (if configured) is writable by the service user

## Related Pages

- [Configuration](configuration.md)
- [Scale Planning and Topology Design](scale-topology.md)
- [Performance Tuning](performance-tuning.md)
- [High Availability and Failure Domains](high-availability.md)
- [Components](components.md)
- [Troubleshooting](troubleshooting.md)
