# Install LibreQoS 1.5/2.0

## Step 1 - Validate Network Design Assumptions and Hardware Selection

- [Network Design Assumptions](../design.md)
- [System Requirements](../requirements.md)

## Step 2 - Complete The Installation Prerequisites

[LibreQoS Installation Prerequisites](prereq.md)

## Step 3 - Install LibreQoS

### Use .DEB Package (Recommended Method)

```
cd ~
wget https://libreqos.io/wp-content/uploads/2025/03/libreqos_1.5-BETA10_amd64.zip
sudo apt-get install unzip
unzip libreqos_1.5-BETA10_amd64.zip
sudo apt install ./libreqos_1.5-BETA10_amd64.deb
```

### Git Install (For Developers Only - Not Recommended)

[Complex Installation](../git-install.md)

## Step 4 - Configure LibreQoS

You are now ready to [Configure](./configuration.md) LibreQoS!
