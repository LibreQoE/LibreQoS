# Advanced Configuration Reference

Use this page when you need CLI-driven configuration, direct file editing, or deep reference details.

For the full topology/shaping pipeline and file-role diagrams, see [Topology Data Flow](topology-data-flow.md).

## Topology Pattern Guardrails

Use these guardrails before deeper tuning:

- Single-interface (on-a-stick): supported, but queue count and directional mapping must be validated after any interface/queue change.
- VLAN-heavy designs: supported when interface roles and topology parent mapping are clear; avoid mixing ambiguous parent definitions across multiple systems.
- Integration users: do not manually maintain long-term conflicting edits in files that integration refresh cycles regenerate.

If results diverge from expectations after edits, use [Troubleshooting](troubleshooting.md) before additional changes.

```{warning}
If built-in integration mode is enabled, do not treat `network.json` or `ShapedDevices.csv` as the durable source of truth. Use your CRM/NMS, topology settings, and `lqos_overrides.json` for durable changes.
```

## Configuration via Command Line

You can also modify settings using the command line.

### Main Configuration File
#### /etc/lqos.conf

The configuration for each LibreQoS shaper box is stored in the file `/etc/lqos.conf`.

Edit the file to match your setup with

```shell
sudo nano /etc/lqos.conf
```

In the ```[bridge]``` section, change `to_internet` and `to_network` to match your network interfaces.
- `to_internet = "enp1s0f1"`
- `to_network = "enp1s0f2"`

In the `[bridge]` section of the lqos.conf file, you can enable or disable the XDP Bridge with the setting `use_xdp_bridge`. The default value is `false` - because the default setup assumes a [Linux Bridge](prereq.md). If you chose to use the XDP Bridge during that pre-requisites setup, please set `use_xdp_bridge = true` instead.

- Set downlink_bandwidth_mbps and uplink_bandwidth_mbps to match the bandwidth in Mbps of your network's upstream / WAN internet connection. The same can be done for generated_pn_download_mbps and generated_pn_upload_mbps.
- to_internet would be the interface facing your edge router and the broader internet
- to_network would be the interface facing your core router (or bridged internal network if your network is bridged)

Note: If you find that traffic is not being shaped when it should, please make sure to swap the interface order and restart lqosd as well as lqos_scheduler with ```sudo systemctl restart lqosd lqos_scheduler```.

After changing any part of `/etc/lqos.conf` it is highly recommended to always restart lqosd, using `sudo systemctl restart lqosd`. This re-parses any new values in lqos.conf, making those new values accessible to both the Rust and Python sides of the code.

Optional cobrand logo:
- `display_cobrand` is an optional top-level boolean in `/etc/lqos.conf`.
- If the key is omitted, LibreQoS treats it as `false`.
- The WebUI only shows the operator logo when `display_cobrand = true` and `cobrand.png` exists in the runtime static assets directory.

#### Netflow (optional)
To enable netflow, add the following `[flows]` section to the `/etc/lqos.conf` configuration file, setting the appropriate `netflow_ip`:
```
[flows]
flow_timeout_seconds = 30
netflow_enabled = true
netflow_port = 2055
netflow_ip = "100.100.100.100"
netflow_version = 5
do_not_track_subnets = ["192.168.0.0/16"]
```

#### On-a-stick mode queue mapping (single interface)

When running on-a-stick mode, LibreQoS splits available TX queues in half:
- first half for one direction
- second half for the reverse direction

So if 16 queues are available, each direction gets 8 queues. This directional offset is computed automatically at startup.

If your NIC exposes unusual queue counts, you can set `override_available_queues` in `[queues]` and restart `lqosd`.

If shaping appears asymmetric in on-a-stick deployments, verify:
- the interface has enough TX queues
- `override_available_queues` is not forcing an incorrect value
- you have restarted after config changes

See also [Troubleshooting](troubleshooting.md).

#### Source of Truth Boundary for Integration Users

If built-in integration mode is enabled, the integration and topology compiler own the working topology/shaping artifacts. DIY/manual mode remains the place where `network.json` and `ShapedDevices.csv` are durable inputs.

- Use WebUI/manual edits for short operational adjustments only.
- Put permanent changes in your integration system, integration overrides, or declared external source of truth workflow.

#### Static queue visibility policy

Current builds separate logical topology from queue-visible topology.

- Topology Manager keeps the full logical tree.
- `network.effective.json`, `shaping_inputs.json`, `tree.html`, and HTB planning use the queue-visible runtime tree.
- Large aggregation nodes can be auto-virtualized for queueing while still remaining visible in the runtime tree and Topology Manager.

The topology config now includes:

```toml
[topology]
queue_auto_virtualize_threshold_mbps = 5000
```

In the WebUI, this lives at `Configuration -> Integration - Common` as `Queue Auto-Virtualize Threshold (Mbps)`.

How it works:

- `queue_auto_virtualize_threshold_mbps` is the static queue-policy threshold used by `lqos_topology`.
- Integration roots default to static virtual nodes in the runtime tree.
- Site nodes above the threshold may also be virtualized automatically when they are acting as aggregation-only branches.
- Nodes with directly attached circuits stay queue-visible by default.

This static queue policy is now the primary way to avoid wasting HTB depth or creating artificial aggregate choke points. TreeGuard runtime link virtualization remains available, but is disabled by default.

##### `QueueAuto` decision flow

`QueueAuto` is a static topology policy resolved by `lqos_topology` during runtime-effective export. It is not the same thing as TreeGuard runtime virtualization.

```{mermaid}
flowchart TD
    A[Node queue policy = QueueAuto] --> B{Is this a root node?}
    B -->|Yes| C[Hide for queueing / promote children]
    B -->|No| D{Is this a Site node?}
    D -->|No| E[Keep queue-visible]
    D -->|Yes| F{Does it have child branches?}
    F -->|No| G[Keep queue-visible]
    F -->|Yes| H{Final effective node rate >= threshold?}
    H -->|No| I[Keep queue-visible]
    H -->|Yes| J[Mark static virtual for queueing]
```

Current rule summary:

- Root nodes default to queue-hidden or static virtual behavior depending on policy.
- Non-`Site` nodes stay queue-visible under `QueueAuto`.
- A `Site` with no child branches stays queue-visible.
- A `Site` with child branches only becomes static virtual when its final effective node rate is at or above `queue_auto_virtualize_threshold_mbps`.
- The rate used for this decision is the recompiled runtime-effective rate, not an earlier raw attachment max.

When a node becomes static virtual:

- it stays visible logically and in `tree.html`
- it keeps monitoring, throughput, RTT, and roll-up context
- it does not consume a physical HTB class
- its children are shaped through the nearest non-virtual queue-visible path

#### IP range allow/ignore behavior for integrations

The `[ip_ranges]` section is also used when integrations generate subscriber/device shaping data.

- `allow_subnets` defines the address space LibreQoS should consider shapeable.
- `ignore_subnets` removes matching addresses from generated subscriber devices even if they are otherwise present in the source CRM/NMS.
- Current shared integration-output pruning uses `ignore_subnets` to exclude generated subscriber/device rows, but does not newly require all imported integration IPs to be inside `allow_subnets` just to survive generation.
- If an imported device is left with no remaining non-ignored IPs after `ignore_subnets` is applied, that device is omitted from generated `ShapedDevices.csv`.
- If an imported circuit is left with no remaining shaped devices, that circuit is omitted from generated `ShapedDevices.csv`.

This can be used to exclude entire subscriber populations from LibreQoS shaping and Insight-reported shaped-device inventory. For example, some operators shape wireless subscribers in LibreQoS but exclude FTTH subscribers whose ONTs already enforce service rates.

Use this carefully: `ignore_subnets` is broader than a billing-only toggle. The same setting also affects other LibreQoS/Insight IP-policy handling.

#### Dynamic circuits (optional)

LibreQoS can be configured with an optional `[dynamic_circuits]` section. This is intended for the dynamic-circuit overlay layer (including unknown-IP promotion workflows).

Example:

```toml
[dynamic_circuits]
enabled = false
ttl_seconds = 300
enable_unknown_ip_promotion = false

[[dynamic_circuits.ranges]]
name = "Default"
ip_range = "0.0.0.0" # shorthand for 0.0.0.0/0
download_min_mbps = 10.0
upload_min_mbps = 10.0
download_max_mbps = 100.0
upload_max_mbps = 100.0
attach_to = "" # optional network.json node name
```

Notes:
- `ip_range` must be a CIDR. `0.0.0.0` (and `::`) are allowed shorthands for the match-all `/0` networks.
- `attach_to` is a `network.json` node name (optional; empty is allowed).

#### CRM/NMS Integrations

Learn more about [configuring integrations here](integrations.md).

### Runtime overrides (`lqos_overrides.json`)

LibreQoS supports runtime-friendly adjustments via `lqos_overrides.json` in your `lqos_directory`.

```{mermaid}
flowchart LR
    A[CRM/NMS or manual files] --> B[Base network.json + ShapedDevices.csv]
    C[lqos_overrides API/CLI] --> D[lqos_overrides.json]
    B --> E[lqos_scheduler refresh]
    D --> E
    E --> F[Merged shaping plan]
    F --> G[lqosd active queues/classes]
```

Use the `lqos_overrides` CLI:

```bash
/opt/libreqos/src/bin/lqos_overrides --help
```

Common examples:

```bash
# List persistent devices
/opt/libreqos/src/bin/lqos_overrides persistent-devices list

# Add/replace per-circuit speed adjustment
/opt/libreqos/src/bin/lqos_overrides adjustments add-circuit-speed --circuit-id "1234" --max-download-bandwidth 200 --max-upload-bandwidth 50

# Set a node to logical-only (virtual) without editing network.json directly
/opt/libreqos/src/bin/lqos_overrides network-adjustments set-virtual "AP_GROUP_A" true

# List network adjustments
/opt/libreqos/src/bin/lqos_overrides network-adjustments list
```

How overrides apply:
- `lqos_scheduler` applies overrides during refresh cycles.
- persistent devices are merged into `ShapedDevices.csv`.
- circuit/device/network adjustments are applied on top of imported/manual data.
- operator-owned site bandwidth overrides prefer `node_id` when present and fall back to legacy name-only matching.
- tree-page `Operator Override` writes to the operator override layer in `lqos_overrides.json`, not to legacy integration bandwidth CSV files.
- automated runtime layers such as StormGuard and TreeGuard remain separate from the operator layer and are not written back into operator-authored source files.

### Network Hierarchy
#### Network.json

Network.json allows ISP operators to define a Hierarchical Network Topology, or Flat Network Topology.

Each topology node may optionally include an `"id"` field. This is intended to carry a stable node identifier from the source CRM/NMS when one exists. Current builds prefer this ID when matching operator-owned site bandwidth overrides, while still supporting legacy name-only matching as a fallback.

Recommended format:

```json
{
  "Tower_A": {
    "id": "uisp:site:abc123",
    "downloadBandwidthMbps": 1000,
    "uploadBandwidthMbps": 1000,
    "type": "site"
  }
}
```

Notes:
- Use namespaced string IDs such as `uisp:site:<id>`, `splynx:network_site:<id>`, or `sonar:ap:<id>`.
- Generated LibreQoS-only nodes may use stable generated IDs such as `libreqos:generated:uisp:site:orphans`.
- Existing integration-specific metadata fields such as `uisp_site` and `uisp_device` may also appear alongside the generic `id` field.

#### Queue mode (`shape` / `observe`)

LibreQoS currently uses `queue_mode` in the `[queues]` section to control whether the shaping tree is active:

- `queue_mode = "shape"`: normal shaping mode
- `queue_mode = "observe"`: remove the subscriber shaping tree for a true baseline while keeping the root MQ in place

Switching from `shape` to `observe` cancels any in-flight Bakery live queue-migration verification so the Observe transition does not surface stale queue-tree cleanup as Bakery errors. IP mapping and per-circuit traffic attribution continue to work in Observe mode.

The older `monitor_only` setting is retained as a compatibility alias in some configs and serialized output, but `queue_mode` is the current operator-facing setting and documentation term.

If you plan to use the built-in UISP, Splynx, or Netzur integrations, you do not need to create a network.json file quite yet.
If you plan to use the built-in UISP integration, it will create this automatically on its first run (assuming network.json is not already present).

If you will not be using an integration, you can manually define the network.json following the template file - [network.example.json](https://github.com/LibreQoE/LibreQoS/blob/develop/src/network.example.json). Below is a table illustration of network.example.json. 

<table><thead><tr><th colspan="5">Entire Network</th></tr></thead><tbody><tr><td colspan="3">Site_1</td><td colspan="2">Site_2</td></tr><tr><td>AP_A</td><td colspan="2">Site_3</td><td>Pop_1</td><td>AP_1</td></tr><tr><td></td><td colspan="2">PoP_5</td><td>AP_7</td><td></td></tr><tr><td></td><td>AP_9</td><td>PoP_6</td><td></td><td></td></tr><tr><td></td><td></td><td>AP_11</td><td></td><td></td></tr></tbody></table>

For networks with no Parent Nodes (no strictly defined Access Points or Sites) edit the network.json to use a Flat Network Topology with
```
echo "{}" > network.json
```

##### Virtual (logical-only) nodes

LibreQoS supports **virtual nodes** in `network.json` for organizational grouping and monitoring/aggregation in the WebUI/Insight. Virtual nodes are **not** included in the physical HTB shaping tree (they won’t create HTB classes and won’t enforce bandwidth limits).

```{mermaid}
flowchart TD
    A[Logical tree includes virtual node] --> B[Scheduler build phase]
    B --> C[Promote virtual children to nearest non-virtual ancestor]
    C --> D{Sibling name collision after promotion?}
    D -->|No| E[Physical shaping tree generated]
    D -->|Yes| F[Build error: rename/restructure nodes]
```

To mark a node as virtual, set `"virtual": true` on that node.

Example:

```json
{
  "Region": {
    "downloadBandwidthMbps": 1000,
    "uploadBandwidthMbps": 1000,
    "children": {
      "Town": {
        "virtual": true,
        "downloadBandwidthMbps": 500,
        "uploadBandwidthMbps": 500,
        "children": {
          "AP_A": {
            "downloadBandwidthMbps": 200,
            "uploadBandwidthMbps": 200
          }
        }
      }
    }
  }
}
```

Notes:
- During shaping, virtual nodes are removed and their children are promoted to the nearest non-virtual ancestor (see `queuingStructure.json` for the active physical plan).
- `ShapedDevices.csv` can still use a virtual node as a `Parent Node` for display/grouping; LibreQoS will attach those circuits for shaping to the nearest non-virtual ancestor (top-level virtual nodes will be treated as unparented for shaping).
- Avoid name collisions after promotion (two nodes with the same name ending up at the same level).

#### CPU Considerations

CPU planning should follow the **physical shaping tree** (post-promotion), not the raw logical tree from `network.json`.

```{mermaid}
flowchart LR
    A[Logical topology in network.json<br/>may include virtual nodes] --> B[Promotion for shaping build<br/>remove virtual nodes and promote children]
    B --> C[Physical HTB shaping tree<br/>real nodes only]
    C --> D[CPU/binpacking placement<br/>distribute physical top-level shaped nodes]
```

```{mermaid}
flowchart LR
    subgraph Logical Tree
        L1[Region]
        L2[Town virtual]
        L3[AP_A]
        L4[AP_B]
        L1 --> L2
        L2 --> L3
        L2 --> L4
    end
    subgraph Physical HTB Tree
        P1[Region]
        P2[AP_A]
        P3[AP_B]
        P1 --> P2
        P1 --> P3
    end
```

```{mermaid}
flowchart TD
    WAN[Shaped WAN target 20 Gbps example] --> C1[CPU 1 safe budget ~5 Gbps]
    WAN --> C2[CPU 2 safe budget ~5 Gbps]
    WAN --> C3[CPU 3 safe budget ~5 Gbps]
    WAN --> C4[CPU 4 safe budget ~5 Gbps]
    C1 --> N1[Physical top-level nodes assigned here]
    C2 --> N2[Physical top-level nodes assigned here]
    C3 --> N3[Physical top-level nodes assigned here]
    C4 --> N4[Physical top-level nodes assigned here]
```

Notes:
- Virtual nodes are logical-only and do not create HTB classes.
- CPU placement/binpacking acts on the physical post-promotion tree.
- If promotion creates sibling name collisions, shaping build fails.
- The per-core bandwidth numbers above are planning examples, not hard coded limits.

##### CSV to JSON conversion helper

You can use

```shell
python3 csvToNetworkJSON.py
```

to convert manualNetwork.csv to a network.json file.
manualNetwork.csv can be copied from the template file, manualNetwork.template.csv

Note: The parent node name must match that used for clients in ShapedDevices.csv

### Circuits

LibreQoS shapes individual devices by their IP addresses, which are grouped into "circuits".

A circuit represents an ISP subscriber's internet service, which may have just one associated IP (the subscriber's router may be assigned a single /32 IPv4 for example) or it might have multiple IPs associated (maybe their router has a /29 assigned, or multiple /32s).

LibreQoS knows how to shape these devices, and what Node (AP, Site, etc) they are contained by, with the ShapedDevices.csv file.

#### ShapedDevices.csv

The ShapedDevices.csv file correlates device IP addresses to Circuits (each internet subscriber's unique service).

The base format has 15 columns, with an optional 16th `sqm` column for per-circuit queue overrides:

```
Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment[,sqm]
```

##### Optional `sqm` column

If present, `sqm` overrides queueing for that circuit.

Allowed values:
- Single token: `cake`, `fq_codel`, `none`
- Directional token: `down_sqm/up_sqm` where each side is `cake`, `fq_codel`, `none`, or empty

Examples:
- `cake` (both directions)
- `cake/fq_codel` (download cake, upload fq_codel)
- `fq_codel/` (download fq_codel, upload uses global default)
- `/none` (upload disabled, download uses global default)

If `sqm` is empty/missing, global queue defaults apply.

#### TreeGuard and per-circuit SQM

TreeGuard can dynamically adjust per-circuit SQM (`cake`/`fq_codel`) based on circuit conditions.

Important:
- TreeGuard remains enabled by default for circuit SQM management.
- Runtime link virtualization is disabled by default and is no longer the primary queue-planning mechanism.
- If you want fixed/manual SQM behavior, review TreeGuard circuit settings early in deployment and either narrow its enrollment or disable it explicitly.

See [TreeGuard](treeguard.md).

Here is an example of an entry in the ShapedDevices.csv file:
| Circuit ID | Circuit Name                                        | Device ID | Device Name | Parent Node | Parent Node ID | Anchor Node ID | MAC | IPv4                    | IPv6                 | Download Min Mbps | Upload Min Mbps | Download Max Mbps | Upload Max Mbps | Comment |
|------------|-----------------------------------------------------|-----------|-------------|-------------|----------------|----------------|-----|-------------------------|----------------------|-------------------|-----------------|-------------------|-----------------|---------|
| 1          | 968 Circle St., Gurnee, IL 60031                    | 1         | Device 1    | AP_A        |                | site_1         |     | 100.64.0.1, 100.64.0.14 | fdd7:b724:0:100::/56 | 1                 | 1               | 155               | 20              |         |
| 2          | 31 Marconi Street, Lake In The Hills, IL 60156      | 2         | Device 2    | AP_A        |                | site_2         |     | 100.64.0.2              | fdd7:b724:0:200::/56 | 1                 | 1               | 105               | 18              |         |
| 3          | 255 NW. Newport Ave., Jamestown, NY 14701           | 3         | Device 3    | AP_9        |                | site_3         |     | 100.64.0.3              | fdd7:b724:0:300::/56 | 1                 | 1               | 105               | 18              |         |
| 4          | 8493 Campfire Street, Peabody, MA 01960             | 4         | Device 4    | AP_9        |                | site_4         |     | 100.64.0.4              | fdd7:b724:0:400::/56 | 1                 | 1               | 105               | 18              |         |
| 2794       | 6 Littleton Drive, Ringgold, GA 30736               | 5         | Device 5    | AP_11       |                | site_2794      |     | 100.64.0.5              | fdd7:b724:0:500::/56 | 1                 | 1               | 105               | 18              |         |
| 2794       | 6 Littleton Drive, Ringgold, GA 30736               | 6         | Device 6    | AP_11       |                | site_2794      |     | 100.64.0.6              | fdd7:b724:0:600::/56 | 1                 | 1               | 105               | 18              |         |
| 5          | 93 Oklahoma Ave., Parsippany, NJ 07054              | 7         | Device 7    | AP_1        |                | site_5         |     | 100.64.0.7              | fdd7:b724:0:700::/56 | 1                 | 1               | 155               | 20              |         |
| 6          | 74 Bishop Ave., Bakersfield, CA 93306               | 8         | Device 8    | AP_1        |                | site_6         |     | 100.64.0.8              | fdd7:b724:0:800::/56 | 1                 | 1               | 105               | 18              |         |
| 7          | 9598 Peg Shop Drive, Lutherville Timonium, MD 21093 | 9         | Device 9    | AP_7        |                | site_7         |     | 100.64.0.9              | fdd7:b724:0:900::/56 | 1                 | 1               | 105               | 18              |         |
| 8          | 115 Gartner Rd., Gettysburg, PA 17325               | 10        | Device 10   | AP_7        |                | site_8         |     | 100.64.0.10             | fdd7:b724:0:a00::/56 | 1                 | 1               | 105               | 18              |         |
| 9          | 525 Birchpond St., Romulus, MI 48174                | 11        | Device 11   | Site_1      |                | site_9         |     | 100.64.0.11             | fdd7:b724:0:b00::/56 | 1                 | 1               | 105               | 18              |         |

If you are using one of our CRM integrations, this file will be automatically generated. If you are not using an integration, you can manually edit the file using either the WebUI or by directly editing the ShapedDevices.csv file through the CLI.

Directional SQM examples:

```
2794,"6 Littleton Drive, Ringgold, GA 30736",5,Device 5,AP_11,,site_2794,,100.64.0.5,"fdd7:b724:0:500::/56",1,1,105,18,"",cake/fq_codel
2795,"7 Example Ave",6,Device 6,AP_11,,site_2795,,100.64.0.6,,1,1,105,18,"",/none
```

##### Multiple IPs per Circuit
If you need to list multiple IPv4s in the IPv4 field, or multiple IPv6s in the IPv6 field, add a comma between them. If you are editing with a CSV editor (LibreOffice Calc, Excel), the CSV editor will automatically wrap these comma-seperated items with quotes for you. If you are editing the file manually with a utility like notepad or nano, please add quotes surrounding the comma-seperated entries.

```
2794,"6 Littleton Drive, Ringgold, GA 30736",5,Device 5,AP_11,,site_2794,,100.64.0.5,"fdd7:b724:0:500::/56,fdd7:b724:0:600::/56",1,1,105,18,""
```

##### Manual Editing by WebUI
Navigate to the LibreQoS WebUI (http://a.b.c.d:9123) and select Configuration > Shaped Devices.

##### Manual Editing by CLI

- Modify the ShapedDevices.csv file using your preferred spreadsheet editor (LibreOffice Calc, Excel, etc), following the template file - ShapedDevices.example.csv
- Circuit ID is required. The Circuit ID can be a number or string. This field must NOT include any number symbols (#). Every circuit requires a unique CircuitID - they cannot be reused. Here, circuit essentially refers to a customer's service. If a customer has multiple locations on different parts of your network, use a unique CircuitID for each of those locations.
- At least one IPv4 address or IPv6 address is required for each entry.
- `Anchor Node ID` is the preferred topology key. When present, it should contain the stable node ID for the circuit's own topology node so LibreQoS runtime can derive the effective parent from canonical topology, overrides, and attachment health.
- `Parent Node` and `Parent Node ID` remain supported for compatibility and display/grouping, but new integrations should treat them as legacy fields rather than the primary shaping key.
- The ShapedDevices.csv file allows you to set minimum (guaranteed), and maximum allowed bandwidth per subscriber.
- The Download Min and Upload Min for each Circuit must be 1 Mbps or greater. Generally, these should be set to 1 Mbps by default.
- The Download Max and Upload Max for each Circuit must be 2 Mbps or greater. Generally, these correspond to the customer's speed plan.
- Recommendation: set the min bandwidth to 1/1 and max to 1.15X advertised plan rate:
  - This way, when an AP hits its ceiling, users have any remaining AP capacity fairly distributed between them.
  - By setting the max to 1.15X the speed plan, this makes it more likely that the subscriber will see a satisfactory speed test result, even if there is some small light traffic on their circuit running in the background - such as an HD video stream, software updates, etc.
  - This allows subscribers to utilize up to the maximum rate when AP has the capacity to allow that.

Note regarding SLAs: For customers with SLA contracts that guarantee them a minimum bandwidth, you can set their plan rate as the minimum bandwidth. That way when an AP approaches its ceiling, SLA customers will always see that rate available. Make sure that the combined minimum rates for circuits connected to a parent node do not exceed the rate of the parent node. If that happens, LibreQoS has a fail-safe that will [reduce the minimums to 1/1](https://github.com/LibreQoE/LibreQoS/pull/643) for all affected circuits. 

Once your configuration is complete. You're ready to run the application and start the [systemd services](./components.md#systemd-services)
