# CRM/NMS Integrations

**Version: LibreQoS v3.** How LibreQoS v3 imports subscribers and topology
from your CRM or NMS.

Integrations work the same way as in v2.0. They now run inside the
`libreqos-scheduler` service, which starts after `lqosd`, and the built-in
compiler turns what they import into the bridge's shaping configuration.

```{mermaid}
flowchart LR
    A[CRM/NMS] --> B[Integration job in libreqos-scheduler]
    B --> H[topology_import.json]
    H --> C[lqosd compiler]
    C --> D[DPDK bridge]
    C --> G[WebUI status]
```

## Choose your source of truth

| Path | Best fit | Where permanent shaping changes belong |
|---|---|---|
| Built-in integration | Operators using a supported CRM/NMS | The CRM/NMS, through the integration |
| Your own files | Operators with their own sync scripts | `network.json` and `ShapedDevices.csv`, which you generate |

With a built-in integration enabled, edits made directly to the imported data
may be replaced on the next refresh. If you maintain your own files, keep
writing `network.json` and `ShapedDevices.csv` as in v2.0.

## Built-in integrations

- [Splynx](integrations-splynx.md)
- [UISP](integrations-uisp.md)
- [Netzur](integrations-netzur.md)
- [VISP](integrations-visp.md)
- [WISPGate](integrations-wispgate.md)
- [Powercode](integrations-powercode.md)
- [Sonar](integrations-sonar.md)

```{toctree}
:hidden:

integrations-splynx
integrations-uisp
integrations-netzur
integrations-visp
integrations-wispgate
integrations-powercode
integrations-sonar
```

For PPPoE or DHCP-RADIUS networks where subscribers appear and disappear, see
[RADIUS accounting and dynamic circuits](radius.md).

## Where to configure them

- Settings: in the WebUI, **Configuration → Data Sources** picks the topology
  source and the shared integration defaults, and each integration has its own
  page under **Configuration**. The same settings live in
  `/etc/libreqos/lqos.conf`.
- Refresh interval: `queue_refresh_interval_mins`.
- After editing `lqos.conf` by hand: `sudo systemctl restart libreqos-scheduler`.
- Health after a change: the **Scheduler** and **Alerts** views in the WebUI,
  or
  `journalctl -u libreqos-scheduler --since "30 minutes ago"`.

The imported data lands in `/opt/libreqos/state/topology/topology_import.json`
and `/opt/libreqos/state/shaping/shaping_inputs.json`.

## Topology modes

Each integration supports a set of `topology.compile_mode` values:

- UISP: `flat`, `ap_only`, `ap_site`, `full`
- Splynx: `flat`, `ap_only`, `ap_site`, `full`
- Sonar: `flat`, `full`

`ap_site` is the recommended default. In v3 the mode changes only the shape of
the tree; the bridge handles deep trees natively, and virtual nodes no longer
exist (a node marked `virtual` is shaped as a real site).

## Node IDs

Nodes in `network.json` can carry an optional `"id"` field. When present,
LibreQoS uses it to match saved site bandwidth overrides; name-only matching
remains the fallback, so names must still be unique.

| Integration | Node ID support | Notes |
|---|---|---|
| UISP | Yes | Real sites and devices; synthetic nodes get stable generated IDs. |
| Splynx | Yes | Network sites and AP/site nodes. |
| Sonar | Yes | Site and AP nodes. |
| Netzur | Partial | Only when the zone data has a stable zone ID. |
| VISP | Partial | When VISP IRM relationships are populated; otherwise flat attachment. |
| Powercode | No | Builds no topology nodes. |
| WISPGate | No | Builds no topology nodes. |

## Client rates

For integrations that import raw plan speeds, LibreQoS sets each circuit's
maximum rate to
`max(plan_rate × bandwidth_overhead_factor, plan_rate × client_bandwidth_multiplier)`.
Integrations that already import effective shaped rates keep them as they are.

Legacy `integrationUISPbandwidths.csv` and `integrationSplynxBandwidths.csv`
files are migrated into `AdjustSiteSpeed` entries in `lqos_overrides.json` and
renamed to `.backup`.
