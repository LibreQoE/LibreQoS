# Quickstart: ISP Deployment Path

Use this page to move from install to a safe pilot with minimal ambiguity.

Need definitions for key terms? See the [Glossary](glossary.md).

## 1) Common Install Foundation

Complete this once:

1. Review architecture and sizing:
- [Deployment Scenarios](design.md)
- [System Requirements](requirements.md)

2. Prepare host and OS:
- [Server Setup - Prerequisites](prereq.md)
- [Install Ubuntu Server 24.04](ubuntu-server.md)

3. Configure bridge mode:
- [Configure Shaping Bridge](bridge.md)

4. Install LibreQoS (`.deb` recommended):

```bash
cd /tmp
sudo apt-get update
sudo apt-get upgrade
wget https://download.libreqos.com/{deb_url_v1_5}
sudo apt install ./{deb_url_v1_5}
```

Using `/tmp` avoids local `.deb` permission issues where `apt` cannot access a package stored in a private home directory as user `_apt`.

### Ubuntu 24.04 hotfix if the `.deb` install stops

On affected Ubuntu 24.04 hosts using `systemd-networkd`, the `.deb` install may stop and print a hotfix requirement message. This is expected.

If that happens, run:

```bash
sudo /opt/libreqos/src/systemd_hotfix.sh install
sudo reboot
```

After the reboot, resume the install:

```bash
cd /tmp
wget https://download.libreqos.com/{deb_url_v1_5}
sudo apt install ./{deb_url_v1_5}
```

5. Open WebUI at `http://your_shaper_ip:9123`.

## 2) 10-Minute Health Check

Run:

```bash
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqosd -u lqos_scheduler --since "10 minutes ago"
```

Confirm:
- WebUI Dashboard loads.
- Scheduler Status is healthy.
- No urgent/fatal startup errors in logs.

If this fails, go to [Troubleshooting](troubleshooting.md) before proceeding.

## 3) Decision A: Deployment Stage

Choose one:

- **Lab first**: validate behavior in a controlled environment before inline traffic.
- **Inline pilot now**: proceed directly with limited production traffic.

If lab-first:
1. Build a lab topology.
2. Generate test traffic.
3. Validate Dashboard, Tree, Flow, Scheduler Status, and Urgent Issues.
4. Continue to Decision B.

## 4) Decision B: Source of Truth (Pick One Owner)

| If this describes you | Mode | Owner of durable shaping data |
|---|---|---|
| You use a supported CRM/NMS integration | Built-in integration mode | Integration jobs |
| You generate `network.json` and `ShapedDevices.csv` with your own scripts | Custom source of truth mode | Your scripts |
| You maintain files manually for a small/simple network | Manual files mode | Manual edits |

Rule: keep one owner for persistent shaping inputs.

## 5) Path Cards

### Built-In Integration Mode

When to choose:
- Your CRM/NMS is supported by built-in integrations.

Do this now:
1. Configure integration settings in WebUI.
2. Run initial sync and validate imported shaping/topology data.
3. Place LibreQoS inline for pilot traffic.
4. Validate Scheduler Status, Urgent Issues, and topology/flow views.
5. Expand pilot scope after stable operation.

Next:
- [CRM/NMS Integrations](integrations.md)
- [Troubleshooting](troubleshooting.md)

### Custom Source of Truth (Your Scripts)

When to choose:
- Your CRM/NMS is unsupported and you generate `network.json` + `ShapedDevices.csv` with your own pipeline.

Do this now:
1. Implement script/process to generate and refresh shaping files.
2. Declare script outputs as your source of truth.
3. Place LibreQoS inline for pilot traffic.
4. Use WebUI for operational checks and short-term adjustments.
5. Keep permanent changes in your external script workflow.

Format reference:
- See `network.json` and `ShapedDevices.csv` sections in [Advanced Configuration Reference](configuration-advanced.md).

Next:
- [Operating Modes and Source of Truth](operating-modes.md)
- [Troubleshooting](troubleshooting.md)

### Manual Files Mode (<100 Subscribers)

When to choose:
- You intentionally maintain `network.json` + `ShapedDevices.csv` without CRM/NMS synchronization.

Do this now:
1. Build and maintain shaping files directly.
2. Place LibreQoS inline for pilot traffic.
3. Validate shaping and scheduler status in WebUI.
4. Maintain strict manual change discipline.
5. Plan migration to supported integration or scripts if scale/change volume grows.

Next:
- [Advanced Configuration Reference](configuration-advanced.md)
- [Troubleshooting](troubleshooting.md)

## 6) Common First-Run Mistakes

- Unclear source of truth ownership between integration and manual edits.
- Changing topology depth before passing the health gate.
- Skipping post-change service/log validation before pilot traffic.

## 7) Related Pages

- [Operating Modes and Source of Truth](operating-modes.md)
- [CRM/NMS Integrations](integrations.md)
- [Advanced Configuration Reference](configuration-advanced.md)
- [Troubleshooting](troubleshooting.md)
