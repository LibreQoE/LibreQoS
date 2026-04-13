# Topology Data Flow

This page is the canonical reference for how topology and shaping data move through LibreQoS.

It answers 3 questions:

1. Where does topology/shaping data enter the system?
2. Which files are logical source of truth versus runtime outputs?
3. Which UI/runtime components consume each layer?

## High-Level Flow

```{mermaid}
flowchart LR
    subgraph Integration Ingress
        I0[UISP / Splynx / other built-in integrations]
        I1[topology_import.json]
        I0 --> I1
    end

    subgraph DIY Ingress
        D0[Operator / external scripts]
        D1[network.json]
        D2[ShapedDevices.csv]
        D0 --> D1
        D0 --> D2
    end

    subgraph Logical Topology
        L1[topology_canonical_state.json]
        L2[topology_editor_state.json]
        TM[Topology Manager]
    end

    subgraph Runtime Queue Topology
        R0[lqos_topology]
        R1[topology_effective_state.json]
        R2[network.effective.json]
        R3[shaping_inputs.json]
    end

    subgraph Runtime Consumers
        C1[tree.html / Site Map / Sankey]
        C2[lqos_scheduler / Bakery / TC]
    end

    I1 --> L1
    D1 --> L1
    L1 --> L2
    L2 --> TM
    L1 --> R0
    L2 --> R0
    D2 --> R0
    R0 --> R1
    R0 --> R2
    R0 --> R3
    R2 --> C1
    R3 --> C2
```

## Logical vs Queue-Visible Topology

LibreQoS now intentionally separates logical topology from queue-visible topology.

- `Topology Manager` works on the logical topology.
- `tree.html`, `Site Map`, `Sankey`, `shaping_inputs.json`, and HTB/Bakery use the queue-visible runtime topology.

That split is what allows:

- roots to remain manageable in Topology Manager without becoming artificial HTB choke points
- static virtual nodes to remain visible for monitoring while staying out of the physical queue tree
- transport/backhaul paths to remain manageable logically while being squashed for queueing

```{mermaid}
flowchart TD
    A[Logical Topology] --> B[topology_canonical_state.json]
    B --> C[topology_editor_state.json]
    C --> D[Topology Manager]

    B --> E[lqos_topology]
    C --> E
    E --> F[Runtime Queue Policy]
    F --> G[Static virtualization / root promotion]
    G --> H[Runtime squashing]
    H --> I[network.effective.json]
    H --> J[shaping_inputs.json]

    I --> K[tree.html / Site Map / Sankey]
    J --> L[Bakery / TC / Scheduler]
```

## Integration vs DIY Modes

### Integration Mode

Built-in integrations own the imported topology and shaping facts.

- `topology_import.json` is the ingress artifact.
- `network.json` is not the source of truth in this mode.
- `Topology Manager` edits logical topology on top of imported data.
- `lqos_topology` compiles runtime outputs from the logical state.

### DIY / Manual Mode

Operator-managed files remain the ingress contract.

- `network.json` is the topology ingress file.
- `ShapedDevices.csv` is the shaping ingress file.
- `lqos_topology` still produces the same runtime outputs:
  - `network.effective.json`
  - `shaping_inputs.json`

```{mermaid}
flowchart LR
    A[Built-in Integration Mode] --> B[topology_import.json]
    B --> C[topology_canonical_state.json]
    C --> D[lqos_topology]
    D --> E[network.effective.json]
    D --> F[shaping_inputs.json]

    G[DIY / Manual Mode] --> H[network.json]
    G --> I[ShapedDevices.csv]
    H --> C
    I --> D
```

## File Roles

| File | Producer | Main consumer | Role | Authoritative in integration mode | Authoritative in DIY mode | Operator editable |
|---|---|---|---|---|---|---|
| `topology_import.json` | built-in integrations | topology compiler / runtime | integration ingress | Yes | No | No |
| `network.json` | operator or external scripts | canonical import fallback / DIY ingress | DIY/manual topology ingress | No | Yes | Yes |
| `ShapedDevices.csv` | operator or external scripts | shaping ingress | DIY/manual shaping ingress | No | Yes | Yes |
| `topology_canonical_state.json` | compiler / runtime prep | `lqos_topology` | logical canonical topology | Internal source of truth | Internal source of truth | No |
| `topology_editor_state.json` | compiler / runtime prep | Topology Manager | logical editable topology | Internal source of truth | Internal source of truth | No |
| `topology_effective_state.json` | `lqos_topology` | runtime diagnostics | resolved logical/effective attachment state | Runtime output | Runtime output | No |
| `network.effective.json` | `lqos_topology` | tree/UI/runtime consumers | queue-visible runtime topology | Yes | Yes | No |
| `shaping_inputs.json` | `lqos_topology` | scheduler / Bakery / TC | shaping-ready runtime input | Yes | Yes | No |
| `lqos_overrides.json` | operator / WebUI / CLI | scheduler / topology runtime | durable operator intent | Yes | Yes | Yes |
| `topology_attachment_health_state.json` | topology probes | Topology Manager / debug pages | runtime attachment health | Runtime state | Runtime state | No |

## Consumer Map

```{mermaid}
flowchart LR
    A[topology_editor_state.json] --> B[Topology Manager]
    C[network.effective.json] --> D[Network Tree Overview]
    C --> E[Site Map]
    C --> F[Tree Overview Sankey]
    G[shaping_inputs.json] --> H[lqos_scheduler]
    H --> I[lqos_bakery]
    I --> J[Linux TC / HTB]
```

## Topology Manager Persistence Flow

Topology Manager edits are overlay intent. They do not replace imported topology facts at the source; they persist as operator intent and are reapplied on top of refreshed topology.

```{mermaid}
flowchart LR
    A[Topology Manager edit] --> B[lqos_overrides.json]
    B --> C[lqos_scheduler refresh]
    C --> D[topology_editor_state.json]
    C --> E[lqos_topology]
    E --> F[network.effective.json]
    E --> G[shaping_inputs.json]
    G --> H[lqos_bakery]
    H --> I[Linux TC / HTB]
```

What this means:

- A saved move or attachment preference survives later integration refreshes.
- Imported topology remains the base data set; operator intent is reapplied on top.
- Bakery and TC are driven from regenerated runtime outputs, not directly from the Topology Manager page.

## Key Rules

- Topology Manager uses logical topology, not the queue tree.
- `network.effective.json` is the queue-visible runtime tree.
- `shaping_inputs.json` is the shaping-ready runtime contract for scheduler/Bakery/TC.
- Runtime consumers should prefer active runtime outputs such as `shaping_inputs.json`; ingress
  files such as `ShapedDevices.csv` and `topology_import.json` are upstream inputs and fallback
  sources when runtime outputs are not yet ready.
- In built-in integration mode, `network.json` and `ShapedDevices.csv` are not the working source of truth.
- Compatibility or legacy tree artifacts are downstream-only in integration mode.
- Static virtual nodes remain visible logically/runtime for monitoring, but they do not consume physical HTB classes.
- Runtime squashing removes queue-useless transport hops from the queue-visible tree while preserving logical manageability elsewhere.
- Topology Manager saves operator intent as overlay state; runtime outputs are regenerated from imported topology plus overrides.

## Related Pages

- [CRM/NMS Integrations](integrations.md)
- [Operating Modes and Source of Truth](operating-modes.md)
- [LibreQoS WebUI (Node Manager)](node-manager-ui.md)
- [Advanced Configuration Reference](configuration-advanced.md)
- [Troubleshooting](troubleshooting.md)
