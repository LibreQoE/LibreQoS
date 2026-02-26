# Splynx Integration

## Summary

Use this when Splynx is your CRM/NMS source of truth.

## Basic Setup

1. Configure Splynx settings in `/etc/lqos.conf`.
2. Select topology strategy (`flat`, `ap_only`, `ap_site`, `full`).
3. Enable automatic sync and restart scheduler.

## Operational Notes

- `ShapedDevices.csv` is regenerated each sync.
- `network.json` overwrite depends on `always_overwrite_network_json`.
- Use WebUI Integration settings for day-to-day adjustments.

## Full Reference

- [Detailed Splynx Reference](integrations-reference.md#splynx-integration)
- [Operating Modes and Source of Truth](operating-modes.md)
