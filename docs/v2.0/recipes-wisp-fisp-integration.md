# Recipe: WISP/FISP with Built-In CRM/NMS Integration

Use this pattern when your operator workflow is already centered on a supported CRM/NMS (UISP, Splynx, Netzur, VISP, WISPGate, Powercode, or Sonar).

## Fit

- Best for: recurring subscriber changes, CRM-owned service plans, and ongoing subscriber lifecycle automation.
- Avoid when: your durable source of truth is a custom external pipeline (use custom source of truth mode instead).

## Prerequisites

1. Complete [Quickstart](quickstart.md) and pass the health gate.
2. Confirm integration choice in [CRM/NMS Integrations](integrations.md).
3. Confirm source-of-truth ownership in [Operating Modes and Source of Truth](operating-modes.md).

## Implementation

1. Configure integration credentials/settings in WebUI (`Configuration -> Integrations`).
2. For integration-driven deployments, treat `network.json` as DIY/manual-only and let the integration publish `topology_import.json` instead.
3. Choose the lightest topology strategy that meets requirements.

| Requirement | Suggested strategy |
|---|---|
| Maximum performance, minimal hierarchy | `flat` |
| Moderate hierarchy visibility/control | `ap_only` or `ap_site` |
| Full path/backhaul shaping required | `full` |

4. Enable scheduler-driven recurring refresh in `/etc/lqos.conf` (for example `enable_uisp = true`, `enable_splynx = true`, `enable_netzur = true`; use the matching flag for your selected integration).
5. Restart scheduler and verify sync behavior:

```bash
sudo systemctl restart lqos_scheduler
sudo systemctl status lqos_scheduler
journalctl -u lqos_scheduler --since "15 minutes ago"
```

## Data Flow Illustration

```{mermaid}
flowchart LR
    CRM[CRM/NMS]
    INT[Integration Job]
    SD[ShapedDevices.csv]
    NJ[network.json]
    SCH[lqos_scheduler]
    LQD[lqosd]
    UI[WebUI Status]
    MAN[Manual edits to generated files]

    CRM --> INT
    INT --> SD
    INT --> NJ
    SD --> SCH
    NJ --> SCH
    SCH --> LQD
    SCH --> UI
    MAN -. May be overwritten by next sync .-> SD
    MAN -. May be overwritten by next sync .-> NJ
```

What this shows:

- Integration jobs regenerate shaping inputs consumed by the scheduler.
- In integration mode, direct manual edits to generated files are typically non-durable.

## Validation Checklist

1. `ShapedDevices.csv` is regenerated as expected after sync.
2. `network.json` behavior matches overwrite policy.
3. WebUI views are healthy.
4. Check `Scheduler Status` and `Urgent Issues`.
5. Check `Network Tree Overview` and `Flow Globe`.
6. Parent placement and queue distribution look sane (no unexpected hierarchy collapse).

## Common Failure Modes

- Integration and manual edits fighting each other.
- Unexpected topology depth causing avoidable CPU pressure.
- Missing warnings for unparented circuits in day-1 validation.

Use [Scale Planning and Topology Design](scale-topology.md) and [Troubleshooting](troubleshooting.md) before deeper changes.

## Rollback

1. Revert integration strategy to previous known-good mode.
2. Restore backed up shaping files if needed.
3. Restart `lqos_scheduler` and `lqosd`.
4. Confirm urgent issues clear and views repopulate.

## Related Pages

- [CRM/NMS Integrations](integrations.md)
- [Scale Planning and Topology Design](scale-topology.md)
- [Advanced Configuration Reference](configuration-advanced.md)
- [Troubleshooting](troubleshooting.md)
