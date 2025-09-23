# Configure LibreQoS

## Main Configuration File
### /etc/lqos.conf

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

### Sandwich Mode Settings

Sandwich mode is an optional compatibility and rate‑limiting layer for Bridge Mode. Enable and configure it in the Web UI under Configuration → Network Mode → Bridge Mode → “Sandwich Bridge (veth pair)”.

When appropriate
- Compatibility with unsupported NICs or special environments (acceptable performance trade‑off for testing).
- Compatibility when using bonded NICs/LACP.
- Enforcing a hard/accurate rate limit in one or both directions (metered bandwidth).

Key options (under `[bridge]`)
- `to_internet` and `to_network`: existing physical shaping interfaces (unchanged).
- `sandwich.Full.with_rate_limiter`: one of `"None"`, `"Download"`, `"Upload"`, or `"Both"`.
- `sandwich.Full.rate_override_mbps_down`: optional integer; overrides the Download limit if set.
- `sandwich.Full.rate_override_mbps_up`: optional integer; overrides the Upload limit if set.
- `sandwich.Full.queue_override`: optional integer; sets veth TX queue count (default is number of CPU cores).
- `sandwich.Full.use_fq_codel`: optional boolean; attach `fq_codel` under the HTB class for better queueing.

Example (TOML)
```
[bridge]
to_internet = "enp1s0f1"
to_network  = "enp1s0f2"

  [bridge.sandwich.Full]
  with_rate_limiter        = "Both"
  rate_override_mbps_down  = 500
  rate_override_mbps_up    = 100
  queue_override           = 8
  use_fq_codel             = true
```

Rate limiting details
- Sandwich rate limiting uses an HTB class for the cap; `fq_codel` (if enabled) is attached as a child qdisc to improve queueing behavior.
- Choose the limiter direction based on the need: Download (ISP→LAN), Upload (LAN→ISP), or Both.

### Netflow (optional)
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

### CRM/NMS Integrations

Learn more about [configuring integrations here](integrations.md).

## Network Hierarchy
### Network.json

Network.json allows ISP operators to define a Hierarchical Network Topology, or Flat Network Topology.

If you plan to use the built-in UISP or Splynx integrations, you do not need to create a network.json file quite yet.
If you plan to use the built-in UISP integration, it will create this automatically on its first run (assuming network.json is not already present).

If you will not be using an integration, you can manually define the network.json following the template file - [network.example.json](https://github.com/LibreQoE/LibreQoS/blob/develop/src/network.example.json). Below is a table illustration of network.example.json. 

<table><thead><tr><th colspan="5">Entire Network</th></tr></thead><tbody><tr><td colspan="3">Site_1</td><td colspan="2">Site_2</td></tr><tr><td>AP_A</td><td colspan="2">Site_3</td><td>Pop_1</td><td>AP_1</td></tr><tr><td></td><td colspan="2">PoP_5</td><td>AP_7</td><td></td></tr><tr><td></td><td>AP_9</td><td>PoP_6</td><td></td><td></td></tr><tr><td></td><td></td><td>AP_11</td><td></td><td></td></tr></tbody></table>

For networks with no Parent Nodes (no strictly defined Access Points or Sites) edit the network.json to use a Flat Network Topology with
```
echo "{}" > network.json
```

#### CSV to JSON conversion helper

You can use

```shell
python3 csvToNetworkJSON.py
```

to convert manualNetwork.csv to a network.json file.
manualNetwork.csv can be copied from the template file, manualNetwork.template.csv

Note: The parent node name must match that used for clients in ShapedDevices.csv

## Circuits

LibreQoS shapes individual devices by their IP addresses, which are grouped into "circuits".

A circuit represents an ISP subscriber's internet service, which may have just one associated IP (the subscriber's router may be assigned a single /32 IPv4 for example) or it might have multiple IPs associated (maybe their router has a /29 assigned, or multiple /32s).

LibreQoS knows how to shape these devices, and what Node (AP, Site, etc) they are contained by, with the ShapedDevices.csv file.

### ShapedDevices.csv

The ShapedDevices.csv file correlates device IP addresses to Circuits (each internet subscriber's unique service).

Here is an example of an entry in the ShapedDevices.csv file:
| Circuit ID | Circuit Name                                        | Device ID | Device Name | Parent Node | MAC | IPv4                    | IPv6                 | Download Min Mbps | Upload Min Mbps | Download Max Mbps | Upload Max Mbps | Comment |
|------------|-----------------------------------------------------|-----------|-------------|-------------|-----|-------------------------|----------------------|-------------------|-----------------|-------------------|-----------------|---------|
| 1          | 968 Circle St., Gurnee, IL 60031                    | 1         | Device 1    | AP_A        |     | 100.64.0.1, 100.64.0.14 | fdd7:b724:0:100::/56 | 1                 | 1               | 155               | 20              |         |
| 2          | 31 Marconi Street, Lake In The Hills, IL 60156      | 2         | Device 2    | AP_A        |     | 100.64.0.2              | fdd7:b724:0:200::/56 | 1                 | 1               | 105               | 18              |         |
| 3          | 255 NW. Newport Ave., Jamestown, NY 14701           | 3         | Device 3    | AP_9        |     | 100.64.0.3              | fdd7:b724:0:300::/56 | 1                 | 1               | 105               | 18              |         |
| 4          | 8493 Campfire Street, Peabody, MA 01960             | 4         | Device 4    | AP_9        |     | 100.64.0.4              | fdd7:b724:0:400::/56 | 1                 | 1               | 105               | 18              |         |
| 2794       | 6 Littleton Drive, Ringgold, GA 30736               | 5         | Device 5    | AP_11       |     | 100.64.0.5              | fdd7:b724:0:500::/56 | 1                 | 1               | 105               | 18              |         |
| 2794       | 6 Littleton Drive, Ringgold, GA 30736               | 6         | Device 6    | AP_11       |     | 100.64.0.6              | fdd7:b724:0:600::/56 | 1                 | 1               | 105               | 18              |         |
| 5          | 93 Oklahoma Ave., Parsippany, NJ 07054              | 7         | Device 7    | AP_1        |     | 100.64.0.7              | fdd7:b724:0:700::/56 | 1                 | 1               | 155               | 20              |         |
| 6          | 74 Bishop Ave., Bakersfield, CA 93306               | 8         | Device 8    | AP_1        |     | 100.64.0.8              | fdd7:b724:0:800::/56 | 1                 | 1               | 105               | 18              |         |
| 7          | 9598 Peg Shop Drive, Lutherville Timonium, MD 21093 | 9         | Device 9    | AP_7        |     | 100.64.0.9              | fdd7:b724:0:900::/56 | 1                 | 1               | 105               | 18              |         |
| 8          | 115 Gartner Rd., Gettysburg, PA 17325               | 10        | Device 10   | AP_7        |     | 100.64.0.10             | fdd7:b724:0:a00::/56 | 1                 | 1               | 105               | 18              |         |
| 9          | 525 Birchpond St., Romulus, MI 48174                | 11        | Device 11   | Site_1      |     | 100.64.0.11             | fdd7:b724:0:b00::/56 | 1                 | 1               | 105               | 18              |         |

If you are using one of our CRM integrations, this file will be automatically generated. If you are not using an integration, you can manually edit the file using either the WebUI or by directly editing the ShapedDevices.csv file through the CLI.

#### Multiple IPs per Circuit
If you need to list multiple IPv4s in the IPv4 field, or multiple IPv6s in the IPv6 field, add a comma between them. If you are editing with a CSV editor (LibreOffice Calc, Excel), the CSV editor will automatically wrap these comma-seperated items with quotes for you. If you are editing the file manually with a utility like notepad or nano, please add quotes surrounding the comma-seperated entries.

```
2794,"6 Littleton Drive, Ringgold, GA 30736",5,Device 5,AP_11,,100.64.0.5,"fdd7:b724:0:500::/56,fdd7:b724:0:600::/56",1,1,105,18,""
```

#### Manual Editing by WebUI
Navigate to the LibreQoS WebUI (http://a.b.c.d:9123) and select Configuration > Shaped Devices.

#### Manual Editing by CLI

- Modify the ShapedDevices.csv file using your preferred spreadsheet editor (LibreOffice Calc, Excel, etc), following the template file - ShapedDevices.example.csv
- Circuit ID is required. The Circuit ID can be a number or string. This field must NOT include any number symbols (#). Every circuit requires a unique CircuitID - they cannot be reused. Here, circuit essentially refers to a customer's service. If a customer has multiple locations on different parts of your network, use a unique CircuitID for each of those locations.
- At least one IPv4 address or IPv6 address is required for each entry.
- The Access Point or Site name should be set in the Parent Node field. Parent Node can be left blank for flat networks.
- The ShapedDevices.csv file allows you to set minimum (guaranteed), and maximum allowed bandwidth per subscriber.
- The Download Min and Upload Min for each Circuit must be 1 Mbps or greater. Generally, these should be set to 1 Mbps by default.
- The Download Max and Upload Max for each Circuit must be 2 Mbps or greater. Generally, these correspond to the customer's speed plan.
- Recommendation: set the min bandwidth to 1/1 and max to 1.15X advertised plan rate:
  - This way, when an AP hits its ceiling, users have any remaining AP capacity fairly distributed between them.
  - By setting the max to 1.15X the speed plan, this makes it more likely that the subscriber will see a satisfactory speed test result, even if there is some small light traffic on their circuit running in the background - such as an HD video stream, software updates, etc.
  - This allows subscribers to utilize up to the maximum rate when AP has the capacity to allow that.

Note regarding SLAs: For customers with SLA contracts that guarantee them a minimum bandwidth, you can set their plan rate as the minimum bandwidth. That way when an AP approaches its ceiling, SLA customers will always see that rate available. Make sure that the combined minimum rates for circuits connected to a parent node do not exceed the rate of the parent node. If that happens, LibreQoS has a fail-safe that will [reduce the minimums to 1/1](https://github.com/LibreQoE/LibreQoS/pull/643) for all affected circuits. 

Once your configuration is complete. You're ready to run the application and start the [systemd services](./components.md#systemd-services)
