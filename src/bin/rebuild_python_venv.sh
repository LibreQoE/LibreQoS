#!/bin/bash
set -euo pipefail

VENV_DIR="/opt/libreqos/venv"
WHEELHOUSE="/opt/libreqos/src/wheelhouse"
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
# beta18 (P0, lqos-74t.37.30): no network at install time. The package carries
# a wheelhouse built at package-build time; install from it with --no-index.
# Do NOT `pip install --upgrade pip` — the venv's bundled ensurepip is enough
# and pip would otherwise reach the index.
PIP_ARGS=(--disable-pip-version-check)
if [ -d "$WHEELHOUSE" ] && [ -n "$(ls -A "$WHEELHOUSE" 2>/dev/null)" ]; then
  PIP_ARGS+=(--no-index --find-links "$WHEELHOUSE")
  echo "rebuild_python_venv: installing offline from $WHEELHOUSE"
elif [ "${LQOS_PYTHON_ALLOW_NETWORK:-0}" = "1" ]; then
  echo "rebuild_python_venv: WARNING: no wheelhouse at $WHEELHOUSE; LQOS_PYTHON_ALLOW_NETWORK=1 — falling back to the network" >&2
else
  echo "rebuild_python_venv: ERROR: no wheelhouse at $WHEELHOUSE (a packaged install must be offline); set LQOS_PYTHON_ALLOW_NETWORK=1 only for a development host" >&2
  exit 1
fi

if [ -n "$CONSTRAINTS" ]; then
  "$VENV_DIR/bin/python" -m pip install "${PIP_ARGS[@]}" --upgrade -c "$CONSTRAINTS" -r "$REQUIREMENTS"
else
  "$VENV_DIR/bin/python" -m pip install "${PIP_ARGS[@]}" --upgrade -r "$REQUIREMENTS"
fi

chown -R root:root "$VENV_DIR"
chmod -R go-w "$VENV_DIR"
