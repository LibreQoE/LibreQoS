# LibreQoS Node API

## Requirements

The LibreQoS Node API requires an active LibreQoS Insight subscription.

## Installation

### If installed via `.deb` (recommended)

`lqos_api` is installed with LibreQoS into:

`/opt/libreqos/src/bin/lqos_api`

### Test run

```bash
/opt/libreqos/src/bin/lqos_api
```

### Systemd Service

The package includes a template service file:

```bash
sudo cp /opt/libreqos/src/bin/lqos_api.service.example /etc/systemd/system/lqos_api.service
```

Then run:

```bash
sudo systemctl daemon-reload
sudo systemctl enable lqos_api
sudo systemctl start lqos_api
```

### Update just the API binary

Use the helper script:

```bash
cd /opt/libreqos/src
./update_api.sh
```

This downloads the latest `lqos_api` and installs it to `/opt/libreqos/src/bin/lqos_api`.

You can optionally skip service restart:

```bash
./update_api.sh --no-restart
```

Or install to an alternate location:

```bash
./update_api.sh --bin-dir /some/path/bin --no-restart
```

### Manual service file (if needed)

If you are creating the service manually, use:

```
[Unit]
After=network.service lqosd.service
Requires=lqosd.service

[Service]
WorkingDirectory=/opt/libreqos/src/bin
ExecStart=/opt/libreqos/src/bin/lqos_api
Restart=always

[Install]
WantedBy=default.target
```

Then run:

```bash
sudo systemctl daemon-reload
sudo systemctl enable lqos_api
sudo systemctl start lqos_api
```

## Usage

See `localhost:9122/api-docs` for the Swagger UI.

`/api-docs` is the source of truth for your installed version. Endpoint availability can change between releases.

## Deployment and Hardening Guidance

Recommended production posture:

1. Keep the Node API reachable only from trusted management networks.
2. Do not expose the API directly to the public Internet.
3. If remote access is required, place it behind a reverse proxy with TLS and authentication.
4. Restrict inbound access with host/network firewall allowlists.
5. Monitor API and service logs for abnormal access patterns.

If the API is not needed on a node, disable the service:

```bash
sudo systemctl disable --now lqos_api
```

If enabled for operations, verify service state after upgrades:

```bash
sudo systemctl status lqos_api
```

## API Endpoints

### Health & System
- `/health` - Service health check
- `/reload_libreqos` - Reload LibreQoS configuration
- `/validate_shaped_devices` - Validate shaped devices CSV
- `/lqos_stats` - Get lqosd internal statistics
- `/stormguard_stats` - Get Stormguard statistics
- `/bakery_stats` - Get Bakery active circuits count

### Traffic Metrics
- `/current_throughput` - Current network throughput stats
- `/top_n_downloaders/{start}/{end}` - Top downloaders by traffic
- `/worst_rtt/{start}/{end}` - Hosts with worst round-trip time
- `/worst_retransmits/{start}/{end}` - Hosts with worst TCP retransmits
- `/best_rtt/{start}/{end}` - Hosts with best round-trip time
- `/host_counter` - All host traffic counters

### Network Topology
- `/network_map/{parent}` - Network map from parent node
- `/full_network_map` - Complete network topology
- `/top_map_queues/{n}` - Top N queues by traffic
- `/node_names` - Get node names from IDs
- `/funnel/{target}` - Analyze traffic funnel to circuit

### Circuit Management
- `/all_circuits` - List all circuits
- `/raw_queue_data/{circuit_id}` - Raw queue data for circuit

### Flow Analysis
- `/dump_active_flows` - Dump all active flows (slow)
- `/count_active_flows` - Count of active flows
- `/top_flows/{flow_type}/{n}` - Top flows by metric type
- `/flows_by_ip/{ip}` - Flows for specific IP
- `/flow_duration` - Flow duration statistics

### Geo & Protocol Stats
- `/current_endpoints_by_country` - Traffic endpoints by country
- `/current_endpoint_latlon` - Endpoint geographic coordinates
- `/ether_protocol_summary` - Ethernet protocol statistics
- `/ip_protocol_summary` - IP protocol statistics

## Newer API Coverage Areas

Recent builds expanded API coverage in a few areas. Depending on your version, the API may also include:

- Alerts and warnings summaries
- Device and circuit count summaries
- Single-circuit detail lookups
- Search result helpers for UI workflows
- Scheduler and queue-stat detail endpoints

Use `localhost:9122/api-docs` to confirm exact paths and schemas on your node.

## Capturing an endpoint inventory for your release

For release notes or internal runbooks, capture a snapshot of API paths from your node:

```bash
curl -s http://localhost:9122/api-docs | jq -r '.paths | keys[]' | sort
```

Save this output with your release artifacts so operators can compare endpoint differences between versions.
