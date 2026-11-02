# LibreQoS Node API

> **Documents the shipped v3 API.** The API is part of `lqosd`; it is not a
> separate service or package. See the [v3 documentation](../v3/index.md) for the
> rest of the product.

## What It Is

`lqosd` links the API as a library and serves it at `/api/v3` on the node
manager's listener (default `:9123`). There is nothing extra to install, enable,
download or update:

- API base path: `/api/v3`
- Interactive documentation (Swagger UI): `/api/v3/docs`
- Machine-readable specification: `/api/v3/openapi.json`
- Bind address, TLS and the listener itself: the node manager's (see
  [operate](../v3/operate.md))

Because the API shares the node manager's listener, exposing the node manager
exposes the API. Keep that listener on trusted management networks.

## Authentication

Every request needs a local API key in the `x-bearer` header:

```bash
curl -sS -H "x-bearer: $LQOS_API_KEY" http://<node-ip>:9123/api/v3/health
```

Create keys in the WebUI under **License & Services → Local API Keys**.
Administrators can generate up to 16 named keys. LibreQoS displays each key once,
stores only its SHA-256 digest and non-secret metadata in `/etc/lqos.conf`, and
cannot recover it later, so copy the complete `lqos_api_...` value into the client
immediately and treat it like a password.

Named keys do not expire automatically; revoke the ones you no longer need. The
verifier reads the current configuration on every request, so a creation or a
revoke takes effect immediately — no restart. The older single
`local_api.bearer_token` still works and is labelled **Legacy local API key** in
the UI; remove it once your clients have moved to named keys.

There is no license requirement, no circuit-count limit and no subscription gate
on API access: a valid key is the whole requirement.

## Source of Truth and Testing

Swagger on your own node is the complete reference and test surface for the build
you are running:

- Direct: `http://<node-ip>:9123/api/v3/docs`
- Behind the optional Caddy HTTPS proxy (which proxies the node manager):
  `https://<hostname>/api/v3/docs`

Use this page as a capability map. Use Swagger for the full endpoint inventory,
request and response schemas, and live testing.

Need definitions for persistence and runtime-impact terms? See the
[Glossary](glossary.md).

## What ISPs Can Do with the API

All paths below are relative to `/api/v3`.

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
- `GET /shaped_devices`
- `POST /shaped_devices/update`

`POST /network_json/set_site_speed` and `POST /network_json/set_site_speed_batch`
were removed in v3: they answer a JSON `404` with a key (`401` without) and are
not part of the mount. Use `POST /overrides/network_adjustments/site_speed`
(max bandwidth only) instead. The v3 API manual
(`src/src/rust/lqos_api/manual/`, generated into `/api/v3/openapi.json`) is the
single endpoint reference of record; this page is the operator summary.

The legacy topology writes (`/network_json/update`, `/network_json/node`,
`/network/reparent_site`) are retired: they answer `409` and point at the
LibreQoS Topology Manager. Reparent and site-speed changes belong in the overrides
endpoints.

### 4) Monitoring and Diagnostics

Read health, scheduler, throughput, flow, queue, and circuit status.

Representative endpoints:
- `GET /health`
- `GET /version`
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

Treat these as higher-risk actions in production.

## Persistence Model (Important)

Not all writes persist the same way:

- `overrides/*`: persistent policy state.
- `shaped_devices/update`: direct file replacement, often integration-overwritable.

The v2 `network_json/set_site_speed*` writes do not exist in v3 (JSON `404`);
site-speed changes are persistent override state via
`POST /overrides/network_adjustments/site_speed`.

If integrations are enabled, integration refresh cycles may overwrite direct file edits.

## Recommended Production Workflow

1. Validate endpoint behavior and payload schema in Swagger.
2. Apply the smallest change needed.
3. Verify the result with read-only checks (`/health`, `/scheduler_status`,
   circuit/throughput views).
4. Keep rollback snapshots for config/data-changing operations.

## Deployment Hardening

- Keep API access limited to trusted management networks.
- Do not expose the node manager listener directly to the public Internet.
- If remote access is needed, use a reverse proxy with TLS (see
  [Optional HTTPS With Caddy](https-caddy.md)).
- Restrict inbound access with firewall allowlists.
