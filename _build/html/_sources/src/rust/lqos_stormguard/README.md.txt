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
targets = [ "CALVIN 1" ]
minimum_download_percentage = 0.5 # For 50% of download as minimum
minimum_upload_percentage = 0.5 # For 50% of upload as minimum
```

| **Entry Name** | **Description**                                                                                           |
|----------------|-----------------------------------------------------------------------------------------------------------|
| `enabled`      | Enable or disable Tornado. Default: `false`                                                               |
| `dry_run`      | If true, Tornado will not change the rate. It will only log what it *would* have done. Default: `false`   |
| `log_file`     | If set, a CSV will be appended with time (unix secs), download rate, upload rate entries. Default: absent |

You can list as many sites as you want in the `targets` array. I strongly recommend `dry_run` for now, which just
emits what it *would* have done to the console!

## How it works

StormGuard maintains a ring-buffer of recent throughput, TCP retransmits and TCP round-trip times for each target site.
These are updated once per second, when `lqosd` "ticks". A second buffer maintains a moving average of a larger time period.

Each circuit also maintains a "current queue bandwidth", which is adjusted dynamically. If `dry_run` is not set,
this is applied directly to the HTB queue associated with the monitoring.

> *Warning*: Do not apply this to HTB circuits that have a directly attached CAKE instance.

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