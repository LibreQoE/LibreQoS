# Operating Modes and Source of Truth

## Page Purpose

Use this page to decide where permanent shaping changes should live before you go to production.

Need definitions for terms on this page? See the [Glossary](glossary.md).

```{mermaid}
flowchart TD
    A[Choose Where Changes Live] --> B{Using built-in CRM/NMS integration?}
    B -->|Yes| C[Built-in Integrations Mode]
    B -->|No| D{Generating files from your own scripts/systems?}
    D -->|Yes| E[Custom Source of Truth Mode]
    D -->|No| F[Manual Files Mode]
    C --> G[Integration sync keeps shaping data current]
    E --> H[Your scripts refresh the shaping files]
    F --> I[You maintain the files directly]
```

LibreQoS supports three common operating modes.

## Built-in Integrations (Recommended for Most Operators)

In this mode, your CRM/NMS is where permanent subscriber and topology changes belong, and LibreQoS keeps itself in sync from that system.

Key behavior:
- Integration sync refreshes the topology and shaping data LibreQoS uses.
- Built-in integrations do not use `network.json` as their normal working file.
- Direct file edits may be overwritten on the next scheduler refresh.
- `flat` mode intentionally simplifies the tree when you want lower overhead.

## Custom Source of Truth

In this mode, your own scripts or systems generate `network.json` and `ShapedDevices.csv`.

Key behavior:
- Your external workflow is where permanent changes belong.
- WebUI edits are fine for quick operational changes.
- Keep your long-term changes in your own scripts or automation.

## Manual Files Mode

In this mode, you maintain `network.json` and `ShapedDevices.csv` yourself.

Key behavior:
- Best fit for small networks, short pilots, or temporary workarounds.
- WebUI helps you validate what LibreQoS is using.
- Manual discipline matters because there is no upstream system keeping those files in sync for you.

## Mode Declaration Checklist (Before Go-Live)

1. Pick one primary place for permanent shaping changes.
2. Confirm which system writes production shaping data.
3. Confirm scheduler refresh behavior and overwrite cadence.
4. Document your hotfix workflow (WebUI, external editor, or both).
5. Do not maintain competing edits in multiple systems for the same objects.

## Topology and Mode Expectations

- Single-interface (on-a-stick) and VLAN-heavy designs are valid, but require explicit queue/interface planning and careful validation after changes.
- Integration mode is best when you want CRM/NMS-driven topology and subscriber lifecycle data to control shaping.
- If you need topology behavior that your integration cannot represent, use custom source of truth mode and keep the responsibility clear.

See:
- [Advanced Configuration Reference](configuration-advanced.md)
- [Troubleshooting](troubleshooting.md)

If you are running built-in integrations, continue to [CRM/NMS Integrations](integrations.md).

## Related Pages

- [Configure LibreQoS](configuration.md)
- [CRM/NMS Integrations](integrations.md)
- [Advanced Configuration Reference](configuration-advanced.md)
