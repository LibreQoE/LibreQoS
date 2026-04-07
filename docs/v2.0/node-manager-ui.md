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
- `Runtime Operations` summarizes TreeGuard/Bakery topology mutations, deferred cleanup work, retryable failures, structurally blocked runtime operations, and subtrees waiting for a full reload.
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
- `Node Details` summarizes the selected node’s configured rates, override state, and effective rate.
- `Node Snapshot` provides a quick visual summary of throughput and QoO for the selected node.
- Attached circuits are shown in a dedicated table for the selected node.
- The attached-circuits IP column keeps rows compact by showing one address inline and collapsing additional addresses as `+X`, with the full list still available on hover.
- Ethernet-limited attached circuits can show inline `10M`, `100M`, or `1G` warning badges beside the `Plan (Mbps)` value; hovering explains the auto-cap and clicking the badge opens the dedicated Ethernet review page.
- Administrators can save or clear `Rate Override` values where node-level overrides are supported. Read-only users and unsupported nodes continue to display current values without edit controls.
- Tree-page operator rate edits write operator-owned `AdjustSiteSpeed` overrides to `lqos_overrides.json`.
- On UISP `full` builds, legacy `integrationUISPbandwidths.csv` files are auto-migrated into those operator overrides on the next integration run when no operator rate overrides exist yet; otherwise the CSV is ignored.
- Tree-page operator rate edits require an administrator session, a stable node ID, and a non-generated node. Generated/integration-synthetic nodes remain read-only in this editor.
- The tree page keeps `Node Details` as a compact summary card and places the override editors as compact inline rows directly beneath the details table.
- The tree page now shows topology override state as read-only summary only. Parent and attachment edits have moved to `Topology`, and the tree details panel links directly into Topology Manager for the selected node.
- Runtime-selected attachment hops are collapsed out of the main tree hierarchy. When a site is currently using a specific backhaul or radio path, `Node Details` shows that as `Active Attachment` metadata on the site instead of exposing the attachment as its own branch node.
- Inactive alternate PtP/wired backhaul attachment stubs are also pruned from the runtime tree. `tree.html` shows the effective path only; dormant alternates remain visible in Topology Manager rather than as standalone runtime nodes.
- On the synthetic `Root` node, topology override still shows as not applicable rather than as a generic missing-node-ID warning.

### Topology Manager
- Focused branch-reparenting editor for integration-backed topology state, intended primarily for branch/topology nodes rather than end-customer leaves.
- The page places a full-width hierarchy summary comparison directly under the hero, then uses the main workspace for the focused move preview plus a details panel for legal parents and attachment/radio preferences.
- The page keeps the hero, hierarchy summary, and details actions intentionally compact so the branch preview, current attachment health, and move controls stay higher on screen without excessive vertical whitespace.
- The right-hand Details panel now uses a compact branch summary plus a single attachment-focused section, so `Current Attachment Preference`, `Attachment Health`, and the `Edit Attachment Preference` path live together instead of repeating the same branch state across multiple cards.
- The `Edit Link Path` action is owned by the primary Details-panel action row beside `Start Move`; the `Radio Paths` section itself is read-only summary plus health/attachment data until that edit mode is entered.
- In the Details panel, the selected branch name links directly to that branch's `tree.html` view when the node has a stable topology ID, so operators can jump from move/edit context back to the runtime tree quickly.
- For nodes with multiple radio paths, the Details panel now emphasizes attachment decisions before branch moves: attachment cards highlight `Using Now` and `Preferred` state directly, while `Start Move` is visually demoted until the operator is actually rehoming the branch.
- Default selection prefers nodes with richer branch context, such as saved overrides, movable branch nodes, site-level objects, descendants, and multiple legal parent targets.
- Search shows live matching suggestions and is biased toward exact or branch-relevant matches so operator queries land on the intended site/branch more reliably than simple alphabetical matching.
- The page opens in inspect mode. Clicking nodes changes selection and lets the operator navigate upstream/downstream branch context in the preview; it does not change parentage.
- The current selected node is mirrored into the page URL, so refreshes and shared links reopen Topology Manager on the same node when that node still exists in the current topology state. When the page is opened without a `node_id`, it now defaults to the synthetic `Root` view before falling back to the ranked branch picker.
- In the Move Preview, deep upstream ancestry is collapsed into a compact stub after the nearest two upstream nodes so long branch chains do not crush the left side of the map. The full breadcrumb remains visible in the hierarchy summary above.
- While the page is open, Topology Manager lightly auto-refreshes topology-manager state in the background so attachment health and suppression state changes show up without a manual reload. Active move or attachment drafts stay in place unless the selected node itself disappears, and the refresh loop now defers itself while an operator is typing in Details-panel edit fields so the page does not steal focus mid-edit.
- Parent changes are gated behind an explicit `Start Move` step in the Details panel. In move mode, legal parent targets are highlighted in green and can be chosen from the target cards or by dragging the selected branch onto a highlighted target.
- For UISP-backed sites, direct inter-site radio links can also surface sibling or alternate upstream sites as legal parent targets when the linked remote radio reaches the root without traversing the local site first.
- Exported shaping/runtime trees anchor those cross-site moves under the parent-side peer attachment so the moved site stays visible after regeneration.
- Canonical integration snapshots stay integration-derived; saved Topology Manager re-parents are applied only in the runtime-effective topology layer, which now refuses to publish an effective tree that drops or duplicates a site.
- Saving a Topology Manager move, probe-policy change, manual attachment group, or attachment-rate override now triggers an immediate runtime-effective recompute and publish; routine topology edits do not need a fresh UISP import to take effect.
- When a move or attachment-preference save is in flight, the primary `Save` button switches to a short spinner/disabled state so operators can see that the runtime-effective publish is still underway.
- Effective publish validation is ID-based and fail-closed. Before validation, the runtime-effective compiler normalizes duplicate canonical attachment/device candidates by stable `node_id` so UISP duplicate radio rows do not block otherwise valid site moves. If a saved change would still select an invalid parent or attachment, create a cycle, duplicate a site, or drop a site from the exported tree, LibreQoS rejects the publish and keeps the last good effective topology in place.
- Canonical and effective topology snapshots are written with atomic file replacement so normal refreshes do not rely on partially written JSON artifacts.
- The Hierarchy Summary now shows three distinct states: `Canonical` for the latest integration snapshot, `Live` for the runtime-effective topology currently used for shaping, and `Saved`/`Proposed` for operator intent. When a saved move is already active in the runtime-effective topology, the status chip reads `Live Override`; otherwise it remains `Saved Override`.
- Main operator-facing hierarchy chips now prefer display names only. If a label cannot be resolved, the page shows a neutral placeholder instead of a raw internal node ID.
- Attachment or radio preference edits for the current logical parent do not require move mode; when multiple explicit attachments are available, inspect mode exposes a lighter `Edit Attachment Preference` path in Details.
- Attachment rows now surface runtime health directly in the page, including `Healthy`, `Suppressed`, `Probe Unavailable`, or `Probe Disabled` state, plus reason text, suppression hold-down timestamps, and per-pair probe enable/disable actions.
- UISP attachment rows now also classify the feed role, such as `PtP Backhaul`, `PtMP Uplink`, or `Wired Uplink`, so operators can tell a real backhaul path from an access/uplink path without inferring it from raw AP names.
- The single runtime debug file for topology probes is `topology_attachment_health_state.json`. It now carries the probe pair ID plus attachment name/ID, child and parent node names/IDs, configured local/remote probe IPs, enable/probeable state, recent endpoint reachability, and current suppression/health counters. The Details panel links directly to probe debugging from `Attachment Health`.
- Attachment Health now also exposes attachment-scoped rate overrides for editable attachments. These overrides are directional (`download` / `upload`), are stored in `topology_overrides.json`, and apply only to the selected `(child node, parent node, attachment)` path instead of behaving like a node-wide `AdjustSiteSpeed`.
- Dynamic UISP radio-capacity attachments stay read-only in this editor. Static UISP attachments, manual attachment groups, and other non-dynamic attachment sources can expose `Attachment Rate` controls directly in the attachment row.
- When UISP automatically transport-caps an attachment because the active or known Ethernet ports cannot carry the raw reported radio capacity, Attachment Health shows that cap reason inline so operators can see why a 2G or 2.7G radio exported as a lower effective topology rate.
- Automatic probe targets for UISP-backed attachments come from the management IPs UISP reports for the two radios/devices in that pair. These topology probe IPs are no longer limited by shaping `allow_subnets`; they are treated as management-plane data rather than customer shaping addresses.
- Operators can create, edit, or clear manual attachment groups from the Details panel for a legal child/parent pair. Manual groups define explicit parallel attachments, including ordered preference, capacity, management IPs, and probe opt-in, without hand-editing JSON.
- The focused SVG graph auto-centers and stretches the selected branch context to use more of the available map area while keeping the view bounded to the current path, children, and legal move context instead of rendering the full network at once.
- Child fanout on the right side of the focused preview is capped and biased toward branch-bearing or movable children; dead-end leaves are deprioritized or hidden when richer branch context is available.
- When UISP exposes multiple parallel links between the same two sites, Topology Manager groups them under one logical parent target and exposes the concrete radios/devices as explicit attachment preferences.
- `tree.html` only squashes runtime-selected UISP attachments when the effective attachment role is a true backhaul-style path (`PtP Backhaul` or `Wired Uplink`). PtMP access/uplink APs remain visible in the runtime tree.
- When a chosen parent exposes only one valid explicit attachment option, the UI auto-saves that override immediately.
- Saved topology overrides are stored as operator-owned intent in `topology_overrides.json`, separate from integration-generated runtime editor state.

### Topology Probes
- Read-only debug page for system-wide topology probe troubleshooting, linked from Topology Manager rather than promoted as a primary sidebar destination.
- Loads from the single runtime snapshot `topology_attachment_health_state.json`.
- Defaults to enabled probes only, with filters to switch to all or disabled rows when troubleshooting.
- Uses the same denser config-panel styling as other operational inventory pages, with a small status summary strip above the probe table.
- Each row shows child node, parent node, attachment, probe IPs, current runtime health/suppression state, endpoint reachability, and a direct link back into Topology Manager.

### Site Map
- Flat operational map of Sites and APs using imported node geodata.
- Defaults to QoO coloring with an RTT toggle, while marker size reflects recent combined throughput.
- APs can inherit parent site coordinates for display when explicit AP coordinates are missing.
- Nearby site markers cluster and expand as the operator zooms or selects a cluster.
- APs without explicit coordinates are represented through their parent site and can be expanded temporarily around the selected site for inspection.
- When a site or AP is selected and the right-hand details panel opens, the selected node name links directly to that node's `tree.html` page when a stable node ID is available.
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
- During startup or scheduled refresh work, the left navigation now shows a phase-aware circular progress ring instead of a generic spinner.
- The scheduler modal includes the current phase label, step position, coarse percent, and any recent output/error text.
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
