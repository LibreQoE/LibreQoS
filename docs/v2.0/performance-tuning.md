# Performance Tuning

## Symptom-First Triage

| Symptom | First checks | Likely next action |
|---|---|---|
| Throughput lower than expected | queue/CPU distribution, integration strategy depth, scheduler health | reduce hierarchy depth or use `promote_to_root`, then retest |
| High CPU on one core | IRQ/queue imbalance, root-node bottlenecks | rebalance queue/IRQ affinity and/or promote heavy remote sites to root |
| Performance changed after integration edits | strategy and topology depth changed unintentionally | confirm strategy (`flat`/`ap_only`/`ap_site`/`full`) and validate tree/flow shape |

## CPU and IRQ Baseline

Set CPU frequency governor to performance on bare metal/hypervisor hosts:

```bash
sudo cpupower frequency-set --governor performance
```

Confirm NIC queue count and CPU distribution are sensible for your hardware:

```bash
ethtool -l <interface>
grep -E 'CPU\\(|IRQ' /proc/interrupts | head -n 50
```

If a single queue/CPU is saturated while others are idle, rebalance queue/IRQ affinity and revisit queue-count settings in `/etc/lqos.conf`.

## Topology/Queue Pressure

When shaping large hierarchies:

- Prefer lower-depth integration strategies (`ap_only`/`ap_site`) unless full hierarchy is required.
- Use `promote_to_root` for large multi-site topologies to avoid single-core choke points.
- Validate WebUI CPU Tree/CPU Weights after major topology changes.

## Scheduler Refresh and Load

`lqos_scheduler` refresh cadence affects control-plane load and change responsiveness.

- Start with a conservative `queue_refresh_interval_mins`.
- Decrease interval only if your integration/API and host can handle the extra churn.
- After changing interval, monitor:
  - `journalctl -u lqos_scheduler --since "30 minutes ago"`
  - WebUI scheduler status

## StormGuard Rollout

If using StormGuard:

1. Start with `dry_run = true`.
2. Observe at least one busy period.
3. Move to live mode only after reviewing debug/status behavior.

See [StormGuard](stormguard.md) for details.

## Startup/Boot Delays

If Ubuntu startup is slow due to network-online dependencies, inspect target dependencies:

```bash
systemctl show -p WantedBy network-online.target
```

On some Ubuntu installs, disabling unused cloud/iSCSI services may help:

```bash
sudo systemctl disable cloud-config iscsid cloud-final
```

Validate this against your environment before disabling services.

## Routing Convergence (OSPF)

For routed deployments, tune OSPF neighbor timers on core and edge routers to reduce outage windows during reboots:

- hello interval
- dead interval

## Related Optimization and Resilience Pages

- [Scale Planning and Topology Design](scale-topology.md)
- [StormGuard](stormguard.md)
- [High Availability and Failure Domains](high-availability.md)
- [Troubleshooting](troubleshooting.md)
