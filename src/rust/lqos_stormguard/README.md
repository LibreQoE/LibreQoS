# LibreQoS StormGuard

**WARNING**: This is extremely experimental. Don't try this on anyone you like.

LibreQoS StormGuard. Automatic top-level HTB rate adjustment, based on capacity monitoring.

Heavily inspired by LynxTheCat's Cake AutoRate project. https://github.com/lynxthecat/cake-autorate

## Usage

Add the following to your `lqos.conf`:

```toml
[stormguard]
enabled = true
dry_run = true
log_file = "/tmp/stormguard.csv" # Optional
all_sites = false
targets = [ "CALVIN 1" ]
exclude_sites = []
minimum_download_percentage = 0.5 # For 50% of download as minimum
minimum_upload_percentage = 0.5 # For 50% of upload as minimum
increase_fast_multiplier = 1.30
increase_multiplier = 1.15
decrease_multiplier = 0.95
decrease_fast_multiplier = 0.88
increase_fast_cooldown_seconds = 2.0
increase_cooldown_seconds = 1.0
decrease_cooldown_seconds = 3.75
decrease_fast_cooldown_seconds = 7.5
circuit_fallback_enabled = false
circuit_fallback_persist = true
circuit_fallback_sqm = "fq_codel"
```

| **Entry Name** | **Description**                                                                                           |
|----------------|-----------------------------------------------------------------------------------------------------------|
| `enabled`      | Enable or disable StormGuard. Default: `false`                                                            |
| `dry_run`      | If true, StormGuard will not change or persist the rate. It only logs what it would have done. Default: `true` |
| `log_file`     | If set, a CSV will be appended with time (unix secs), download rate, upload rate entries. Default: absent |
| `all_sites`    | Monitor all eligible top-level sites. If `false`, only the `targets` allowlist is monitored.            |
| `targets`      | Site allowlist used when `all_sites = false`.                                                             |
| `exclude_sites`| Sites to skip when `all_sites = true`.                                                                    |
| `*_multiplier` | Per-action rate multipliers used when StormGuard increases or decreases a site cap.                       |
| `*_cooldown_seconds` | Per-action cooldowns that suppress repeated adjustments after a change.                            |
| `circuit_fallback_*` | Optional TreeGuard-style per-circuit SQM fallback for circuit queues StormGuard cannot safely change with HTB. |

You can list as many sites as you want in `targets`, or turn on `all_sites` and carve out exceptions with `exclude_sites`.
`dry_run` is the recommended starting point while tuning the thresholds for a network.

## How it works

StormGuard maintains a ring-buffer of recent throughput, TCP retransmits and TCP round-trip times for each target site.
These are updated once per second, when `lqosd` "ticks". A second buffer maintains a moving average of a larger time period.

Each site also maintains a current queue bandwidth, which is adjusted dynamically. In live mode, StormGuard persists
those adaptive site caps into its own override layer so scheduler rebuilds preserve the current StormGuard state.

> *Warning*: StormGuard will not directly change HTB on qdisc-hosting circuit queues. When circuit fallback is enabled,
> it can request a per-circuit SQM override instead.

Periodically:

* Saturation is calculated as current throughput / max throughput.
* Live saturation is calculated as current throughput / current queue bandwidth.
* Retransmits are set to either High, RisingFast, Rising, Stable, Falling, FallingFast.
* RTT is set to either Rising, Stable or Falling.

These are fed through a decision matrix to determine if the queue bandwidth should be increased or decreased.

Changes have a "cool-down" following their application, during which monitoring will continue but no changes will be made.
This is to prevent oscillation between two states.

## Running StormGuard

StormGuard is integrated into `lqosd`. If it is enabled, it will run automatically when `lqosd` is started.
