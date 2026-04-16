#!/bin/bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "disable_caddy.sh must be run as root." >&2
  exit 1
fi

rm -f /etc/caddy/Caddyfile

if systemctl list-unit-files caddy.service >/dev/null 2>&1; then
  systemctl stop caddy.service || true
  systemctl disable caddy.service || true
fi

echo "Removed the LibreQoS-managed Caddyfile and disabled Caddy."
