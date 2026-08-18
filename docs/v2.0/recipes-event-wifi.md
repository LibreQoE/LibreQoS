# Recipe: Event WiFi with Subnet-Group Shaping

Use this pattern for short-lived, high-density event networks where grouping client devices by subnet is operationally simpler than per-device lifecycle tracking.

## Fit

- Best for: temporary events, rapidly changing attendees/devices, low-friction operations.
- Avoid when: you need strict long-term subscriber-level lifecycle shaping.

## Pattern

- Define one circuit per subnet group (for example per `/24`).
- Keep topology intentionally simple (often `flat` or shallow parent grouping).
- Use explicit `Parent Node` names in `ShapedDevices.csv` for grouping clarity.

Example row (one subnet as one shaped circuit):

```text
Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment
EVT-24-101,Event Hall A Subnet,EVT-24-101,HallA-Clients,Event_Core,,100.64.101.0/24,,10,10,300,300,Temporary event subnet group
```

If integration mode owns your shaping files, direct edits like this may be overwritten on the next integration sync.

## Implementation

1. Confirm one owner of shaping data (manual files, custom source of truth, or integration mode).
2. Pre-create subnet-based circuits for expected attendee segments.
3. Keep parent hierarchy shallow to reduce queue churn during rapid changes.
4. Validate in WebUI before opening registration traffic.

## Validation Checklist

1. Active traffic appears under expected subnet-group circuits.
2. Scheduler remains healthy under rapid client churn.
3. Queue/class pressure remains stable (watch urgent issues, including overflow warnings).
4. Flow/Tree views remain coherent during peak arrivals.

## Rollback

1. Restore pre-event `ShapedDevices.csv` / `network.json`.
2. Restart scheduler.
3. Validate baseline traffic behavior.

## Related Pages

- [Advanced Configuration Reference](configuration-advanced.md)
- [Scale Planning and Topology Design](scale-topology.md)
- [Troubleshooting](troubleshooting.md)
