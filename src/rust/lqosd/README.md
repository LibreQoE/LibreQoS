# LQOSD

**The LibreQoS Daemon** is designed to run as a `systemd` service at all times. It provides:

* Load/Unload the XDP/TC programs (they unload when the program exits)
* Configure XDP/TC, based on the content of `ispConfig.py`.
   * Includes support for "on a stick" mode, using `OnAStick = True, StickVlanA = 1, StickVlanB = 2`.
* Hosts a lightweight server offering "bus" queries for clients (such as `lqtop` and `xdp_iphash_to_cpu_cmdline`).
   * See the `lqos_bus` sub-project for bus details.
* Periodically gathers statistics for distribution to other systems via the bus.

## Required Configuration

You *must* have a file present called `/etc/lqos`. At a minimum, it must tell `lqosd` where to find the LibreQoS configuration. For example:

```toml
lqos_directory = '/opt/libreqos/v1.3'
```

## Offload Tuning

`lqosd` can set kernel tunables for you on start-up. These are specified in `/etc/lqos` also, in the `[tuning]` section:

```toml
[tuning]
stop_irq_balance = true
netdev_budget_usecs = 20
netdev_budget_packets = 1
rx_usecs = 0
tx_usecs = 0
disable_rxvlan = true
disable_txvlan = true
disable_offload = [ "gso", "tso", "lro", "sg", "gro" ]
```

> If this section is not present, no tuning will be performed.

## Bifrost - eBPF Kernel Bridge

To enable the kernel-side eBPF bridge, edit `/etc/lqos`:

```toml
[bridge]
use_kernel_bridge = true
interface_mapping = [
	{ name = "eth1", redirect_to = "eth2", scan_vlans = false },
	{ name = "eth2", redirect_to = "eth1", scan_vlans = false }
]
vlan_mapping = []
```

Each interface must be a *physical* interface, not a VLAN. If you set `scan_vlans` to `true`, you can specify mapping rules.

```toml
[bridge]
use_kernel_bridge = true
interface_mapping = [
	{ name = "eth1", redirect_to = "eth1", scan_vlans = true },
]
vlan_mapping = [
	{ parent = "eth1", tag = 3, redirect_to = 4 },
	{ parent = "eth1", tag = 4, redirect_to = 3 },
]
```

Reciprocal mappings are created NOT automatically, you have to specify each mapping. When you are using "on a stick" mode, you need to redirect to the same interface.