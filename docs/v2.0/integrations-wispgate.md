# WISPGate Integration

## Summary

Use this when WISPGate is your CRM/NMS source of truth.

## Basic Setup

1. Configure WISPGate integration settings in `/etc/lqos.conf`.
2. Run a manual import test.
3. Enable scheduler-driven sync.

## Operational Notes

- `ShapedDevices.csv` is regenerated on sync.
- `network.json` behavior depends on your overwrite settings.

## Full Reference

- [Detailed WISPGate Reference](integrations-reference.md#wispgate-integration)
- [Operating Modes and Source of Truth](operating-modes.md)
