# Sonar Integration

## Summary

Use this when Sonar is your CRM/NMS source of truth.

## Basic Setup

1. Configure Sonar integration settings in `/etc/lqos.conf`.
2. Run a manual import test.
3. Enable scheduler-driven sync.

## Operational Notes

- `ShapedDevices.csv` is regenerated on sync.
- `network.json` behavior depends on overwrite settings.

## Full Reference

- [Detailed Sonar Reference](integrations-reference.md#sonar-integration)
- [Operating Modes and Source of Truth](operating-modes.md)
