# Install LibreQoS

## Step 1 - Validate Network Design Assumptions and Hardware Selection

- [Network Design Assumptions](design.md)
- [System Requirements](requirements.md)

## Step 2 - Complete The Installation Prerequisites

- [Server Setup - Prerequisites](prereq.md)
- [Install Ubuntu Server 24.04](ubuntu-server.md)
- [Configure Shaping Bridge](bridge.md)

## Step 3 - Install LibreQoS v1.5 / Upgrade to LibreQoS v1.5

### Use .DEB Package (Recommended Method)

```
cd ~
sudo apt-get update
sudo apt-get upgrade
wget {{deb_url_v1_5}}
sudo apt install ./{{deb_url_v1_5}}
```

### Git Install (For Developers Only - Not Recommended)

[Complex Installation](git-install.md)

## Step 4 - Configure LibreQoS

You are now ready to [Configure](configuration.md) LibreQoS!
