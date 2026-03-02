# Recipe: Switch-Centric Fabric with Maintenance Bypass (SD-WAN Variant Included)

Use this pattern when the network core is switch-centric, VLAN-driven, and you want shaped primary paths plus backup bypass paths for no-downtime LibreQoS maintenance.

## Fit

- Best for: fabrics where explicit L2/L3 path engineering is already in place.
- Avoid when: path ownership and parent mapping are unclear across systems.

## Topology Pattern

Primary (shaped) VLAN paths traversing LibreQoS:

1. `EdgeA <-> CoreA`
2. `EdgeA <-> CoreB`
3. `EdgeB <-> CoreA`
4. `EdgeB <-> CoreB`

Backup (bypass) VLAN paths should also exist, with dynamic routing policy preferring shaped paths during normal operation.

## Example Topology Illustrations

### Shaped VLAN Set

```{mermaid}
flowchart LR
    EA[EdgeA Router]
    EB[EdgeB Router]
    SW[SW1/SW2 Switch Pair]
    LQ[LibreQoS Bridge]
    CA[CoreA Router]
    CB[CoreB Router]

    EA --> SW
    EB --> SW
    SW -->|Shaped VLANs 110,120,210,220| LQ
    LQ --> SW
    SW --> CA
    SW --> CB
```

Shaped VLAN mapping:

- VLAN 110: `EdgeA <-> CoreA` (through LibreQoS)
- VLAN 120: `EdgeA <-> CoreB` (through LibreQoS)
- VLAN 210: `EdgeB <-> CoreA` (through LibreQoS)
- VLAN 220: `EdgeB <-> CoreB` (through LibreQoS)

### Bypass VLAN Set

```{mermaid}
flowchart LR
    EA[EdgeA Router]
    EB[EdgeB Router]
    SW[SW1/SW2 Switch Pair]
    CA[CoreA Router]
    CB[CoreB Router]

    EA -.-> SW
    EB -.-> SW
    SW -. Bypass VLANs 310,320,410,420 .-> CA
    SW -. Bypass VLANs 310,320,410,420 .-> CB
```

Bypass VLAN mapping:

- VLAN 310: `EdgeA <-> CoreA` (bypass)
- VLAN 320: `EdgeA <-> CoreB` (bypass)
- VLAN 410: `EdgeB <-> CoreA` (bypass)
- VLAN 420: `EdgeB <-> CoreB` (bypass)

What this shows:

- Shaped VLANs traverse the switch pair and the LibreQoS bridge path.
- Bypass VLANs traverse the switch pair without traversing LibreQoS.
- `SW1/SW2` are shown as a single logical switch-pair node for readability.
- This is a logical VLAN-path illustration, not a full physical cabling diagram.

## MikroTik RouterOS v7 Example (Conceptual OSPF Preference)

Set lower OSPF cost on shaped interfaces, higher cost on bypass interfaces.

```text
/routing ospf instance
add name=default-v2 router-id=10.255.255.1

/routing ospf area
add name=backbone-v2 area-id=0.0.0.0 instance=default-v2

/routing ospf interface-template
add interfaces=vlan-edgea-corea-lq area=backbone-v2 cost=10
add interfaces=vlan-edgea-coreb-lq area=backbone-v2 cost=10
add interfaces=vlan-edgeb-corea-lq area=backbone-v2 cost=10
add interfaces=vlan-edgeb-coreb-lq area=backbone-v2 cost=10
add interfaces=vlan-edgea-corea-bypass area=backbone-v2 cost=200
add interfaces=vlan-edgea-coreb-bypass area=backbone-v2 cost=200
add interfaces=vlan-edgeb-corea-bypass area=backbone-v2 cost=200
add interfaces=vlan-edgeb-coreb-bypass area=backbone-v2 cost=200
```

Adjust interface names/costs to match your design policy.

## Implementation

1. Build and verify all primary and bypass VLAN interfaces.
2. Keep path preference deterministic (OSPF/BGP policy).
3. Place LibreQoS inline on the primary VLAN paths.
4. Validate failover and failback behavior during a maintenance window.

## Validation Checklist

1. Normal state: traffic follows shaped primary path.
2. Failure or maintenance state: routing converges to bypass path.
3. Recovery state: traffic returns to shaped path without instability.
4. WebUI health remains stable across transitions.

## SD-WAN Variant

For SD-WAN, use the same primary/bypass control model:

- Primary underlay paths pass through LibreQoS shaper path.
- Secondary underlay paths bypass as maintenance/failure fallback.
- Keep node naming and parent relationships stable so hierarchy does not churn after path events.

## Rollback

1. Temporarily force routing preference to bypass paths.
2. Restore previous LibreQoS path policy.
3. Re-enable shaped path preference after verification.

## Related Pages

- [High Availability and Failure Domains](high-availability.md)
- [Scale Planning and Topology Design](scale-topology.md)
- [Configure Shaping Bridge](bridge.md)
- [Troubleshooting](troubleshooting.md)
