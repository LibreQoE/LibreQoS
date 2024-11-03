# Share your before and after

We ask that you please share an anonymized screenshot of your LibreQoS deployment before (monitor only mode) and after (queuing enabled) to the [LibreQoS Chat](https://chat.libreqos.io/join/fvu3cerayyaumo377xwvpev6/). This helps us gauge the impact of our software. It also makes us smile.

1. Enable monitor only mode
2. Run for 1 week
3. Disable monitor only mode
4. Toggle "Redact" to hide customer info in the LTS WebUI
5. Screenshot

## Enable monitor only mode

```shell
sudo nano /etc/lqos.conf
```

In the `[queues]` section, set `monitor_only = true` to switch to monitor only mode.

The restart lqosd and lqos_scheduler:
```shell
sudo systemctl restart lqosd lqos_scheduler
```

## Disable monitor only mode

```shell
sudo nano /etc/lqos.conf
```

In the `[queues]` section, set `monitor_only = false` to switch off monitor only mode.

The restart lqosd and lqos_scheduler:
```shell
sudo systemctl restart lqosd lqos_scheduler
```
