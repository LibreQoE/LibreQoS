# High Availability and Failure Domains

This page describes a practical active/backup model for LibreQoS.

## Scope and Assumptions

- This page covers LibreQoS high availability as an active/backup design.
- Failover should be controlled by dynamic routing (for example OSPF or BGP).
- Hardware and router vendors are intentionally not prescribed.
- Insight-specific high availability guidance is out of scope for this page.

## Active/Backup HA Model

One path is active (preferred), and one path is backup (standby). Routing policy controls path selection and failover.

```{mermaid}
stateDiagram-v2
    [*] --> PrimaryActive
    PrimaryActive: Primary path preferred (cost 1)
    PrimaryActive --> FailoverConverging: Primary failure detected
    FailoverConverging --> BackupActive: Routing converged to backup (cost 100)
    BackupActive --> RecoveryValidation: Primary repaired
    RecoveryValidation --> PrimaryActive: Preference restored and stable
```

## OSPF Example (Primary Cost 1, Backup Cost 100)

Use OSPF interface cost to prefer the active path.

- Primary LibreQoS path: `ip ospf cost 1`
- Backup LibreQoS path: `ip ospf cost 100`

Conceptual result:

- Normal state: traffic uses the primary path because cost `1` is lower than `100`.
- Failure state: if the primary path/router goes down, OSPF converges and traffic uses the backup path.
- Recovery state: after the primary is healthy again, traffic returns to primary based on lower cost.

Example policy intent:

1. Keep both paths up and routable at all times.
2. Ensure backup has enough capacity for expected peak load.
3. Test failover and failback during maintenance windows.

## BGP Equivalent (If You Use BGP Instead of OSPF)

If you run BGP, use standard preference controls to make one path primary and one backup (for example local preference, MED, or AS path prepending based on your design). Keep policy deterministic and documented.

## Recovery and Failback (Conceptual Runbook)

1. Confirm whether failover occurred (routing table/path checks).
2. Verify customer traffic is flowing on backup path.
3. Repair the failed primary path.
4. Validate primary path health.
5. Return route preference to normal (primary preferred).
6. Verify traffic has returned to primary and performance is stable.

## Planned Maintenance (Conceptual Procedure)

1. Announce window and success criteria.
2. Confirm backup path health and capacity.
3. Shift traffic to backup using routing policy.
4. Perform maintenance on former active path.
5. Validate repaired path.
6. Optionally return traffic to normal active state.
7. Close window with post-change validation notes.

## HA Readiness Checklist

- Dynamic routing is deployed and documented.
- Active/backup preferences are explicit and tested.
- Monitoring and alerting cover both paths and key dependencies.
- On-call runbook includes failover and failback steps.
- Regular drill cadence exists (for example quarterly failover tests).
- Capacity on backup path is validated for realistic peak load.

## Known Limits

- High availability depends on surrounding network design quality.
- Dynamic routing convergence is not zero-time.
- Misconfigured policy can cause asymmetric or unstable failover behavior.
- HA does not replace backups or operational discipline.

## Related Documentation

- [Scale Planning and Topology Design](scale-topology.md)
- [Performance Tuning](performance-tuning.md)
- [StormGuard](stormguard.md)
- [Configuration](configuration.md)
- [Integrations](integrations.md)
- [Troubleshooting](troubleshooting.md)
