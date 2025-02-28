# Docker Support

> Docker support is experimental, and hasn't been tested in production yet. We *do not* recommend using it yet!

The Docker setup will eventually let you pull images from LibreQoS and have a setup script to get everything running. We're not there yet!

For now, this will get you going. Note that you absolutely MUST be on a Linux system with eBPF support,
and supported NICs.

## Setup

1. Go into the `docker/cfg` directory and copy `lqos.conf` into there. The interface names are the same. Configure any integrations - only the default ones are supported, currently.
2. Make sure that `ShapedDevices.csv` and `network.json` are as you want them (or in default state - they MUST exist in the `cfg` directory).

## Launch

```bash
cd docker
docker compose up
```

You can now go to `http://localhost:9123` to see the dashboard system. Currently, logins are NOT
persisted - you will be prompted for a default user on every startup. Obviously, that's on the list!
