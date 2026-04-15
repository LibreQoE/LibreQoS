# Quickstart: WebUI Deployment Path

Use this page to move from package install to a safe pilot with the fewest moving parts.

Follow this page in order:
1. Complete the common install foundation.
2. Open the WebUI and create the first admin user if needed.
3. Use `Complete Setup` to choose where subscriber and topology data will come from.
4. Pass the 10-minute health check.
5. Start with limited pilot traffic before broad rollout.

Need definitions for key terms? See the [Glossary](glossary.md).

## 1) Common Install Foundation

Complete this once before trying to shape live traffic:

1. Review architecture and sizing:
- [Deployment Scenarios](design.md)
- [System Requirements](requirements.md)

2. Prepare the host and operating system:
- [Server Setup - Prerequisites](prereq.md)
- [Install Ubuntu Server 24.04](ubuntu-server.md)

3. Install LibreQoS (`.deb` recommended):

```bash
cd /tmp
sudo apt-get update
sudo apt-get upgrade
wget https://download.libreqos.com/{deb_url_v1_5}
sudo apt install ./{deb_url_v1_5}
```

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

## 2) Open WebUI And Complete First Login

1. Open the WebUI at `http://your_shaper_ip:9123`.
2. If no WebUI users exist yet, LibreQoS redirects you to `first-run.html`.
3. Create the initial WebUI admin user if prompted.
4. Sign in and confirm the Dashboard opens.

At this point, the WebUI being available does not yet prove that LibreQoS is ready to shape subscribers.

If Scheduler Status says `Setup Required`, that is expected until you choose a topology source and publish usable shaping data.

## 3) Use `Complete Setup` To Choose Your Topology Source

After you can sign in, open `Complete Setup`.

This page is where most ISPs should make the next decision. Choose one source of truth for persistent shaping changes:

| If this describes you | Use this path | Where permanent shaping changes belong |
|---|---|---|
| You use a supported CRM/NMS such as UISP, Splynx, VISP, Netzur, Powercode, Sonar, or WispGate | Built-in integration | Your integration system and its LibreQoS integration settings |
| You already have your own internal importer | Custom importer | Your external script or process |
| You intentionally maintain files by hand | Manual files | `network.json` and `ShapedDevices.csv` |

Rule: pick one place for permanent shaping changes. Do not mix manual edits with scheduled integration refreshes unless you intentionally want that complexity.

### Built-in Integration

This is the recommended path for most ISPs.

Do this now:
1. Open the provider page from `Complete Setup`.
2. Save the integration settings.
3. Run the initial sync or wait for the first scheduled import.
4. Return to Scheduler Status and confirm LibreQoS is no longer waiting on setup.

Next:
- [CRM/NMS Integrations](integrations.md)
- [Troubleshooting](troubleshooting.md)

### Custom Importer

Choose this only if another internal process already writes LibreQoS-compatible shaping files.

Do this now:
1. Configure shared topology behavior under `Integration - Common`.
2. Publish `network.json` and `ShapedDevices.csv` from your own process.
3. Reload or wait for the scheduler so LibreQoS can validate and use those files.

Next:
- [Operating Modes and Source of Truth](operating-modes.md)
- [Advanced Configuration Reference](configuration-advanced.md)

### Manual Files

Choose this only if you intentionally want LibreQoS to own the files directly.

Do this now:
1. Build `network.json`.
2. Build `ShapedDevices.csv`.
3. Use the WebUI editors or file-based workflow to maintain them.
4. Confirm the scheduler accepts the data and the expected topology appears.

Next:
- [Advanced Configuration Reference](configuration-advanced.md)
- [Troubleshooting](troubleshooting.md)

## 4) 10-Minute Health Check

After `Complete Setup` is finished and your chosen data source has published usable data, run:

```bash
sudo systemctl status lqosd lqos_scheduler
journalctl -u lqosd -u lqos_scheduler --since "10 minutes ago"
```

Confirm:
- The Dashboard loads.
- `lqosd` and `lqos_scheduler` are active.
- Scheduler Status is no longer `Setup Required`.
- Scheduler Status is healthy, or clearly shows expected active work without validation or startup errors.
- No urgent or fatal startup problems appear in the logs.
- The expected topology or subscriber/device list appears in WebUI.

If this fails, go to [Troubleshooting](troubleshooting.md) before pilot traffic.

## 5) Start With A Limited Pilot

Do not begin with broad inline rollout.

Start with a small pilot and confirm:
- one test subscriber/device shapes as expected
- expected parent nodes and hierarchy depth appear
- Scheduler Status stays healthy after refreshes
- no new urgent errors appear in logs after the first shaping cycles

Expand only after you have one known-good baseline.

## 6) Common Early Mistakes

- Treating `Dashboard loads` as proof that shaping is ready.
- Ignoring `Setup Required` and assuming the scheduler is already shaping customers.
- Mixing integration-owned data with manual file edits.
- Changing too many topology details before one clean health check.
- Starting a broad rollout before validating a small pilot.

## 7) Day 1 Is Done When

- You can sign in successfully.
- The Dashboard loads.
- `Complete Setup` is finished for your chosen workflow.
- Scheduler Status is no longer `Setup Required`.
- No urgent or fatal startup issues remain.
- The expected topology or subscriber list appears.
- One pilot subscriber/device behaves as expected.

## 8) Related Pages

- [Configure LibreQoS](configuration.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [CRM/NMS Integrations](integrations.md)
- [Advanced Configuration Reference](configuration-advanced.md)
- [Troubleshooting](troubleshooting.md)
