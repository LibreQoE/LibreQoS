# LibreQoS Insight (Insight)

## About Insight
Learn more about Insight on our website, [here](https://libreqos.io/insight/).

## Insight UI

### Taskbar

<img width="355" height="871" alt="taskbar" src="https://github.com/user-attachments/assets/796cba7b-49d0-4a49-96a5-cd12823a6bd8" />

If you have more than one LibreQoS shaper box, you can select the shaper box you want to view using the Shaper Nodes dropdown box (located just below the search box).

### Time Selector

<img width="529" height="83" alt="image" src="https://github.com/user-attachments/assets/b706a230-883a-4064-9436-2e82749eb8b7" />

By clicking the time selector (Last 24 Hours by default) you can set the time range for data display anywhere from 15 minutes to 28 days. You can also specify custom time periods (Now minus X minutes) or highly-specific detailed time periods.

### Dashboard

You can see which shaper box "perspective" is in use by the listed shpaer box name at the top-left corner next to "Select a destination".

<img width="3828" height="2160" alt="01 dashboard" src="https://github.com/user-attachments/assets/29ee98e3-55c7-4466-a444-de9542cc0940" />

The dashboard for LibreQoS Insight is widget-based, and has multiple tabs - each of which can be edited.

#### Traffic Overview Tab

The default tab is the Traffic Overview tab - which displays both Live traffic levels as well as top-traffic endpoints - both by ASN and by Node.

#### Shaper Tab

<img width="3840" height="2160" alt="02 shaper tab" src="https://github.com/user-attachments/assets/721fd195-35ad-421b-8d0a-e2aa6e5cf7e9" />

The Shaper tab shows high-level stats for a particular LibreQoS shaper box. 

- Active Circuit Count: The number of active customer circuits observed - based on the subscriber's traffic movement.
- Throughput: Shaper box throuhgput
- Shaper Packets: Packet-per-second rate of the Shaper box over time.
- Shaper TCP Retransmits Percentage: The percentage of TCP packets observed that were retransmitted. Consider this a proxy for packet loss. This value should typically remain below 1 % across the network.
- Shaper CAKE Activity: How active the CAKE shapers are on the network over time.
- Shaper Round Trip Time: The Round Trip Time average of traffic moving across the network.
- Shaper Round Trip Time Histogram: The Round Trip Time average of traffic moving across the network in histogram format.
- Shaper CPU Utilization: The peak and average CPU utilization of the shaper box over time.
- Shaper Memory Utilization: The RAM utilization of the shaper box over time.

#### Children Tab

<img width="3840" height="2160" alt="03 child view" src="https://github.com/user-attachments/assets/1f0a0e3c-672b-4982-b334-60a681248b99" />

- Shaper Child Throughput: The throughput of each top-level child node on the shaper box.
- Shaper Child TCP Retransmits: This can be used to spot outliers top-level nodes with regard to Retransmits (packet loss).
- Shaper Child Round Trip Time: This can be used to spot outliers top-level nodes with regard to Round Trip Time.

#### Heatmaps

<img width="3840" height="2160" alt="04 heatmap" src="https://github.com/user-attachments/assets/f3911ca3-8157-43dc-9f57-1402a6cd0204" />

This view shows the Round Trip Time, Retransmit, and Capacity heatmaps for the top-level nodes on this shaper box.

#### Site Rankings

<img width="3840" height="2160" alt="05 health" src="https://github.com/user-attachments/assets/37e69c41-646c-458f-912d-8556acace102" />

This tab shows the health of Sites / Access Points / etc across the network in terms of their Round Trip Time, TCP Retransmits, and Capacity in each direction.

### Node Map

<img width="3840" height="2160" alt="06 map" src="https://github.com/user-attachments/assets/219d75b3-739a-4f41-86e6-5bc270f22afd" />

This view allows you to identify the general topology of the network from LibreQoS Insight's perspective. If you hover over the bands between nodes, it will display the current throughput and TCP retransmits for that leg of the network.

### Libby (AI Assistant)
<img width="1784" height="882" alt="07 libby" src="https://github.com/user-attachments/assets/591a3fd1-3946-44ed-a4fd-e1b1d84b9ef6" />

Libby is LibreQoS Insight's AI Chat Assistant. Libby can help with questions you may have about using LibreQoS or LibreQoS Insight. Libby leverages the official LibreQoS documentation to help yu with any questions you may have. Libby can access both live and long-term data - both from synced LibreQoS shaper boxes and from Insight itself. Libby can reply to queries in most major languages with automatic translation.

You may find new ways to use Libby that we had not originally considered. Please feel welcome to share with us ways you have found Libby to help your workflow.

### Site Heatmap
<img width="3830" height="2160" alt="08 heatmap" src="https://github.com/user-attachments/assets/dfaea245-3221-4cea-874b-fd795ac8da33" />

This view provides RTT, Retransmit, and Capacity heatmaps for every Access Point, OLT, and Site across your network in one view. This allows you to quickly spot trouble points on your network.

### Tree History

<img width="3840" height="2160" alt="09 tree history" src="https://github.com/user-attachments/assets/ea0d0417-c937-41ee-ac4d-7d84c162c6dd" />

The Tree History is based on the Tree Overview display from the LibreQoS WebUI, and displays the sankey over time to help quickly identify network bottlenecks impacting performance.

### Reports
<img width="3828" height="2160" alt="10 report" src="https://github.com/user-attachments/assets/37f6bd91-8937-4755-a095-6bc38822f544" />

LibreQoS Insight enables you to trigger AI-generated reports on specific subscriber circuits. These reports draw in from the customer's last 7 days of network activity - including per-ASN performance characteristics, network topology context, and geographic context. The report identifies the User Profile, Key Findings, Critical Issues, Performance Trends, an Upgrade Recommendation, and suggestions for items to manually review.

### Alerts
<img width="3831" height="2160" alt="11 alerts" src="https://github.com/user-attachments/assets/66f0a465-eb00-4bfb-9cf8-c8302af78ead" />

The Alerts section provides you with automated warnings of out-of-norm performance for nodes across the network (Access Points, OLTs, Sites).
