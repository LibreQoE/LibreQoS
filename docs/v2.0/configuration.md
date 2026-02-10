# Configure LibreQoS

## Initial Configuration Via Setup Tool (From the .deb installer)
<img width="1605" height="1030" alt="setup_tool" src="https://github.com/user-attachments/assets/5a645da8-c411-4635-9777-a881966981df" />

NOTES: 
- The LibreQoS Setup Tool can only be controlled with a keyboard. Use the arrows to move around the tool, and click ```Enter``` to Select.
- Clicking the ```Q``` key will close the setup tool without saving.
- If you are in the process of using the setup tool and it closes beforehand, in order to launch again the setup tool you will have to run the following commands:
  ```
  sudo apt remove libreqos
  sudo apt install ./{deb_url_v1_5}
  ``` 

The first step is to give a name to your Shaper Box (Node), by default it is named LibreQoS.
Then, you can use the arrow keys on your keyboard to go over the different setup sections, as follows:

### Bridge Mode
<img width="1668" height="397" alt="bridge_mode" src="https://github.com/user-attachments/assets/22bc05cc-f1e5-451a-b4f8-5e75d7b8d64f" />

By default, the Linux Bridge is selected. If you chose the XDP Bridge in previous step, please make the adjustment.

NOTE:  Single Interface mode, as it name implies, its for users that can only use 1 interface, and require special support. For more information regarding Single Interface mode, please vistit our [Zulip Chat].

### Interfaces
<img width="885" height="352" alt="interfaces" src="https://github.com/user-attachments/assets/4afedfe6-65b8-450c-a675-bea25ef4553c" />

Following the recommended setup diagram mentioned above, the "To Internet" interface should be the one facing the Edge Router (and therefore, the Internet) and the "To Network" interface should be the one facing the Core Router.

### Bandwidth
<img width="1089" height="350" alt="bandwidth" src="https://github.com/user-attachments/assets/f68185c3-82dc-4fb5-b78a-d812665533fb" />

In the context of bandwidth hired from your upstream provider, ```To Internet``` means the Upload Bandwidth, and ```To Network``` means the download bandwidth.

### IP Range
<img width="1331" height="481" alt="ip_ranges" src="https://github.com/user-attachments/assets/b846baa7-288e-460c-ab77-ad400384057c" />

In this section, you should specify all the ip ranges utilized by your customers' routers, including ranges for static IPs. By default, we include 4 common IP Ranges, as seen in the image above.

*Tip: In this section, in order to remove an IP range, you have to highlight the IP range with the keyboard arrows, and then click the ```Tab``` key until ```<Remove Selected>``` is highlighted, then click ```Enter```.

### Web Users
<img width="1664" height="528" alt="web_users" src="https://github.com/user-attachments/assets/8db17e0e-cc3d-4d67-9c59-4751bc4d9b0f" />

In this section, you can create users to access the LibreQoS Web Dashboard, with their respective roles. There's 2 different types of roles: ```Admin``` and ```Read Only```.
***
After clicking on save file, you may see the following message on the terminal
```
No VM guests are running outdated hypervisor (qemu) binaries on this host.
N: Download is performed unsandboxed as root as file '/home/libreqos/libreqos_1.5-RC2.202510052233-1_amd64.deb' couldn't be accessed by user '_apt'. - pkgAcquire::Run (13: Permission denied)
```
This error is benign and does not reflect any issues. Please disregard it.

### Next Steps

If the installation is successful, you will be able to access the LibreQoS WebUI at ```http://your_shaper_ip:9123```. Upon first visit, you can specify your preferred username and password.

Next, you will want to configure your [CRM or NMS integration](integrations.md) using the Web Interface Configuration page. If you do not use a supported CRM/NMS, you will need to create a script or process to produce the needed files for LibreQoS to shape traffic - specifically network.json and ShapedDevices.csv. The format of these files are explained in further detail in later sections of this page.

## Configuration Via Web Interface

Most LibreQoS shaper settings can be modified via the Configuration page on the WebUI (http://your_shaper_ip:9123/config_general.html).

### QoO (Quality of Outcome) profiles (`qoo_profiles.json`)

LibreQoS displays **QoO** (Quality of Outcome) as an estimate of “Internet Quality” based on latency and loss, aligned with the IETF IPPM QoO draft:
https://datatracker.ietf.org/doc/draft-ietf-ippm-qoo/

LibreQoS uses a **QoO profile** to define what “good” and “bad” look like for your network. Profiles are configured in `qoo_profiles.json`.

#### Where the file lives

LibreQoS loads profiles from:

`<lqos_directory>/qoo_profiles.json`

This is the same directory where LibreQoS reads/writes `network.json` and `ShapedDevices.csv` (your `lqos_directory` is set in `/etc/lqos.conf`). The WebUI also displays the resolved path on the Configuration → General page.

#### Selecting a profile

You can select the active profile in either place:

- **WebUI**: Configuration → General → “QoO Profile”
- **Config file**: set `qoo_profile_id` in `/etc/lqos.conf` (top-level key)

Example:

```toml
# /etc/lqos.conf
qoo_profile_id = "web_browsing"
```

If `qoo_profile_id` is not set, LibreQoS uses the default profile from `qoo_profiles.json` (and otherwise falls back to `web_browsing` / first available profile).

#### File format (schema v2)

`qoo_profiles.json` is strictly validated. It must contain `schema_version: 2`, an optional `default_profile_id`, and a `profiles` list. Each profile defines:

- `latency`: one or more RTT percentiles with thresholds in **milliseconds**
  - `high_ms` = good/target
  - `low_ms` = bad/unacceptable
- `loss_percent`: thresholds in **percent** (0..100) (LibreQoS uses TCP retransmit fraction as a loss proxy)
  - `high` = good/target
  - `low` = bad/unacceptable

Minimal example (one profile):

```json
{
  "schema_version": 2,
  "default_profile_id": "web_browsing",
  "profiles": [
    {
      "id": "web_browsing",
      "name": "Web browsing",
      "description": "Targets responsive page loads; defines acceptable interactivity.",
      "latency": [{ "percentile": 95, "low_ms": 150.0, "high_ms": 50.0 }],
      "loss_percent": { "low": 1.0, "high": 0.5 },
      "latency_normalization": { "mode": "none" },
      "loss_handling": "confidence_weighted"
    }
  ]
}
```

#### Applying changes

- Changes to `qoo_profiles.json` are picked up automatically (no restart required).
- If you change `/etc/lqos.conf` (including `qoo_profile_id`), restart with `sudo systemctl restart lqosd`.
- If the profile file is missing/invalid, QoO may show as unknown (and errors will appear in `journalctl -u lqosd`).

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

#### CRM/NMS Integrations

Learn more about [configuring integrations here](integrations.md).

### Network Hierarchy
#### Network.json

Network.json allows ISP operators to define a Hierarchical Network Topology, or Flat Network Topology.

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

To mark a node as virtual, set `"virtual": true` on that node. (Legacy compatibility: `"type": "virtual"` is also recognized, but `"virtual": true` is recommended so you can keep a real `"type"` like `"Site"` or `"AP"`.)

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

<img width="3276" height="1944" alt="cpu" src="https://github.com/user-attachments/assets/e4eeed5e-eeeb-4251-9258-d301c3814237" />

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

##### Multiple IPs per Circuit
If you need to list multiple IPv4s in the IPv4 field, or multiple IPv6s in the IPv6 field, add a comma between them. If you are editing with a CSV editor (LibreOffice Calc, Excel), the CSV editor will automatically wrap these comma-seperated items with quotes for you. If you are editing the file manually with a utility like notepad or nano, please add quotes surrounding the comma-seperated entries.

```
2794,"6 Littleton Drive, Ringgold, GA 30736",5,Device 5,AP_11,,100.64.0.5,"fdd7:b724:0:500::/56,fdd7:b724:0:600::/56",1,1,105,18,""
```

##### Manual Editing by WebUI
Navigate to the LibreQoS WebUI (http://a.b.c.d:9123) and select Configuration > Shaped Devices.

##### Manual Editing by CLI

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
