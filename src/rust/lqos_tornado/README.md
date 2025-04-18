# LibreQoS Tornado

**WARNING**: This is extremely experimental. Don't try this on anyone you like.

> The name is a bit of a joke, and will change. I kept thinking people said "autorotate", and decided to name it "Tornado" because it was a tornado of autorotate. I don't know why I thought that, but it stuck.

LibreQoS Tornado. Automatic top-level HTB rate adjustment, based on capacity monitoring.

Heavily inspired by LynxTheCat's Cake AutoRate project. https://github.com/lynxthecat/cake-autorate

## Usage

Add the following to your `lqos.conf`:

```toml
[tornado]
enabled = true
dry_run = true
log_file = "/tmp/tornado.csv" # Optional

[[targets]]
name = "CALVIN 1"
max_mbps = [ 150, 150 ]
min_mbps = [ 100, 100 ]
step_mbps = [ 2, 2 ]

# You can add as many targets as you want
```

| **Entry Name** | **Description**                                                                                           |
|----------------|-----------------------------------------------------------------------------------------------------------|
| `enabled`      | Enable or disable Tornado. Default: `false`                                                               |
| `dry_run`      | If true, Tornado will not change the rate. It will only log what it *would* have done. Default: `false`   |
| `log_file`     | If set, a CSV will be appended with time (unix secs), download rate, upload rate entries. Default: absent |

For the targets:
| **Entry Name** | **Description**                                                                                           |
|----------------|-----------------------------------------------------------------------------------------------------------|
| `name`         | The name of the target. This is used for logging and debugging, and must match network.json.              |
| `max_mbps`     | The maximum bandwidth for the target. This is a list of two values, one for download and one for upload.  |
| `min_mbps`     | The minimum bandwidth for the target. This is a list of two values, one for download and one for upload.  |
| `step_mbps`    | The step size for the target. This is a list of two values, one for download and one for upload.          |

Changes will be made in increments of the `step_mbps` value. For example, if the current bandwidth is 100/100 and the max is 150/150, and the step is 2/2, then the bandwidth will be increased to 102/102, then 104/104, etc.

You can list as many sites as you want in the `targets` array. I strongly recommend `dry_run` for now, which just
emits what it *would* have done to the console!

## How it works

Tornado maintains a ring-buffer of recent throughput, TCP retransmits and TCP round-trip times for each target site.
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

## Running Tornado

Currently `lqos_tornado` is a separate binary. It requires that `lqosd` is running (it'll idle if it isn't), and
it requires root --- to update the HTB queue bandwidths.