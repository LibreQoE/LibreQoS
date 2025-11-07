# LibreQoS Node API

## Requirements

The LibreQoS Node API requires an active LibreQoS Insight subscription.

## Installation

### Download

```
cd ~
sudo apt-get update
sudo apt-get upgrade
wget https://libreqos.io/wp-content/uploads/2025/09/lqos_api.zip
sudo apt-get install unzip
sudo mkdir /opt/lqos_api/
sudo chown -R $USER /opt/lqos_api/
cd /opt/lqos_api/
unzip lqos_api.zip
```

### Test run
```
cd /opt/lqos_api/
./lqos_api
```

### Systemd Service

If the test run succeeds, use `sudo nano /etc/systemd/system/lqos_api.service` and paste these contents:

```
[Unit]
After=network.service lqosd.service
Requires=lqosd.service

[Service]
WorkingDirectory=/opt/lqos_api/
ExecStart=/opt/lqos_api/lqos_api
Restart=always

[Install]
WantedBy=default.target
```
Then run:
```
sudo systemctl daemon-reload
sudo systemctl enable lqos_api
sudo systemctl start lqos_api
```

## Usage

See `localhost:9122/api-docs` for the Swagger UI.

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
