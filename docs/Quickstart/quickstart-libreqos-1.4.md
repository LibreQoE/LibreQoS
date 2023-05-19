# Install LibreQoS 1.4

## Updating from v1.3

### Remove offloadOff.service

```shell
sudo systemctl disable offloadOff.service
sudo rm /usr/local/sbin/offloadOff.sh /etc/systemd/system/offloadOff.service
```

### Remove cron tasks from v1.3

Run ```sudo crontab -e``` and remove any entries pertaining to LibreQoS from v1.3.

## Simple install via .Deb package (Recommended)

Use the deb package from the [latest v1.4 release](https://github.com/LibreQoE/LibreQoS/releases/).

```shell
sudo echo "deb http://stats.libreqos.io/ubuntu jammy main" > /etc/apt/sources.list.d/libreqos.list
wget -O - -q http://stats.libreqos.io/repo.asc | apt-key add -
apt-get update
apt-get install libreqos
```

You will be asked some questions about your configuration, and the management daemon and webserver will automatically start. Go to http://<your_ip>:9123/ to finish installation.

## Complex Install (Not Reccomended)

```{note}
Use this install if you'd like to constantly deploy from the main branch on Github. For experienced users only!
```

[Complex Installation](../TechnicalDocs/complex-install.md)

You are now ready to [Configure](./configuration.md) LibreQoS!
