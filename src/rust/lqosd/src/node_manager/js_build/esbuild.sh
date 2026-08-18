#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENTRYPOINTS_FILE="${SCRIPT_DIR}/entrypoints.txt"
SRC_DIR="${SCRIPT_DIR}/src"
OUT_DIR="${SCRIPT_DIR}/out"
ESBUILD_VERSION="${ESBUILD_VERSION:-0.25.3}"
ESBUILD_TARGETS="${ESBUILD_TARGETS:-chrome85,firefox78,safari14}"
ESBUILD_INSTALL_DIR="${ESBUILD_INSTALL_DIR:-${SCRIPT_DIR}/../../../../target/esbuild}"
ESBUILD_STAGE_PATTERN="${ESBUILD_INSTALL_DIR}/staging.XXXXXX"

if [[ ! -f "${ENTRYPOINTS_FILE}" ]]; then
  echo "Missing entrypoints file: ${ENTRYPOINTS_FILE}" >&2
  exit 1
fi

MANAGED_ESBUILD=0
REQUESTED_ESBUILD_BIN="${ESBUILD_BIN:-}"
ESBUILD_BIN=""
ESBUILD_BIN_VERSION=""

esbuild_version() {
  local bin="$1"

  if [[ -x "${bin}" ]]; then
    "${bin}" --version 2>/dev/null || true
  fi
}

install_managed_esbuild() {
  local stage_dir=""
  local stage_bin=""
  local stage_installer=""
  local stage_tmp=""
  local stage_version=""

  mkdir -p "${ESBUILD_INSTALL_DIR}"
  stage_dir="$(mktemp -d "${ESBUILD_STAGE_PATTERN}")"
  stage_bin="${stage_dir}/esbuild"
  stage_installer="${stage_dir}/install-esbuild.sh"
  stage_tmp="${stage_dir}/tmp"

  mkdir -p "${stage_tmp}"
  curl -fsSL "https://esbuild.github.io/dl/v${ESBUILD_VERSION}" -o "${stage_installer}"
  pushd "${stage_dir}" >/dev/null
  TMPDIR="${stage_tmp}" sh "${stage_installer}"
  popd >/dev/null
  chmod a+x "${stage_bin}"

  stage_version="$(esbuild_version "${stage_bin}")"
  if [[ "${stage_version}" != "${ESBUILD_VERSION}" ]]; then
    echo "Downloaded esbuild version ${stage_version:-unknown} does not match ${ESBUILD_VERSION}: ${stage_bin}" >&2
    exit 1
  fi

  mv -f "${stage_bin}" "${ESBUILD_BIN}"
  rm -rf "${stage_dir}"
}

if [[ -n "${REQUESTED_ESBUILD_BIN}" ]]; then
  ESBUILD_BIN="${REQUESTED_ESBUILD_BIN}"
  if [[ ! -x "${ESBUILD_BIN}" ]]; then
    echo "Configured ESBUILD_BIN is not executable: ${ESBUILD_BIN}" >&2
    exit 1
  fi
  ESBUILD_BIN_VERSION="$(esbuild_version "${ESBUILD_BIN}")"
else
  SYSTEM_ESBUILD_BIN="$(command -v esbuild || true)"
  if [[ -n "${SYSTEM_ESBUILD_BIN}" ]]; then
    SYSTEM_ESBUILD_VERSION="$(esbuild_version "${SYSTEM_ESBUILD_BIN}")"
    if [[ "${SYSTEM_ESBUILD_VERSION}" == "${ESBUILD_VERSION}" ]]; then
      ESBUILD_BIN="${SYSTEM_ESBUILD_BIN}"
      ESBUILD_BIN_VERSION="${SYSTEM_ESBUILD_VERSION}"
    fi
  fi

  if [[ -z "${ESBUILD_BIN}" ]]; then
    ESBUILD_BIN="${ESBUILD_INSTALL_DIR}/esbuild"
    ESBUILD_BIN_VERSION="$(esbuild_version "${ESBUILD_BIN}")"
    MANAGED_ESBUILD=1
  fi
fi

if [[ "${MANAGED_ESBUILD}" -eq 1 && "${ESBUILD_BIN_VERSION}" != "${ESBUILD_VERSION}" ]]; then
  install_managed_esbuild
  ESBUILD_BIN_VERSION="$(esbuild_version "${ESBUILD_BIN}")"
fi

if [[ ! -x "${ESBUILD_BIN}" ]]; then
  echo "esbuild is not executable: ${ESBUILD_BIN}" >&2
  exit 1
fi

if [[ "${ESBUILD_BIN_VERSION}" != "${ESBUILD_VERSION}" ]]; then
  echo "esbuild version ${ESBUILD_BIN_VERSION:-unknown} does not match ${ESBUILD_VERSION}: ${ESBUILD_BIN}" >&2
  exit 1
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
