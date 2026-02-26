# LibreQoS Node API

## Requirements

The `lqos_api` (Node API service) requires an active LibreQoS Insight subscription.

This is separate from base shaping limits:
- Core LibreQoS shaping can run up to 1000 subscribers without Insight.
- Higher subscriber limits depend on active Insight licensing.

## Source of Truth and Testing

Use Swagger on your node as the complete reference and test surface for your installed build:

- `http://<node-ip>:9122/api-docs`

Use this page as a capability map. Use Swagger for full endpoint inventory, request/response schemas, and live testing.

Need definitions for persistence and runtime-impact terms? See the [Glossary](glossary.md).

## Install and Enable

If installed via `.deb` (recommended), `lqos_api` is included at:

- `/opt/libreqos/src/bin/lqos_api`

Enable service:

```bash
sudo cp /opt/libreqos/src/bin/lqos_api.service.example /etc/systemd/system/lqos_api.service
sudo systemctl daemon-reload
sudo systemctl enable lqos_api
sudo systemctl start lqos_api
```

Update only the API binary:

```bash
cd /opt/libreqos/src
./update_api.sh
```

Verify service state:

```bash
sudo systemctl status lqos_api
```

## Authentication

Most endpoints require:

- Header: `x-bearer`
- Value: your Insight license key

## What ISPs Can Do with the API

### 1) Subscriber/Circuit Lifecycle

Provision, update, and remove subscriber/device records.

- Add or replace:
  - `POST /overrides/persistent_devices`
  - `POST /shaped_devices/update`
- Adjust speeds:
  - `POST /overrides/adjustments/circuit_speed`
  - `POST /overrides/adjustments/device_speed`
- Remove:
  - `DELETE /overrides/persistent_devices/by_circuit/{circuit_id}`
  - `DELETE /overrides/persistent_devices/by_device/{device_id}`
  - `POST /overrides/adjustments/remove_circuit`
  - `POST /overrides/adjustments/remove_device`

### 2) Override and Policy Management

Maintain persistent override policies for circuits, devices, sites, and UISP-specific overrides.

- `GET/POST/DELETE /overrides/adjustments*`
- `GET/POST/DELETE /overrides/network_adjustments*`
- `GET/POST/DELETE /overrides/uisp/bandwidth*`
- `GET/POST/DELETE /overrides/uisp/routes*`

### 3) Topology and Shaping Input Files

Inspect and update shaping/topology files used by runtime workflows.

- `GET /network_json/json`
- `GET /network_json/text`
- `POST /network_json/update`
- `POST /network_json/set_site_speed`
- `POST /network_json/set_site_speed_batch`
- `GET /shaped_devices`
- `POST /shaped_devices/update`

### 4) Monitoring and Diagnostics

Read health, scheduler, throughput, flow, queue, and circuit status.

Representative endpoints:
- `GET /health`
- `GET /status_snapshot`
- `GET /scheduler_status`
- `GET /circuit/{circuit_id}`
- `GET /search`
- `GET /current_throughput`
- `GET /queue_stats_total`
- `GET /warnings`
- `GET /urgent`, `GET /urgent/status`

### 5) Control and Reload Operations

Trigger operational control actions when needed.

- `POST /reload_libreqos`
- `POST /clear_hot_cache`

Treat these as higher-risk actions in production.

## Persistence Model (Important)

Not all writes persist the same way:

- `overrides/*`: persistent policy state.
- `network_json/set_site_speed*`: transient edits.
- `network_json/update` and `shaped_devices/update`: direct file replacement, often integration-overwritable.

If integrations are enabled, integration refresh cycles may overwrite direct file edits.

## Recommended Production Workflow

1. Validate endpoint behavior and payload schema in Swagger.
2. Apply the smallest change needed.
3. Verify result with read-only checks (`/health`, `/scheduler_status`, circuit/throughput views).
4. Keep rollback snapshots for config/data-changing operations.

## Deployment Hardening

- Keep API access limited to trusted management networks.
- Do not expose the API directly to the public Internet.
- If remote access is needed, use a reverse proxy with TLS and authentication.
- Restrict inbound access with firewall allowlists.
