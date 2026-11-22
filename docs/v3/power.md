# CPU power profile

**Version: LibreQoS v3.** How LibreQoS sets the host CPU's power policy,
and how to change it.

The DPDK workers poll their NIC queues continuously, so the CPU clock directly
limits how many packets each worker can shape. LibreQoS manages the CPU power
policy itself through a power profile, set in `/etc/libreqos/lqos.conf`:

```toml
[power]
profile = "balanced"     # balanced (default) | eco | performance
eco_max_freq_pct = 65    # eco only: cap as a percent of the turbo-off maximum
```

## The profiles

Every profile turns **turbo off**. LibreQoS does not offer a way to turn turbo
back on.

| Profile | What it changes | When to use it |
|---|---|---|
| `balanced` (default) | Turbo off; nothing else. | Almost every host. |
| `eco` | Also the `powersave` governor, the `power` energy preference, and a frequency cap of `eco_max_freq_pct` (default 65 %) of the turbo-off maximum. | Lightly loaded hosts where power or heat matters more than capacity. |
| `performance` | Also the `performance` governor and energy preference, and shallow CPU idle states only. Still capped at the turbo-off maximum. | Hosts running near their packet-rate limit. |

On a platform without a turbo switch, "turbo off" means capping the CPU at its
nominal (non-boost) frequency.

`eco` costs capacity. On the [reference host](requirements.md), at 40 Gbps of
production-like traffic, `eco` (2.9 GHz) lost 16.6 % of download traffic
where `balanced` (4.5 GHz) lost 0.03 %.

## Change the profile

```bash
sudo libreqos-power-profile status              # configured profile and platform
sudo libreqos-power-profile set performance     # write [power] profile and apply it now
sudo libreqos-power-profile apply               # re-apply the profile in lqos.conf
sudo libreqos-power-profile restore             # put back the values found before LibreQoS
```

Add `--dry-run` to see what would change without writing anything. The change
does not interrupt shaping.

The `libreqos-power` service applies the profile at every boot. Removing the
`libreqos` package restores the original values.

## Things to know

- **The original values are recorded once**, in `/var/lib/libreqos/power.orig`,
  the first time a profile is applied. `restore` returns to those values, even
  after several profile changes.
- **Other power tools conflict.** If `tuned`, `cpupower` or
  `power-profiles-daemon` is active, `libreqos-power-profile` refuses to run,
  because those tools rewrite the same settings. Disable them, or pass
  `--force`. After applying, the tool re-reads every setting and warns about
  any that did not stick.
- **v2's `[tuning] set_cpu_governor_performance`** is ignored in v3; `[power]`
  owns the CPU settings. `lqosd` logs a warning if the key is still `true`.
- **In a VM** the guest cannot set the host's CPU policy. The tool prints the
  steps to run on the hypervisor host instead.
