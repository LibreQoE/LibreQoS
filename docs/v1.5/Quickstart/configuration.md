# Configure LibreQoS

## Configure lqos.conf

If you installed LibreQoS the complex (Git) installation, you can copy the lqosd daemon configuration file to `/etc`. This is not neccesarry if you installed using the .deb:

```shell
cd /opt/libreqos/src
sudo cp lqos.example /etc/lqos.conf
```

Now edit the file to match your setup with

```shell
sudo nano /etc/lqos.conf
```

In the ```[bridge]``` section, change `to_internet` and `to_network` to match your network interfaces.
- `to_internet = "enp1s0f1"`
- `to_network = "enp1s0f2"`

Then, if using Bifrost/XDP set `use_xdp_bridge = true` under that same `[bridge]` section. If you're not sure whether you need this, we recommend to leave it as `false`.

- Set downlink_bandwidth_mbps and uplink_bandwidth_mbps to match the bandwidth in Mbps of your network's upstream / WAN internet connection. The same can be done for generated_pn_download_mbps and generated_pn_upload_mbps.
- enp1s0f2 would be the interface facing your core router (or bridged internal network if your network is bridged)
- enp1s0f1 would be the interface facing your edge router

Note: If you find that traffic is not being shaped when it should, please make sure to swap the interface order and restart lqosd as well as lqos_scheduler with ```sudo systemctl restart lqosd lqos_scheduler```.

### Integrations

Learn more about [configuring integrations here](../TechnicalDocs/integrations.md).

## Network.json

Network.json allows ISP operators to define a Hierarchical Network Topology, or Flat Network Topology.

For networks with no Parent Nodes (no strictly defined Access Points or Sites) edit the network.json to use a Flat Network Topology with
```nano network.json```
setting the following file content:

```json
{}
```

If you plan to use the built-in UISP or Splynx integrations, you do not need to create a network.json file quite yet.

If you plan to use the built-in UISP integration, it will create this automatically on its first run (assuming network.json is not already present). You can then modify the network.json to more accurately reflect your topology.

If you will not be using an integration, you can manually define the network.json following the template file - network.example.json

```text
+-----------------------------------------------------------------------+
| Entire Network                                                        |
+-----------------------+-----------------------+-----------------------+
| Parent Node A         | Parent Node B         | Parent Node C         |
+-----------------------+-------+-------+-------+-----------------------+
| Parent Node D | Sub 3 | Sub 4 | Sub 5 | Sub 6 | Sub 7 | Parent Node F |
+-------+-------+-------+-------+-------+-------+-------+-------+-------+
| Sub 1 | Sub 2 |       |                       |       | Sub 8 | Sub 9 |
+-------+-------+-------+-----------------------+-------+-------+-------+
```

## Manual Setup

You can use

```shell
python3 csvToNetworkJSON.py
```

to convert manualNetwork.csv to a network.json file.
manualNetwork.csv can be copied from the template file, manualNetwork.template.csv

Note: The parent node name must match that used for clients in ShapedDevices.csv

## ShapedDevices.csv

If you are using an integration, this file will be automatically generated. If you are not using an integration, you can manually edit the file using either the WebUI or by directly editing the ShapedDevices.csv file through the CLI.

### Manual Editing by WebUI
Navigate to the LibreQoS WebUI (http://a.b.c.d:9123) and select Configuration > Shaped Devices.

### Manual Editing by CLI

- Modify the ShapedDevices.csv file using your preferred spreadsheet editor (LibreOffice Calc, Excel, etc), following the template file - ShapedDevices.example.csv
- Circuit ID is required. Must be a string of some sort (int is fine, gets parsed as string). Must NOT include any number symbols (#). Every circuit needs a unique CircuitID - they cannot be reused. Here, circuit essentially means customer location. If a customer has multiple locations on different parts of your network, use a unique CircuitID for each of those locations.
- At least one IPv4 address or IPv6 address is required for each entry.
- The Access Point or Site name should be set in the Parent Node field. Parent Node can be left blank for flat networks.
- The ShapedDevices.csv file allows you to set minimum guaranteed, and maximum allowed bandwidth per subscriber.
- The minimum allowed plan rates for Circuits are 2Mbit. Bandwidth min and max should both be above that threshold.
- Recommendation: set the min bandwidth to something like 25/10 and max to 1.15X advertised plan rate by using bandwidthOverheadFactor = 1.15
  - This way, when an AP hits its ceiling, users have any remaining AP capacity fairly distributed between them.
  - Ensure a reasonable minimum bandwidth minimum for every subscriber, allowing them to utilize up to the maximum provided when AP utilization is below 100%.

Note regarding SLAs: For customers with SLA contracts that guarantee them a minimum bandwidth, set their plan rate as the minimum bandwidth. That way when an AP approaches its ceiling, SLA customers will always get that amount.

![image](https://user-images.githubusercontent.com/22501920/200134960-28709d0f-48fe-4129-b4fd-70b204cade2c.png)

Once your configuration is complete. You're ready to run the application and start the [Deamons](./services-and-run.md)
