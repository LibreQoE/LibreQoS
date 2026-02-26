# Configure LibreQoS

## Page Purpose

Use this page for daily operations and WebUI configuration. Use [Advanced Configuration Reference](configuration-advanced.md) for direct file editing and CLI-heavy workflows.

## Initial Configuration Via Setup Tool (From the .deb installer)
<img width="1605" height="1030" alt="setup_tool" src="https://github.com/user-attachments/assets/5a645da8-c411-4635-9777-a881966981df" />

The setup tool configures initial bridge, interface, bandwidth, IP range, and WebUI user settings.

Notes:
- The setup tool is keyboard-driven (`Enter` to select, `Q` to quit without saving).
- If you need to relaunch it after closing:
  ```
  sudo apt remove libreqos
  sudo apt install ./{deb_url_v1_5}
  ```

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

## Advanced Configuration Reference

CLI-driven configuration, direct file editing, and deep reference content were moved here:
- [Advanced Configuration Reference](configuration-advanced.md)

## Configuration via Command Line

This section moved to [Advanced Configuration Reference](configuration-advanced.md#configuration-via-command-line).

## Network Hierarchy

This section moved to [Advanced Configuration Reference](configuration-advanced.md#network-hierarchy).

## Circuits

This section moved to [Advanced Configuration Reference](configuration-advanced.md#circuits).

## Related Pages

- [Quickstart](quickstart.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [CRM/NMS Integrations](integrations.md)
- [Advanced Configuration Reference](configuration-advanced.md)
- [Troubleshooting](troubleshooting.md)
