# Configure LibreQoS

## Page Purpose

Use this page for daily operations and WebUI configuration. Use [Advanced Configuration Reference](configuration-advanced.md) for direct file editing and CLI-heavy workflows.

## Initial Configuration Via Setup Tool (From the .deb installer)
<img width="1605" height="1030" alt="setup_tool" src="https://github.com/user-attachments/assets/5a645da8-c411-4635-9777-a881966981df" />

The setup tool configures initial bridge, interface, bandwidth, IP range, and WebUI user settings.

Notes:
- The setup tool is keyboard-driven (`Enter` to select, `Q` to quit without saving).

### Next Steps

After install, sign in to WebUI at `http://your_shaper_ip:9123`.

If no WebUI users exist yet, current builds redirect to first-run setup automatically.

For most operators:
1. Choose your operating mode: [Operating Modes and Source of Truth](operating-modes.md)
2. Configure integration settings in WebUI: [CRM/NMS Integrations](integrations.md)
3. Validate scheduler and shaping behavior in WebUI: [LibreQoS WebUI (Node Manager)](node-manager-ui.md)

## Configuration Via Web Interface

Most day-to-day LibreQoS configuration is handled in WebUI (`http://your_shaper_ip:9123/config_general.html`).

Current builds use a consistent configuration layout across the General, RTT, Queues, TreeGuard, Network Mode, Integration Defaults, Network Layout, Insight, provider integration, IP Ranges, Flow Tracking, and Shaped Devices pages. Integration Defaults also includes the shared Ethernet port headroom policy used by integrations that can detect negotiated subscriber-facing port speeds.

### Where in WebUI

- General settings: `Configuration -> General`
- Integration settings: `Configuration -> Integrations`
- Network layout editor: `Configuration -> Network Layout`
- Shaped devices editor: `Configuration -> Shaped Devices`
- Runtime operational validation: `WebUI (Node Manager)` pages (dashboard/tree/flow/scheduler)

When an integration is managing your shaping data, the `Network Layout` and `Shaped Devices` editors remain visible but become read-only in WebUI.

Topology note:
- `network.json` node names must be globally unique across the whole tree. Duplicate node names now fail validation and are not accepted by the WebUI save path or `LibreQoS.py`.
- When a node exposes a stable `id`, LibreQoS prefers that ID for saved site bandwidth overrides while keeping legacy name-only matching as a fallback.

Queue-mode note:
- Current builds use `queue_mode` with `shape` and `observe` values. Older `monitor_only` wording is a compatibility alias rather than the primary operator-facing setting.

## Operating Modes and Source of Truth

Read this first before production changes:
- [Operating Modes and Source of Truth](operating-modes.md)

## QoO (Quality of Outcome) profiles (`qoo_profiles.json`)

LibreQoS displays QoO as an estimate of internet quality based on latency and loss.

### Where the file lives

`<lqos_directory>/qoo_profiles.json`

### Selecting a profile

- **WebUI**: Configuration -> General -> QoO Profile
- **Config file**: set `qoo_profile_id` in `/etc/lqos.conf`

Example:

```toml
# /etc/lqos.conf
qoo_profile_id = "web_browsing"
```

### Applying changes

- Changes to `qoo_profiles.json` are picked up automatically.
- If you change `/etc/lqos.conf`, restart `lqosd`.

## Need CLI or File-Level Changes?

For direct file editing (`/etc/lqos.conf`, `network.json`, `ShapedDevices.csv`), overrides, and deeper topology/circuit reference material, use:

- [Advanced Configuration Reference](configuration-advanced.md)

## Related Pages

- [Quickstart](quickstart.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [CRM/NMS Integrations](integrations.md)
- [Advanced Configuration Reference](configuration-advanced.md)
- [Troubleshooting](troubleshooting.md)
