# LibreQoS WebUI (Node Manager)

This page documents key WebUI (Node Manager) views and operational behavior in the local WebUI (`http://your_shaper_ip:9123`).

## Core Views

### Dashboard
- Widget-based overview of throughput, retransmits, RTT, flow counts, and queue activity.
- Dashboard content can vary by version and enabled features.
- Executive Summary provides a compact operational view for large networks, with a `Network Snapshot` focused on throughput, inventory, and Insight state plus drilldown pages for executive heatmaps and leaderboards.
- Bakery provides dedicated status for queue apply state, qdisc safety/preflight results, circuit live-change progress, and recent Bakery operations.
- The Bakery and TreeGuard tabs present a high-level pipeline or control-loop summary ahead of the more detailed tables.
- The Bakery `Pipeline` widget shows queue-control stages, apply state, verification state, and TC interval timing.
- `Runtime Operations` summarizes TreeGuard/Bakery topology mutations, deferred cleanup work, failures, and subtrees waiting for a full reload.
- `Recent Bakery Events` emphasizes grouped operations, with detailed event history available when deeper troubleshooting is needed.
- `TreeGuard Activity` emphasizes grouped operations, including SQM change batches, with detailed event history available when deeper troubleshooting is needed.
- `TreeGuard Control Loop` shows the current observe/evaluate/act state.
- `TreeGuard Decision Impact` focuses on current impact and any active warnings or errors.
- `TreeGuard State Mix` shows managed nodes, runtime virtualization, managed circuits, and the `cake / mixed / fq_codel` circuit SQM split.
- Bakery qdisc preflight summarizes planned per-interface qdisc usage and budget headroom before apply.
- Some charts may take a short time to populate after first opening a tab, especially on busy systems or immediately after a service restart.
- During a Bakery full reload, queue-count cards can temporarily continue showing the last known HTB/CAKE/fq-codel values and mark them as `Reloading`.

### Network Tree Overview
- Hierarchical topology view of nodes/circuits from the shaper perspective.
- Useful for spotting bottlenecks and parent/child utilization patterns.
- Tree detail pages show a breadcrumb path, branch counts, and status indicators for the selected node.
- `Node Details` summarizes the selected node’s type, branch size, configured rates, and effective rate.
- `Node Snapshot` provides a quick visual summary of throughput and QoO for the selected node.
- Attached circuits are shown in a dedicated table for the selected node.
- The attached-circuits IP column keeps rows compact by showing one address inline and collapsing additional addresses as `+X`, with the full list still available on hover.
- Ethernet-limited attached circuits can show inline `10M`, `100M`, or `1G` warning badges beside the `Plan (Mbps)` value; hovering explains the auto-cap and clicking the badge opens the dedicated Ethernet review page.
- Administrators can save or clear `Operator Override` values where node-level overrides are supported. Read-only users and unsupported nodes continue to display current values without edit controls.
- Tree-page operator rate edits write operator-owned overrides to `lqos_overrides.json`; they do not rewrite legacy integration bandwidth CSV files.
- Tree-page operator rate edits require an administrator session, a stable node ID, and a non-generated node. Generated/integration-synthetic nodes remain read-only in this editor.

### Site Map
- Flat operational map of Sites and APs using imported node geodata.
- Defaults to QoO coloring with an RTT toggle, while marker size reflects recent combined throughput.
- APs can inherit parent site coordinates for display when explicit AP coordinates are missing.
- Nearby site markers cluster and expand as the operator zooms or selects a cluster.
- APs without explicit coordinates are represented through their parent site and can be expanded temporarily around the selected site for inspection.
- Visible unclustered sites show labels as the operator zooms in, and the selected site keeps its label visible while it is being inspected.
- When browser redaction mode is enabled, Site Map replaces displayed site names with `[redacted]` while leaving the underlying topology data unchanged.
- Initial map framing prefers site coordinates for the first view, falling back to AP coordinates when no sites are mapped yet.
- Site Map uses an Insight-hosted OpenStreetMap raster tile cache.
- In dark mode, the raster underlay is muted and tinted client-side toward the same cooler blue/cyan palette used by Flow Globe, so roads and geography stay visible without the bright light-theme basemap glare.
- Site Map depends on outbound access to `https://insight.libreqos.com` for initial bbox/bootstrap and raster tile fetches.
- When tiles are missing from the remote cache, the browser retries automatically for a short period instead of failing immediately, so initial map paint can lag briefly on cold tiles.

### Flow Globe
- Geographic flow visualization based on endpoint geolocation.
- Uses a theme-aware globe with country borders for geographic context.
- Endpoint markers default to latency mode, with a toggle to switch between latency and throughput coloring.
- Marker size indicates recent traffic volume.
- Hover a marker for quick details, or click a marker/cluster to pin its details in the side panel.
- Best used when enough recent flow data is available.

### ASN Analysis
- Live ASN operations page combining a top-20 ASN leaderboard, latency-vs-traffic bubble chart, selected ASN KPI strip, 15-minute ASN trend chart, and embedded Flow Evidence.
- Supports `Impact` and `Throughput` ranking modes while keeping ASN flow evidence on the same page.
- Older `ASN Explorer` bookmarks continue to work through a redirect.
- Empty results usually indicate low recent data volume rather than failure.

### Circuit page
- Circuit pages combine queue behavior, live throughput, RTT, retransmits, and per-flow troubleshooting for an individual subscriber/circuit.
- When integration metadata reports a negotiated CPE Ethernet speed, the `Max` row can show a warning badge such as `100M`; hovering the badge explains when LibreQoS auto-capped shaping below the requested plan to stay within that port speed, and clicking the badge opens the Ethernet review page.
- `Queue Dynamics` shows circuit throughput and RTT behavior over time, including an `Active Flows` KPI based on the same recent flow window used by the `Traffic Flows` table.
- `Top ASNs` summarizes the busiest recent ASNs on the circuit from that same live flow window and sorts by current rate by default.
- `Devices` shows per-device detail tables and live charts for throughput, retransmits, and latency.
- `Queue Stats` shows recent live queue history for the circuit, including backlog, delay, queue length, traffic, ECN marks, and drops.
- Queue Stats charts use synchronized hover so operators can inspect the same second across all queue charts together.
- `Queue Tree` shows the circuit's live upstream queue path, including a path summary and per-node throughput, retransmit, and latency context.
- `Traffic Flows` is a recent-flow operational table rather than a long-term history view.
- `Traffic Flows` includes paging and a `Hide Small Flows` filter so large busy circuits remain usable without trying to render every row at once.
- `Traffic Flows` current-rate display is limited to plausible, plan-aware values for the circuit.
- Long text in the `Protocol`, `ASN`, and `Country` columns is truncated with an ellipsis to keep row height stable; the full value remains available on hover.
- `Flow Sankey` emphasizes the hottest recent flows rather than every older retained flow.

### Ethernet Caps
- The Ethernet review page is a lightweight operator table of circuits automatically down-rated because detected Ethernet speed was below the requested plan.
- It is intentionally not in the main navigation; operators reach it by clicking Ethernet warning badges on the Circuit page or Tree attached-circuits table.
- The page supports search, tier filtering (`10M`, `100M`, `1G+`), and paging across auto-capped circuits.

### CPU Tree / CPU Weights
- Shows queue/circuit distribution by CPU core.
- Helps evaluate binpacking and load distribution behavior.
- CPU Affinity starts with shaping CPUs only, while excluded or host-only cores can be shown when needed.

### Shaped Devices Editor
- CRUD editor for `ShapedDevices.csv`.
- Supports paging and filtering.
- Add, edit, and delete actions save immediately in the dedicated editor.

### Urgent Issues
- WebUI can display urgent operational issues generated by backend services.
- Examples include mapping/license-limit warnings and other high-priority health events.
- Operators can acknowledge/clear issues in the UI after review.
- Common codes include `MAPPED_CIRCUIT_LIMIT` and `TC_U16_OVERFLOW` (see [Troubleshooting](troubleshooting.md#urgent-issue-codes-and-first-actions)).

### Scheduler Status
- WebUI surfaces scheduler health/readiness.
- Use it to quickly verify periodic refresh operation after config/integration changes.
- If status indicates errors, correlate with:
  - `journalctl -u lqos_scheduler --since "30 minutes ago"`
  - `journalctl -u lqosd --since "30 minutes ago"`

## Privacy / Redaction Mode

- Toggle with the mask icon in the top navigation.
- Redaction is client-side and stored in browser local storage.
- Redaction helps hide PII in screenshots/demos.
- Site Map replaces displayed site names with `[redacted]` while redaction mode is enabled.
- Redaction does not modify `ShapedDevices.csv`, `network.json`, or any backend data.

## Common Empty-State Behavior

The following pages may appear sparse/blank when data is low:
- Site Map
- Flow Globe
- Tree Overview Sankey views
- ASN Analysis / Flow Evidence

When this happens:
1. Confirm `lqosd` is healthy.
2. Wait for fresh traffic data.
3. Reload the page.
4. Check logs:

```bash
journalctl -u lqosd --since "10 minutes ago"
```

If only Site Map is blank or slow while other pages are healthy, also check management-network reachability to `insight.libreqos.com` and allow a short delay for cold tile-cache retries.

## Useful Related Pages

- [Components](components.md)
- [Configuration](configuration.md)
- [Troubleshooting](troubleshooting.md)
