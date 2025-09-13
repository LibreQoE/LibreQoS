#!/usr/bin/env bash
set -euo pipefail

FILE=/etc/netplan/libreqos.yaml
BACKUP="/etc/netplan/libreqos.yaml.$(date +%Y%m%d%H%M%S).bak"

if [[ $EUID -ne 0 ]]; then
  echo "Please run as root (sudo)." >&2
  exit 1
fi

if [ ! -f "$FILE" ]; then
  echo "ERROR: $FILE not found" >&2
  exit 1
fi

cp -a "$FILE" "$BACKUP"

# Determine indentation used for the 'version:' line under 'network:' (fallback to two spaces)
indent=$(sed -n 's/^\([[:space:]]*\)version:[[:space:]]*[0-9].*/\1/p' "$FILE" | head -n1)
[[ -n "$indent" ]] || indent="  "

# Remove any existing renderer lines to avoid duplicates
sed -i '/^[[:space:]]*renderer:[[:space:]]*/d' "$FILE"

# Ensure file ends with a newline before appending
if [ -s "$FILE" ] && [ "$(tail -c1 "$FILE" | wc -l)" -eq 0 ]; then
  echo >> "$FILE"
fi

printf "%srenderer: networkd\n" "$indent" >> "$FILE"

chmod 600 "$FILE"

echo "Updated $FILE (backup: $BACKUP)"
echo "Next: run 'sudo netplan apply' during a maintenance window."

