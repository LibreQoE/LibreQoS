#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENTRYPOINTS_FILE="${SCRIPT_DIR}/entrypoints.txt"
SRC_DIR="${SCRIPT_DIR}/src"
OUT_DIR="${SCRIPT_DIR}/out"
STATIC_DIR="$(cd "${SCRIPT_DIR}/../static2" && pwd)"
STATIC_PAGES_RS="${SCRIPT_DIR}/../static_pages.rs"

errors=0

fail() {
  echo "[FAIL] $*" >&2
  errors=$((errors + 1))
}

if [[ ! -f "${ENTRYPOINTS_FILE}" ]]; then
  fail "Missing entrypoints file: ${ENTRYPOINTS_FILE}"
fi

if [[ ! -f "${STATIC_PAGES_RS}" ]]; then
  fail "Missing static page router source: ${STATIC_PAGES_RS}"
fi

mapfile -t entrypoints < <(grep -Ev '^\s*(#|$)' "${ENTRYPOINTS_FILE}")
declare -A entrypoint_set=()

for entrypoint in "${entrypoints[@]}"; do
  entrypoint_set["${entrypoint}"]=1
  [[ -f "${SRC_DIR}/${entrypoint}" ]] || fail "Missing JS source entrypoint: ${SRC_DIR}/${entrypoint}"
  [[ -f "${OUT_DIR}/${entrypoint}" ]] || fail "Missing built bundle: ${OUT_DIR}/${entrypoint}"
  [[ -f "${OUT_DIR}/${entrypoint}.map" ]] || fail "Missing sourcemap: ${OUT_DIR}/${entrypoint}.map"
done

for artifact in "${OUT_DIR}"/*.js; do
  [[ -e "${artifact}" ]] || continue
  name="$(basename "${artifact}")"
  [[ -n "${entrypoint_set[${name}]:-}" ]] || fail "Unexpected built JS artifact not listed in entrypoints.txt: ${name}"
done

for artifact in "${OUT_DIR}"/*.js.map; do
  [[ -e "${artifact}" ]] || continue
  name="$(basename "${artifact}" .map)"
  [[ -n "${entrypoint_set[${name}]:-}" ]] || fail "Unexpected sourcemap not listed in entrypoints.txt: $(basename "${artifact}")"
done

mapfile -t served_pages < <(sed -n 's/^[[:space:]]*"\([A-Za-z0-9_.-]\+\.html\)",$/\1/p' "${STATIC_PAGES_RS}")

for page in "${served_pages[@]}"; do
  [[ -f "${STATIC_DIR}/${page}" ]] || fail "Page is served by static_pages.rs but missing from static2: ${page}"
done

required_template_assets=(
  "node_manager.css"
  "vendor/bootstrap.min.css"
  "vendor/bootstrap.bundle.min.js"
  "vendor/jquery-3.7.1.min.js"
  "vendor/echarts.min.js"
  "vendor/echarts-gl.min.js"
  "vendor/echarts_dark.js"
  "vendor/echart_vintage.js"
  "vendor/sortable.min.js"
  "vendor/fontawesome/css/all.css"
)

for asset in "${required_template_assets[@]}"; do
  [[ -f "${STATIC_DIR}/${asset}" ]] || fail "Missing required node_manager asset referenced by template.html: ${asset}"
done

check_cachebusted_bundle_refs() {
  local file_path="$1"
  local label="$2"
  mapfile -t refs < <(sed -n 's#.*<script src="\([A-Za-z0-9_.-]\+\.js\)%CACHEBUSTERS%"></script>#\1#p' "${file_path}")
  for bundle in "${refs[@]}"; do
    [[ -n "${entrypoint_set[${bundle}]:-}" ]] || fail "${label} references bundle not listed in entrypoints.txt: ${bundle}"
    [[ -f "${SRC_DIR}/${bundle}" ]] || fail "${label} references bundle with no source entrypoint: ${bundle}"
  done
}

check_direct_bundle_refs() {
  local file_path="$1"
  local label="$2"
  mapfile -t refs < <(sed -n 's#.*<script src="\([A-Za-z0-9_.-]\+\.js\)"></script>#\1#p' "${file_path}")
  for bundle in "${refs[@]}"; do
    [[ -n "${entrypoint_set[${bundle}]:-}" ]] || fail "${label} references bundle not listed in entrypoints.txt: ${bundle}"
    [[ -f "${SRC_DIR}/${bundle}" ]] || fail "${label} references bundle with no source entrypoint: ${bundle}"
  done
}

check_cachebusted_bundle_refs "${STATIC_DIR}/template.html" "template.html"

for page in "${served_pages[@]}"; do
  check_cachebusted_bundle_refs "${STATIC_DIR}/${page}" "${page}"
done

for standalone_page in "login.html" "first-run.html"; do
  check_direct_bundle_refs "${STATIC_DIR}/${standalone_page}" "${standalone_page}"
done

if (( errors > 0 )); then
  echo "[FAIL] node_manager build contract check failed with ${errors} issue(s)." >&2
  exit 1
fi

echo "[OK] node_manager build contract is consistent."
