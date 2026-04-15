# Configure LibreQoS

## Page Purpose

Use this page for daily operations and WebUI-based configuration.

Use [Quickstart](quickstart.md) for the install and day-1 deployment path.
Use [Advanced Configuration Reference](configuration-advanced.md) for direct file editing and CLI-heavy workflows.

## Initial Configuration In WebUI

Current installs use the main LibreQoS WebUI on port `9123` for onboarding.

After installing the package:
1. Open `http://your_shaper_ip:9123`
2. Create the first admin user if LibreQoS redirects you to `first-run.html`
3. Sign in
4. Open `Complete Setup`
5. Choose how LibreQoS will receive subscriber and topology data

For most operators, `Complete Setup` is where the important early decision happens:
- Built-in integration for UISP, Splynx, VISP, Netzur, Powercode, Sonar, or WispGate
- Custom importer if your own process writes `network.json` and `ShapedDevices.csv`
- Manual files if you intentionally maintain those files yourself

If Scheduler Status still says `Setup Required`, LibreQoS is not ready to shape subscribers yet. Finish `Complete Setup` and confirm your chosen data source has published valid data before treating the system as production-ready.

## Configuration Via Web Interface

Most day-to-day LibreQoS configuration happens in WebUI (`http://your_shaper_ip:9123/config_general.html`).

Current builds use a consistent configuration layout across the General, RTT, Queues, TreeGuard, Network Mode, Integration Defaults, Network Layout, Insight, provider integration, IP Ranges, Flow Tracking, and Shaped Devices pages. Integration Defaults also includes the shared Ethernet port headroom policy used by integrations that can detect negotiated subscriber-facing port speeds.

### Where In WebUI

- General settings: `Configuration -> General`
- Integration settings: `Configuration -> Integrations`
- Network layout editor: `Configuration -> Network Layout`
- Shaped devices editor: `Configuration -> Shaped Devices`
- Runtime operational validation: `WebUI (Node Manager)` pages such as dashboard, tree, flow, and scheduler

When an integration is managing your shaping data, the `Network Layout` and `Shaped Devices` editors remain visible but become read-only in WebUI.

## Source Of Truth

Read this first before production changes:
- [Operating Modes and Source of Truth](operating-modes.md)

Persistent shaping changes should be made in one place only.

If an integration owns your topology and subscriber data, keep permanent changes in that integration workflow.
If your own importer owns the files, keep permanent changes in that importer.
If you intentionally run manual files, keep permanent changes in `network.json` and `ShapedDevices.csv`.

## Important Notes

Topology note:
- `network.json` node names must be globally unique across the whole tree. Duplicate node names fail validation and are not accepted by the WebUI save path or `LibreQoS.py`.
- When a node exposes a stable `id`, LibreQoS prefers that ID for saved site bandwidth overrides while keeping legacy name-only matching as a fallback.

Queue-mode note:
- Current builds use `queue_mode` with `shape` and `observe` values. Older `monitor_only` wording is a compatibility alias rather than the primary operator-facing setting.

Cobrand logo note:
- `Configuration -> General` includes an optional cobrand image toggle and PNG upload.
- LibreQoS saves the uploaded file as `cobrand.png` in the runtime static assets directory.
- The top-level `display_cobrand` setting in `/etc/lqos.conf` is optional. If it is omitted, LibreQoS treats it as `false`.
- The sidebar renders the cobrand image at 48px tall to match the LibreQoS logo, with a maximum sidebar width of 176px.

## QoO (Quality of Outcome) Profiles (`qoo_profiles.json`)

LibreQoS displays QoO as an estimate of internet quality based on latency and loss.

### Where The File Lives

`<lqos_directory>/qoo_profiles.json`

### Selecting A Profile

- WebUI: `Configuration -> General -> QoO Profile`
- Config file: set `qoo_profile_id` in `/etc/lqos.conf`

Example:

```toml
# /etc/lqos.conf
qoo_profile_id = "web_browsing"
```

### Applying Changes

- Changes to `qoo_profiles.json` are picked up automatically.
- If you change `/etc/lqos.conf`, restart `lqosd`.

## Need CLI Or File-Level Changes?

For direct file editing (`/etc/lqos.conf`, `network.json`, `ShapedDevices.csv`), overrides, and deeper topology or circuit reference material, use:

- [Advanced Configuration Reference](configuration-advanced.md)

## Related Pages

- [Quickstart](quickstart.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [CRM/NMS Integrations](integrations.md)
- [LibreQoS WebUI (Node Manager)](node-manager-ui.md)
- [Advanced Configuration Reference](configuration-advanced.md)
- [Troubleshooting](troubleshooting.md)
