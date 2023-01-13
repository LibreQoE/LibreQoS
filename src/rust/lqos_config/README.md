# LQosConfig

`lqos_config` is designed to manage configuration of LibreQoS.

Since all of the parts of the system need to know where to find LibreQoS, it first looks for a file named `/etc/lqos` and uses that to locate the LibreQoS installation.

`/etc/lqos` looks like this:

```toml
lqos_directory = '/opt/libreqos'
```

The entries are:

* `lqos_directory`: where LibreQoS is installed (e.g. `/opt/libreqos`)
