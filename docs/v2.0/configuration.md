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

For most operators:
1. Choose your operating mode: [Operating Modes and Source of Truth](operating-modes.md)
2. Configure integration settings in WebUI: [CRM/NMS Integrations](integrations.md)
3. Validate scheduler and shaping behavior in WebUI: [LibreQoS WebUI (Node Manager)](node-manager-ui.md)

## Configuration Via Web Interface

Most day-to-day LibreQoS configuration is handled in WebUI (`http://your_shaper_ip:9123/config_general.html`).

### Where in WebUI

- General settings: `Configuration -> General`
- Integration settings: `Configuration -> Integrations`
- Shaped devices editor: `Configuration -> Shaped Devices`
- Runtime operational validation: `WebUI (Node Manager)` pages (dashboard/tree/flow/scheduler)

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

### Sandwich Mode Settings

Sandwich mode is an optional compatibility and rate‑limiting layer for Bridge Mode. Enable and configure it in the Web UI under Configuration → Network Mode → Bridge Mode → “Sandwich Bridge (veth pair)”.

When appropriate
- Compatibility with unsupported NICs or special environments (acceptable performance trade‑off for testing).
- Compatibility when using bonded NICs/LACP.
- Enforcing a hard/accurate rate limit in one or both directions (metered bandwidth).

Key options (under `[bridge]`)
- `to_internet` and `to_network`: existing physical shaping interfaces (unchanged).
- `sandwich.Full.with_rate_limiter`: one of `"None"`, `"Download"`, `"Upload"`, or `"Both"`.
- `sandwich.Full.rate_override_mbps_down`: optional integer; overrides the Download limit if set.
- `sandwich.Full.rate_override_mbps_up`: optional integer; overrides the Upload limit if set.
- `sandwich.Full.queue_override`: optional integer; sets veth TX queue count (default is number of CPU cores).
- `sandwich.Full.use_fq_codel`: optional boolean; attach `fq_codel` under the HTB class for better queueing.

Example (TOML)
```
[bridge]
to_internet = "enp1s0f1"
to_network  = "enp1s0f2"

  [bridge.sandwich.Full]
  with_rate_limiter        = "Both"
  rate_override_mbps_down  = 500
  rate_override_mbps_up    = 100
  queue_override           = 8
  use_fq_codel             = true
```

Rate limiting details
- Sandwich rate limiting uses an HTB class for the cap; `fq_codel` (if enabled) is attached as a child qdisc to improve queueing behavior.
- Choose the limiter direction based on the need: Download (ISP→LAN), Upload (LAN→ISP), or Both.

## Related Pages

- [Quickstart](quickstart.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [CRM/NMS Integrations](integrations.md)
- [Advanced Configuration Reference](configuration-advanced.md)
- [Troubleshooting](troubleshooting.md)
