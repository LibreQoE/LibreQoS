# Netzur Integration

## Summary

Use this when Netzur is your CRM/NMS source of truth.

## Basic Setup

1. Configure `[netzur_integration]` values in `/etc/lqos.conf`.
2. Run a manual import test.
3. Enable scheduler-driven sync.

## Operational Notes

- Integration regenerates `ShapedDevices.csv`.
- `network.json` updates depend on overwrite settings.

## Full Reference

- [Detailed Netzur Reference](integrations-reference.md#netzur-integration)
- [Operating Modes and Source of Truth](operating-modes.md)
