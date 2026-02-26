# Powercode Integration

## Summary

Use this when Powercode is your CRM/NMS source of truth.

## Basic Setup

1. Configure Powercode integration settings in `/etc/lqos.conf`.
2. Run a manual import test.
3. Enable scheduler-driven sync.

## Operational Notes

- `ShapedDevices.csv` is regenerated on sync.
- Review how you want `network.json` handled for your topology workflow.

## Full Reference

- [Detailed Powercode Reference](integrations-reference.md#powercode-integration)
- [Operating Modes and Source of Truth](operating-modes.md)
