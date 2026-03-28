# LibreQoS WebUI (Node Manager)

This page documents key WebUI (Node Manager) views and operational behavior in the local WebUI (`http://your_shaper_ip:9123`).

## Core Views

### Dashboard
- Widget-based overview of throughput, retransmits, RTT, flow counts, and queue activity.
- Dashboard content can vary by version and enabled features.
- Executive Summary provides a compact operational view for large networks, with drilldown pages for executive heatmaps and leaderboards.
- Bakery provides dedicated status for queue apply state, paginated recent Bakery events, qdisc safety/preflight results, and the existing circuit-activity graph.
- The Bakery and TreeGuard tabs now begin with a visual system-model row so operators can understand pipeline/control-loop behavior at a glance before drilling into tables.
- The Bakery `Pipeline` widget now includes the active apply progress bar directly in the pipeline card, including `%`, `completed / total tc commands`, and chunk progress during large full reloads.
- The Bakery top row now includes `Runtime Operations`, which shows live TreeGuard/Bakery topology mutations, deferred cleanup work, failures, dirty subtrees, and whether Bakery has frozen further incremental changes pending a full reload.
- `Recent Bakery Events` now splits each entry into stage, outcome, and scope so operators can quickly tell whether Bakery is planning, blocked in preflight, applying, cleaning up TreeGuard work, or has finished.
- `TreeGuard Activity` now emphasizes intent and outcome instead of a raw `persisted` flag, making dry-runs, applied changes, cleanup-pending actions, skips, and failures much easier to interpret.
- `TreeGuard Decision Impact` now splits operational warnings from true errors, uses amber instead of red for non-fatal warnings, and exposes the current warning/error text directly in the widget summary and hover tooltip.
- Bakery qdisc preflight now presents per-interface planned qdiscs as compact usage bars with qdisc-kind mix badges, plus a separate limits/memory table for safe budget, kernel limit, estimated qdisc memory, current available memory, and safety-floor headroom.
- Some charts may take a short time to populate after first opening a tab, especially on busy systems or immediately after a service restart.
- During a Bakery full reload, queue-count cards keep showing the last known HTB/CAKE/fq-codel values and mark them as `Reloading` instead of briefly zeroing them.

### Network Tree Overview
- Hierarchical topology view of nodes/circuits from the shaper perspective.
- Useful for spotting bottlenecks and parent/child utilization patterns.
- Tree detail pages show a breadcrumb path, branch counts, and status indicators for the selected node.
- `Node Details` summarizes the selected node’s type, branch size, configured rates, and current effective rate.
- `Node Snapshot` provides a quick visual summary of current throughput and QoO for the selected node.
- Attached circuits are shown in a dedicated table for the selected node.
- Administrators can save or clear `Operator Override` values where node-level overrides are supported. Read-only users and unsupported nodes continue to display current values without edit controls.
- Tree-page operator rate edits write operator-owned overrides to `lqos_overrides.json`; they do not rewrite legacy integration bandwidth CSV files.
- Tree-page operator rate edits require an administrator session, a stable node ID, and a non-generated node. Generated/integration-synthetic nodes remain read-only in this editor.

### Site Map
- Flat operational map of Sites and APs using imported node geodata.
- Defaults to QoO coloring with an RTT toggle, while marker size reflects recent combined throughput.
- Uses a 30-second client-side average from `NetworkTree` data rather than adding backend rollup work.
- APs can inherit parent site coordinates for display when explicit AP coordinates are missing.
- Initial map framing now prefers site coordinates for a closer first view, falling back to AP coordinates when no sites are mapped yet.
- Current builds use an Insight-hosted OpenStreetMap raster tile cache rather than a fully local basemap bundle.
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
- Current builds populate ASN executive context from bounded ASN-only executive page requests instead of subscribing to a full executive heatmap feed.
- The legacy `ASN Explorer` route now redirects here so older bookmarks still work.
- Empty results usually indicate low recent data volume rather than failure.

### Circuit page
- Circuit pages combine queue behavior, live throughput, RTT, retransmits, and per-flow troubleshooting for an individual subscriber/circuit.
- `Queue Dynamics` shows circuit throughput and RTT behavior over time, including an `Active Flows` KPI based on the same recent flow window used by the `Traffic Flows` table.
- `Queue Stats` shows the most recent 3 minutes of live queue history for the circuit as raw 1-second scatter samples, including backlog, delay, queue length, traffic, ECN marks, and drops.
- Queue Stats charts now use synchronized hover so operators can inspect the same second across all queue charts together.
- `Queue Tree` now presents the circuit's live upstream queue path in styled ancestor cards, with a path summary plus equal-width throughput, retransmit, and latency panels for each upstream node.
- `Traffic Flows` is a recent-flow operational table rather than a long-term history view.
- `Traffic Flows` includes paging and a `Hide Small Flows` filter so large busy circuits remain usable without trying to render every row at once.
- `Flow Sankey` emphasizes the hottest recent flows rather than every older retained flow.

### CPU Tree / CPU Weights
- Shows queue/circuit distribution by CPU core.
- Helps evaluate binpacking and load distribution behavior.

### Shaped Devices Editor
- CRUD editor for `ShapedDevices.csv`.
- Supports paging and filtering in current builds.
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
