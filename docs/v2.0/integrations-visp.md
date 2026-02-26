# VISP Integration

## Summary

Use this when VISP is your CRM/NMS source of truth.

## Basic Setup

1. Configure `[visp_integration]` credentials in `/etc/lqos.conf`.
2. Run a manual import test.
3. Enable scheduler-driven sync.

## Operational Notes

- `ShapedDevices.csv` is rewritten each run.
- `network.json` is overwritten only when enabled in integration common settings.

## Full Reference

- [Detailed VISP Reference](integrations-reference.md#visp-integration)
- [Operating Modes and Source of Truth](operating-modes.md)
