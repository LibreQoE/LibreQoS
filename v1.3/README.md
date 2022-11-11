# v1.3 (IPv4 + IPv6) (Beta)

![Screenshot from 2022-11-04 17-06-31](https://user-images.githubusercontent.com/22501920/200087282-dae1f329-08c1-4b63-90b2-de53cecf9429.png)

## Features

### Fast TCP Latency Tracking

[@thebracket](https://github.com/thebracket/) has created [cpumap-pping](https://github.com/thebracket/cpumap-pping) which merges the functionality of the [xdp-cpumap-tc](https://github.com/xdp-project/xdp-cpumap-tc) and [ePPing](https://github.com/xdp-project/bpf-examples/tree/master/pping) projects, while keeping CPU use within ~1% of xdp-cpumap-tc.

### Integrations

- Added Splynx integration
- UISP integration overhaul by [@thebracket](https://github.com/thebracket/)
- [LMS integation](https://github.com/interduo/LMSLibreQoS) for Polish ISPs by [@interduo](https://github.com/interduo)


### Partial Queue Reload

In v1.2 and prior, the the entire queue structure had to be reloaded to make any changes. This led to a few milliseconds of packet loss for some clients each time that reload happened. The scheduled.py was set to reload all queues each morning at 4AM to avoid any potential disruptions that could theoretically cause.

Starting with v1.3 - LibreQoS tracks the state of the queues, and can do incremental changes without a full reload of all queues. Every 30 minutes - scheduler.py runs the CRM import, and runs a partial reload affecting just the queues that have changed. It still runs a full reload at 4AM.

### v1.3 Improvements to help scale

#### HTB major:minor handle

HTB uses a hex handle for classes. It is two 16-bit hex values joined by a colon - major:minor (<u16>:<u16>). In LibreQoS, each CPU core uses a different major handle.

In v1.2 and prior, the minor handle was unique across all CPUs, meaning only 30k subscribers could be added total.

Starting with LibreQoS v1.3 - minor handles are counted independently by CPU core. With this change, the maximum possible subscriber qdiscs/classes goes from a hard limit of 30k to instead be 30k x CPU core count. So for a higher end system with a 64 core processor such as the AMD EPYC™ 7713P, that would mean ~1.9 million possible subscriber classes. Of course CPU use will be the bottleneck well before class handles are in that scenario. But at least we have that arbitrary 30k limit out of the way.

#### "Circuit ID" Unique Identifier

In order to improve queue reload time in v1.3, it was necessary to use a unique identifier for each circuit. We went with Circuit ID. It can be a number or string, it just needs to be unique between circuits, and the same for multiple devices in the same circuit. This allows us to avoid costly lookups when sorting through the queue structure.

If you have your own script creating ShapedDevices.csv - you could use your CRM's unique identifier for customer services / circuits to serve as this Circuit ID. The UISP and Splynx integrations already do this automatically.
