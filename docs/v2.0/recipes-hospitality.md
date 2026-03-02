# Recipe: Hotel and Hospitality Per-Device Shaping

Use this pattern when each client device should receive its own circuit behavior (for example room-device fairness and isolation goals).

## Fit

- Best for: hospitality environments with predictable address pools and per-device fairness targets.
- Avoid when: the projected per-device circuit count exceeds practical RAM/queue limits for your hardware.

## Capacity Guardrails

Per-device shaping can raise circuit count quickly. Validate:

1. RAM sizing against expected device volume ([System Requirements](requirements.md)).
2. Queue/class pressure and urgent issue health in production-like load.
3. CAKE qdisc scale impact (memory and operational overhead).
4. If peak tests show persistent queue/class pressure or unsafe memory growth, pivot to per-room or per-subnet grouping.

## Pattern

- Build an enumerated list of IPv4 addresses corresponding to possible client devices.
- Assign one shaped circuit per device IP.
- Use stable parent groups (floor/building/wing) to preserve operational visibility.

## Example Entry

```text
Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment,sqm
HTL-ROOM-1204,Room1204-Device,HTL-ROOM-1204,Room1204-DeviceA,Floor12,,100.70.12.44,,2,2,50,20,Hospitality per-device plan,cake
```

## Validation Checklist

1. Device-to-circuit mapping is stable and correct.
2. No persistent urgent issue signals for queue/class limits.
3. Memory remains within expected envelope at peak occupancy.
4. RTT/retransmit quality remains acceptable under busy-hour contention.

## Rollback

1. Move from per-device to per-room or per-subnet grouping.
2. Reduce circuit count and reload.
3. Re-check scheduler health and queue pressure.

## Related Pages

- [System Requirements](requirements.md)
- [Scale Planning and Topology Design](scale-topology.md)
- [CAKE](cake.md)
- [Troubleshooting](troubleshooting.md)
