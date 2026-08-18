# Performance Tuning

## Ubuntu Starts Slowly (~2 minutes)

### List all services which requires network

```shell
systemctl show -p WantedBy network-online.target
```

### For Ubuntu 22.04 this command can help

```shell
systemctl disable cloud-config iscsid cloud-final
```

### CPU governor on bare metal / hypervisor hosts

LibreQoS now attempts to set the CPU governor to `performance` automatically during startup tuning.

Disable it only if needed with `[tuning].set_cpu_governor_performance = false`, or verify the active governor with:

```shell
cpupower frequency-info | grep 'current policy'
```

### OSPF

It is recommended to set the OSPF timers of both OSPF neighbors (core and edge router) to minimize downtime upon a reboot of the LibreQoS server.

* hello interval
* dead
