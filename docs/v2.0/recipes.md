# Deployment Recipes

This page collects repeatable, scenario-based deployment recipes.

Use recipes after you have completed base install and health validation in [Quickstart](quickstart.md).

## Scope

- Recipes here are for supported deployment patterns.
- Each recipe focuses on practical implementation, validation, and rollback.
- Router examples use MikroTik RouterOS v7 where routing policy examples are useful.

## Recipe Index

1. [WISP/FISP with built-in CRM/NMS integration](recipes-wisp-fisp-integration.md)
2. [Maritime variable WAN with StormGuard](recipes-maritime-stormguard.md)
3. [Event WiFi with subnet-group shaping](recipes-event-wifi.md)
4. [Hotel and hospitality per-device shaping](recipes-hospitality.md)
5. [Education / university per-IP shaping for real-time calls](recipes-education.md)
6. [Switch-centric fabric with maintenance bypass (plus SD-WAN variant)](recipes-switch-fabric-sdwan.md)
7. [Proxmox VM deployment with 3 NICs](recipes-proxmox-vm.md)

```{toctree}
:hidden:

recipes-wisp-fisp-integration
recipes-maritime-stormguard
recipes-event-wifi
recipes-hospitality
recipes-education
recipes-switch-fabric-sdwan
recipes-proxmox-vm
```

## Related Pages

- [Quickstart](quickstart.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [CRM/NMS Integrations](integrations.md)
- [Scale Planning and Topology Design](scale-topology.md)
- [StormGuard](stormguard.md)
- [High Availability and Failure Domains](high-availability.md)
- [Troubleshooting](troubleshooting.md)
