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

### Set proper governor for CPU (baremetal/hypervisior host)

```shell
cpupower frequency-set --governor performance
```

### OSPF

It is recommended to set the OSPF timers of both OSPF neighbors (core and edge router) to minimize downtime upon a reboot of the LibreQoS server.

* hello interval
* dead
