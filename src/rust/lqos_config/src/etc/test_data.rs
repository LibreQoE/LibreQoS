#[allow(dead_code)]
pub const OLD_CONFIG: &str = "
# This file *must* be installed in `/etc/lqos.conf`.
# Change the values to match your setup.

# Where is LibreQoS installed?
lqos_directory = '/opt/libreqos/src'
queue_check_period_ms = 1000
packet_capture_time = 10 # Number of seconds to capture packets in an analysis session
node_id = \"aee0eb53606ef621d386ef5fcfa0d72a5ba64fccd36df2997695dfb6d418c64b\"

[usage_stats]
send_anonymous = true
anonymous_server = \"127.0.0.1:9125\"

[tuning]
set_cpu_governor_performance = true
# IRQ balance breaks XDP_Redirect, which we use. Recommended to leave as true.
stop_irq_balance = false
netdev_budget_usecs = 8000
netdev_budget_packets = 300
rx_usecs = 8
tx_usecs = 8
disable_rxvlan = true
disable_txvlan = true
# What offload types should be disabled on the NIC. The defaults are recommended here.
disable_offload = [ \"gso\", \"tso\", \"lro\", \"sg\", \"gro\" ]

# For a two interface setup, use the following - and replace
# \"enp1s0f1\" and \"enp1s0f2\" with your network card names (obtained
# from `ip link`):
[bridge]
interface_mapping = [
        { name = \"veth_toexternal\", redirect_to = \"veth_tointernal\", scan_vlans = false },
        { name = \"veth_tointernal\", redirect_to = \"veth_toexternal\", scan_vlans = false }
]
vlan_mapping = []

# For \"on a stick\" (single interface mode):
# [bridge]
# interface_mapping = [
#         { name = \"enp1s0f1\", redirect_to = \"enp1s0f1\", scan_vlans = true }
# ]
# vlan_mapping = [
#         { parent = \"enp1s0f1\", tag = 3, redirect_to = 4 },
#         { parent = \"enp1s0f1\", tag = 4, redirect_to = 3 }
# ]
";

