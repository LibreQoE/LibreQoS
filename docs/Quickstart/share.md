# Share your before and after

We ask that you please share an anonymized screenshot of your LibreQoS deployment before (monitor only mode) and after (queuing enabled) to the [LibreQoS Chat](https://chat.libreqos.io/join/fvu3cerayyaumo377xwvpev6/). This helps us gauge the impact of our software. It also makes us smile.

1. Enable monitor only mode
2. Klingon mode (Redact customer info)
3. Screenshot
4. Resume regular queuing
5. Screenshot

## Enable monitor only mode

```shell
sudo systemctl stop lqos_scheduler
sudo systemctl restart lqosd
sudo systemctl restart lqos_node_manager
```

## Klingon mode

Please go to the Web UI and click Configuration. Toggle Redact Customer Information (screenshot mode) and then Apply Changes.

## Resume regular queuing

```shell
sudo systemctl start lqos_scheduler
```

## Screenshot

To generate a screenshot - please go to the Web UI and click Configuration. Toggle Redact Customer Information (screenshot mode), Apply Changes, and then return to the dashboard to take a screenshot.
