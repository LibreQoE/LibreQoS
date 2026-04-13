# CRM/NMS Integrations

## Page Purpose

Use this page to choose and configure a supported CRM/NMS integration. Use the per-integration pages for setup details.

Need definitions for common terms? See the [Glossary](glossary.md).

Most operators use built-in integrations.
If you have not selected a deployment path yet, start with [Quickstart](quickstart.md).

Need the full topology/shaping file-flow model? See [Topology Data Flow](topology-data-flow.md).
For static queue-visibility rules and virtualization semantics, see [Advanced Configuration Reference](configuration-advanced.md) and [LibreQoS WebUI (Node Manager)](node-manager-ui.md).

```{mermaid}
flowchart LR
    A[CRM/NMS] --> B[LibreQoS Integration Job]
    B --> H[topology_import.json]
    H --> E[lqos_scheduler]
    E --> F[Shaping Updates]
    E --> G[WebUI Status / Urgent Issues]
```

## Choose Your Integration Path

| Path | Best fit | Where permanent shaping changes belong |
|---|---|---|
| Built-in integration | Most operators using supported systems | CRM/NMS via LibreQoS integration jobs |
| Custom source of truth | Operators with in-house CRM/NMS sync logic | External generated files (`network.json`, `ShapedDevices.csv`) |

## Where in WebUI

- Integration defaults/common behavior: `Configuration -> Integrations`
- Per-integration configuration fields: `Configuration -> Integrations`
- Operational health checks after sync changes: `WebUI -> Scheduler Status` and `WebUI -> Urgent Issues`
- Topology/result validation: `WebUI -> Network Tree Overview` and `Flow Globe`

## Built-In Integrations

- [Splynx Integration](integrations-splynx.md)
- [UISP Integration](integrations-uisp.md)
- [Netzur Integration](integrations-netzur.md)
- [VISP Integration](integrations-visp.md)
- [WISPGate Integration](integrations-wispgate.md)
- [Powercode Integration](integrations-powercode.md)
- [Sonar Integration](integrations-sonar.md)

## Important Refresh Behavior

When integrations are enabled:
- Integration sync refreshes the imported topology and shaping data LibreQoS uses.
- Built-in integrations do not use `network.json` or `ShapedDevices.csv` as their normal output files.
- Built-in integrations provide native infrastructure topology to Topology Manager; legacy compatibility trees remain derived/export-only data.
- Python-backed integrations currently import infrastructure conservatively as fixed roots or fixed-parent branches. When bounded alternative-parent candidates are available, they are limited to the node's current local parent neighborhood, plus peer root parents of the same type when the current parent is itself a root, rather than the whole imported graph.
- If you run a DIY or manual deployment, `network.json` and `ShapedDevices.csv` remain the files you maintain.
- Direct edits to integration-managed data may be replaced on the next refresh cycle.

Mode capabilities are now explicit by integration:
- UISP: `flat`, `ap_only`, `ap_site`, `full`
- Splynx: `flat`, `ap_only`, `ap_site`, `full`
- Sonar: `flat`, `full`
- Manual edits may be overwritten on the next refresh cycle.

Legacy `integrationUISPbandwidths.csv` and `integrationSplynxBandwidths.csv` files are auto-migrated into operator `AdjustSiteSpeed` entries in `lqos_overrides.json` and renamed to `.backup` files.

DIY/manual deployments continue to use `network.json` and `ShapedDevices.csv` as their primary input files.

## Topology Node ID Support

LibreQoS supports an optional generic `"id"` field on `network.json` nodes. This field is intended to carry stable node identifiers from the integration source where possible.

Current behavior:
- when a node ID is available, LibreQoS uses it to match saved site bandwidth overrides more reliably
- older name-only matching still works as a fallback for older data
- topology names still need to remain globally unique in `network.json`

| Integration | `network.json` node ID support | Notes |
|---|---|---|
| UISP | Yes | Real UISP sites/devices export generic `id` plus existing `uisp_site` / `uisp_device` metadata. Synthetic LibreQoS nodes use stable generated IDs. |
| Splynx | Yes | Network sites and AP/site topology nodes export generic `id`. |
| Sonar | Yes | Site and AP topology nodes export generic `id`. |
| Netzur | Partial | Exported only when the upstream zone data includes a stable zone ID. |
| VISP | Partial | Imports site/upstream topology and stable generic IDs when VISP IRM relationships are populated; subscribers without usable upstream mapping still fall back to flat attachment. |
| Powercode | No | Current importer does not build topology nodes in `network.json`. |
| WISPGate | No | Current importer does not build topology nodes from stable upstream topology identifiers. |

## Common Client Rate Handling

For built-in integrations that import raw subscriber plan speeds, LibreQoS applies the same shared client-rate rule before writing `ShapedDevices.csv`:

- effective client max rate = `max(plan_rate * bandwidth_overhead_factor, plan_rate * client_bandwidth_multiplier)`

Integrations that already ingest effective shaped rates keep those values as-is rather than applying the multiplier a second time.

## Related Pages

- [Quickstart](quickstart.md)
- [Topology Data Flow](topology-data-flow.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [Configure LibreQoS](configuration.md)
- [Scale Planning and Topology Design](scale-topology.md)
- [Troubleshooting](troubleshooting.md)
