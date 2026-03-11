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
strategy = "delay_probe" # "legacy_score", "delay_probe", or "delay_probe_active"
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
delay_threshold_ms = 40.0
delay_threshold_ratio = 1.10
baseline_alpha_up = 0.01
baseline_alpha_down = 0.10
probe_interval_seconds = 10.0
min_throughput_mbps_for_rtt = 0.05
active_ping_target = "1.1.1.1"
active_ping_interval_seconds = 10.0
active_ping_weight = 0.70
active_ping_timeout_seconds = 1.0
```

| **Entry Name** | **Description**                                                                                           |
|----------------|-----------------------------------------------------------------------------------------------------------|
| `enabled`      | Enable or disable StormGuard. Default: `false`                                                            |
| `dry_run`      | If true, StormGuard will not change or persist the rate. It only logs what it would have done. Default: `true` |
| `log_file`     | If set, a CSV will be appended with time (unix secs), download rate, upload rate entries. Default: absent |
| `strategy`     | `delay_probe` (baseline RTT + probing), `delay_probe_active` (add active ICMP ping RTT), or `legacy_score` (original decision matrix). Default: `delay_probe` |
| `all_sites`    | Monitor all eligible top-level sites. If `false`, only the `targets` allowlist is monitored.            |
| `targets`      | Site allowlist used when `all_sites = false`.                                                             |
| `exclude_sites`| Sites to skip when `all_sites = true`.                                                                    |
| `*_multiplier` | Per-action rate multipliers used when StormGuard increases or decreases a site cap.                       |
| `*_cooldown_seconds` | Per-action cooldowns that suppress repeated adjustments after a change.                            |
| `circuit_fallback_*` | Optional TreeGuard-style per-circuit SQM fallback for circuit queues StormGuard cannot safely change with HTB. |
| `delay_threshold_ms` | Standing delay above baseline RTT that triggers a decrease (`delay_probe`). |
| `delay_threshold_ratio` | RTT ratio above baseline that triggers a decrease (`delay_probe`). |
| `baseline_alpha_*` | Baseline RTT EWMA tuning (`delay_probe`). |
| `probe_interval_seconds` | Minimum time between increase probes (`delay_probe`). |
| `min_throughput_mbps_for_rtt` | Ignore RTT-driven adjustments below this throughput (`delay_probe`). |
| `active_ping_target` | Hostname/IP to ping for RTT sampling (`delay_probe_active`). Default: `1.1.1.1`. |
| `active_ping_interval_seconds` | Time between pings (`delay_probe_active`). Default: `10.0`. |
| `active_ping_weight` | Blend weight (0..=1) of active ping RTT vs passive TCP RTT (`delay_probe_active`). Default: `0.70`. |
| `active_ping_timeout_seconds` | Ping timeout seconds (`delay_probe_active`). Default: `1.0`. |

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

When `strategy = "delay_probe"`, StormGuard instead learns an RTT baseline and treats standing delay (RTT above baseline)
as the primary signal for decreasing rates, with periodic probe-style increases when conditions look good.

When `strategy = "delay_probe_active"`, StormGuard also measures RTT via infrequent ICMP pings to `active_ping_target`
and blends that RTT with passive TCP RTT using `active_ping_weight`. This helps keep the delay signal available on
quiet or low-speed links where passive RTT samples are sparse.

Changes have a "cool-down" following their application, during which monitoring will continue but no changes will be made.
This is to prevent oscillation between two states.

## Running StormGuard

StormGuard is integrated into `lqosd`. If it is enabled, it will run automatically when `lqosd` is started.
