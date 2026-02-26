# UISP Integration

## Summary

Use this when UISP is your CRM/NMS source of truth.

## Basic Setup

1. Configure UISP settings in `/etc/lqos.conf`.
2. Choose topology strategy and suspension handling strategy.
3. Enable automatic sync and restart scheduler.

## Operational Notes

- `ShapedDevices.csv` is regenerated each sync.
- `network.json` overwrite depends on `always_overwrite_network_json`.
- Use WebUI for operational changes; treat file edits as temporary in integration mode.

## Full Reference

- [Detailed UISP Reference](integrations-reference.md#uisp-integration)
- [Operating Modes and Source of Truth](operating-modes.md)
