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
