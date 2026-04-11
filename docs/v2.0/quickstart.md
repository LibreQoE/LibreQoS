# Quickstart: ISP Deployment Path

Use this page to move from install to a safe pilot.

Most ISPs, should use a LibreQoS built-in integration.

Use this page in order:
1. Complete install and bridge setup.
2. Complete first-run WebUI access.
3. Pass the 10-minute health check.
4. Choose one system to manage your shaping data.
5. Follow the recommended path for that system.

Need definitions for key terms? See the [Glossary](glossary.md).

## 1) Common Install Foundation

Complete this once before trying to shape live traffic:

1. Review architecture and sizing:
- [Deployment Scenarios](design.md)
- [System Requirements](requirements.md)

2. Prepare host and OS:
- [Server Setup - Prerequisites](prereq.md)
- [Install Ubuntu Server 24.04](ubuntu-server.md)

3. Configure how traffic will pass through the LibreQoS box:
- [Configure Shaping Bridge](bridge.md)

This step is required before production use. If bridge/interface setup is wrong, later WebUI and shaping checks will be misleading.

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

The hotfix installer bootstraps the LibreQoS APT repo at `https://repo.libreqos.com`, installs the patched Noble `systemd` package set, and pins those packages for future updates.

After the reboot, resume the install:

```bash
cd /tmp
wget https://download.libreqos.com/{deb_url_v1_5}
sudo apt install ./{deb_url_v1_5}
```

5. Open WebUI at `http://your_shaper_ip:9123`.

If no WebUI users exist yet, current builds redirect you to the first-run setup page automatically instead of presenting a normal login flow.

6. Complete first-run access:
- Create the initial WebUI user if prompted.
- Confirm you can sign in.
- Confirm the Dashboard opens before doing any integration or topology work.

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
- You can sign in successfully after first-run setup.

If this fails, go to [Troubleshooting](troubleshooting.md) before proceeding.

## 3) Choose One System To Manage Your Shaping Data

This is the most important early decision.

On this page, "source of truth" just means the place where permanent shaping changes should be made.

If you make persistent changes somewhere else, they may be overwritten later.

Choose one:

| If this describes you | Mode | Where permanent shaping changes belong |
|---|---|---|
| You use a supported LibreQoS CRM/NMS integration | Built-In Integration Mode | Integration jobs |
| You generate `network.json` and `ShapedDevices.csv` with your own automation | Custom Source of Truth Mode | Your scripts |
| You intentionally maintain files by hand | Manual Files Mode | Manual edits |

Rule: pick one place for permanent shaping changes.

Most operators should stop here and choose **Built-In Integration Mode**.

Only choose the other two modes if you already know why.

## 4) Recommended Path: Built-In Integration Mode

This is the default path for most ISPs, including most small ISPs.

When to choose:
- Your CRM/NMS is supported by built-in integrations.

Do this now:
1. Configure integration settings in WebUI.
2. Run the initial sync.
3. Confirm Scheduler Status is healthy.
4. Confirm Network Tree reflects the expected hierarchy depth for your chosen integration strategy.
5. Confirm there are no urgent or fatal issues that block shaping.
6. Only then move to limited pilot traffic or inline use.

Do not do this:
- Do not hand-edit files that your integration refreshes as part of your normal workflow.
- Do not mix manual file edits with scheduled integration syncs unless you intentionally want that extra complexity.

Next:
- [CRM/NMS Integrations](integrations.md)
- [Troubleshooting](troubleshooting.md)

## 5) Other Paths

### Custom Source of Truth Mode

When to choose:
- Your CRM/NMS is unsupported and you already generate `network.json` + `ShapedDevices.csv` with your own pipeline.

Do this now:
1. Implement script/process to generate and refresh shaping files.
2. Treat your script outputs as the place for permanent shaping changes.
3. Place LibreQoS inline for pilot traffic.
4. Use WebUI for operational checks and short-term adjustments.
5. Keep permanent changes in your external script workflow.

Format reference:
- See `network.json` and `ShapedDevices.csv` sections in [Advanced Configuration Reference](configuration-advanced.md).

Next:
- [Operating Modes and Source of Truth](operating-modes.md)
- [Troubleshooting](troubleshooting.md)

### Manual Files Mode

When to choose:
- You intentionally maintain `network.json` + `ShapedDevices.csv` without CRM/NMS synchronization.
- Your network is small/simple enough that manual discipline is realistic.
- You are doing a temporary pilot or working around an unsupported system.

Do this now:
1. Build `network.json`.
2. Build `ShapedDevices.csv`.
3. Restart or validate services.
4. Confirm the expected topology and devices appear in WebUI.
5. Validate shaping and scheduler status before adding more complexity.
6. Maintain strict manual change discipline.

This is not the normal recommendation for operators who already use a supported LibreQoS integration.

Next:
- [Advanced Configuration Reference](configuration-advanced.md)
- [Troubleshooting](troubleshooting.md)

## 6) Common First-Run Mistakes

- Not deciding where permanent shaping changes should be made.
- Hand-editing files even though your integration is supposed to own them.
- Changing topology depth before passing the health gate.
- Skipping post-change service/log validation before pilot traffic.
- Treating low-data or cold-start pages as proof that shaping is broken.
- Changing too many things before you have one known-good baseline.

## 7) Done For Day 1 When

- You can sign in to WebUI successfully.
- Dashboard loads.
- Scheduler Status is healthy.
- No urgent/fatal startup errors are present.
- The expected topology or devices appear.
- One test subscriber/device behaves as expected.

## 8) Related Pages

- [Operating Modes and Source of Truth](operating-modes.md)
- [CRM/NMS Integrations](integrations.md)
- [Advanced Configuration Reference](configuration-advanced.md)
- [Troubleshooting](troubleshooting.md)
