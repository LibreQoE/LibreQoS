# Recipe: Maritime Variable WAN with StormGuard

Use this pattern when WAN capacity changes materially over time (for example satellite backhaul under weather/load effects) and you need bounded adaptive queue tuning.

## Fit

- Best for: one vessel or one high-variance WAN domain represented as one top-level node.
- Avoid when: you plan to manage dozens/hundreds of targets with StormGuard.

## Prerequisites

1. Complete [Quickstart](quickstart.md).
2. Review [StormGuard](stormguard.md) scope and limits.
3. Confirm source-of-truth behavior ([Operating Modes](operating-modes.md)).

## Topology Pattern

Use a single top-level node named `Ship`, with all subnodes beneath it.

Example `network.json` skeleton:

```json
{
  "Ship": {
    "downloadBandwidthMbps": 1000,
    "uploadBandwidthMbps": 200,
    "children": {
      "Deck_A": {
        "downloadBandwidthMbps": 500,
        "uploadBandwidthMbps": 100
      },
      "Deck_B": {
        "downloadBandwidthMbps": 500,
        "uploadBandwidthMbps": 100
      }
    }
  }
}
```

## StormGuard Configuration

```toml
[stormguard]
enabled = true
dry_run = true
targets = ["Ship"]
minimum_download_percentage = 0.5
minimum_upload_percentage = 0.5
log_file = "/var/log/stormguard.csv"
```

## Control Loop Illustration

```{mermaid}
flowchart LR
    METRICS[Ship Node Metrics\nthroughput, RTT, loss context]
    SG[StormGuard Evaluator]
    LIMITS[Bounded Queue Limit Adjustments]
    QOE[Observed Link Quality]

    METRICS --> SG
    SG --> LIMITS
    LIMITS --> QOE
    QOE --> METRICS
```

What this shows:

- StormGuard continuously evaluates current `Ship` conditions and applies bounded adjustments.
- Observed quality and saturation behavior feed the next evaluation cycle.

Rollout sequence:

1. Start with `dry_run = true`.
2. Observe multiple busy periods.
3. Confirm adjustments are bounded and sensible.
4. Set `dry_run = false`.

## Validation Checklist

1. StormGuard debug/status pages show `Ship` as active target.
2. Effective limits adjust during congestion but respect floor percentages.
3. RTT/retransmit behavior improves under stress periods.
4. No naming drift between target names and current hierarchy.

## Rollback

1. Set `[stormguard] enabled = false` (or back to `dry_run = true`).
2. Restart services:

```bash
sudo systemctl restart lqosd lqos_scheduler
```

3. Verify stable behavior without adaptive changes.

## Related Pages

- [StormGuard](stormguard.md)
- [Scale Planning and Topology Design](scale-topology.md)
- [Troubleshooting](troubleshooting.md)
