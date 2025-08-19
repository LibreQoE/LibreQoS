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
wget https://libreqos.io/wp-content/uploads/2025/08/libreqos_1.5-RC1_amd64.zip
sudo apt-get install unzip
unzip libreqos_1.5-RC1_amd64.zip
sudo apt install ./libreqos_1.5-RC1_amd64.deb
```

### Git Install (For Developers Only - Not Recommended)

[Complex Installation](git-install.md)

## Step 4 - Configure LibreQoS

You are now ready to [Configure](configuration.md) LibreQoS!
