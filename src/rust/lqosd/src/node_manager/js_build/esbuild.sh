#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENTRYPOINTS_FILE="${SCRIPT_DIR}/entrypoints.txt"
SRC_DIR="${SCRIPT_DIR}/src"
OUT_DIR="${SCRIPT_DIR}/out"
ESBUILD_VERSION="${ESBUILD_VERSION:-0.25.3}"
ESBUILD_TARGETS="${ESBUILD_TARGETS:-chrome85,firefox78,safari14}"

if [[ ! -f "${ENTRYPOINTS_FILE}" ]]; then
  echo "Missing entrypoints file: ${ENTRYPOINTS_FILE}" >&2
  exit 1
fi

if [[ -z "${ESBUILD_BIN:-}" ]]; then
  ESBUILD_BIN="$(command -v esbuild || true)"
fi

if [[ -z "${ESBUILD_BIN}" ]]; then
  mkdir -p /tmp/esbuild
  pushd /tmp/esbuild >/dev/null
  curl -fsSL "https://esbuild.github.io/dl/v${ESBUILD_VERSION}" | sh
  popd >/dev/null
  chmod a+x /tmp/esbuild/esbuild
  ESBUILD_BIN="/tmp/esbuild/esbuild"
fi

mkdir -p "${OUT_DIR}"
find "${OUT_DIR}" -maxdepth 1 -type f \( -name '*.js' -o -name '*.js.map' \) -delete

mapfile -t scripts < <(grep -Ev '^\s*(#|$)' "${ENTRYPOINTS_FILE}")

for script in "${scripts[@]}"; do
  if [[ ! -f "${SRC_DIR}/${script}" ]]; then
    echo "Missing source entrypoint: ${SRC_DIR}/${script}" >&2
    exit 1
  fi

  echo "Building ${script}"
  "${ESBUILD_BIN}" "${SRC_DIR}/${script}" --bundle --minify --sourcemap --target="${ESBUILD_TARGETS}" --outdir="${OUT_DIR}"
done
