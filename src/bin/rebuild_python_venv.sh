#!/bin/bash
set -euo pipefail

VENV_DIR="/opt/libreqos/venv"
REQUIREMENTS=""
CONSTRAINTS=""

if [ "$(id -u)" -ne 0 ]; then
  echo "rebuild_python_venv.sh must be run as root." >&2
  exit 1
fi

for candidate in /opt/libreqos/src/requirements.txt /opt/libreqos/requirements.txt; do
  if [ -f "$candidate" ]; then
    REQUIREMENTS="$candidate"
    break
  fi
done

if [ -z "$REQUIREMENTS" ]; then
  echo "LibreQoS Python requirements file not found in /opt/libreqos/src or /opt/libreqos." >&2
  exit 1
fi

for candidate in /opt/libreqos/src/deb-requirements-constraints.txt /opt/libreqos/deb-requirements-constraints.txt; do
  if [ -s "$candidate" ]; then
    CONSTRAINTS="$candidate"
    break
  fi
done

python3 -m venv --clear "$VENV_DIR"
"$VENV_DIR/bin/python" -m pip install --upgrade pip

if [ -n "$CONSTRAINTS" ]; then
  "$VENV_DIR/bin/python" -m pip install --upgrade -c "$CONSTRAINTS" -r "$REQUIREMENTS"
else
  "$VENV_DIR/bin/python" -m pip install --upgrade -r "$REQUIREMENTS"
fi

chown -R root:root "$VENV_DIR"
chmod -R go-w "$VENV_DIR"
