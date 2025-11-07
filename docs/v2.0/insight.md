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
