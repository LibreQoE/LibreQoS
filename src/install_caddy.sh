#!/bin/bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "install_caddy.sh must be run as root." >&2
  exit 1
fi

export DEBIAN_FRONTEND=noninteractive

if command -v caddy >/dev/null 2>&1; then
  echo "Caddy is already installed."
  exit 0
fi

apt-get update
apt-get install -y debian-keyring debian-archive-keyring apt-transport-https curl gnupg

install -d -m 0755 /usr/share/keyrings
curl -fsSL https://dl.cloudsmith.io/public/caddy/stable/gpg.key \
  | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -fsSL https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt \
  -o /etc/apt/sources.list.d/caddy-stable.list

apt-get update
apt-get install -y caddy

echo "Caddy installation complete."
