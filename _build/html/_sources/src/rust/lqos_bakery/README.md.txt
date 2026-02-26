# LibreQoS Bakery

> The bakery is where the CAKE is made!

The bakery implements TC commands required to build the HTB and CAKE structures. 
It also provides longer-term queue tracking, allowing for the "lazy" creation of queues.

It communicates with the rest of the system via the bus.

## Features

* `LibreQoS.py` no longer directly makes calls to `tc` commands, instead it uses the `Bakery` class (in `lqos_python`) to do so.
* Queue structure is batched, and passed in a single pass to the `lqos_bakery` thread.
* By default, it works exactly as before (with support for fractional bandwidths).

In `lqos.conf`, you can add two items to the `[queues]` section:

```toml
lazy_queues = "No"  # Default: "No"
lazy_expire_seconds = 0 # 0 Disables lazy queue expiration
```

You have the following options:

* `lazy_queues = "Htb"`: All HTB bandwidth-limiting queues will be created when `LibreQoS.py` runs (including by the scheduler). This ensures that you don't have a sudden burst of unshaped traffic. CAKE queues are created lazily, within 1 second of traffic being detected.
* `lazy_queues = "Full"`: All queues are created lazily, within 1 second of traffic being detected.

If you set `lazy_expire_seconds` to a value greater than 0, the bakery will also remove queues that have not been 
used for that many seconds. This is useful for long-term idle queues that you don't want to keep around forever. The 
default expiration time is 600 seconds (10 minutes). If you set it to 0, queues will never be removed.

> It is NOT recommended to set `lazy_expire_seconds` to a very short time-period (under 60 seconds), as this can cause flapping.
