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

declare -A directly_handled_html=(
  ["first-run.html"]=1
  ["login.html"]=1
  ["template.html"]=1
)

for html_path in "${STATIC_DIR}"/*.html; do
  [[ -e "${html_path}" ]] || continue
  page="$(basename "${html_path}")"
  [[ -n "${directly_handled_html[${page}]:-}" ]] && continue
  if ! printf '%s\n' "${served_pages[@]}" | grep -qx -- "${page}"; then
    fail "HTML page is present in static2 but not in the authenticated static_pages.rs route list: ${page}"
  fi
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

required_dashboard_map_assets=(
  "vendor/maplibre-gl.css"
  "vendor/maplibre-gl.js"
  "vendor/site_map_coastlines.geojson"
)

for asset in "${required_dashboard_map_assets[@]}"; do
  [[ -f "${STATIC_DIR}/${asset}" ]] || fail "Missing required dashboard world map asset: ${asset}"
done

for asset in "${required_dashboard_map_assets[@]}"; do
  if ! grep -R -F -q "${asset}" "${SRC_DIR}/dashlets/world_map_assets.mjs" "${SRC_DIR}/dashlets/world_map_down.js"; then
    fail "Dashboard world map asset exists but is not referenced by the source bundle: ${asset}"
  fi
done

I18N_CATALOG_DIR="${STATIC_DIR}/i18n"
I18N_OUT_DIR="${OUT_DIR}/i18n"

# Every locale in locales.json needs a source catalog, a built catalog, and the
# same key set as en.json (catalogs are compiled by esbuild.sh, not esbuild).
# The generated switcher list is checked here too: it has to be the exact
# {tag, name, dir} projection of locales.json, because the language selector
# labels each <option> from `name` (a dropped field renders "undefined") and
# because an exact list cannot leak the en-XA pseudo-locale.
if [[ ! -f "${I18N_CATALOG_DIR}/locales.json" ]]; then
  fail "Missing i18n locale list: ${I18N_CATALOG_DIR}/locales.json"
elif ! command -v node >/dev/null 2>&1; then
  fail "Missing required dependency: node (reads the i18n catalogs)"
elif ! I18N_CATALOG_DIR="${I18N_CATALOG_DIR}" I18N_OUT_DIR="${I18N_OUT_DIR}" node -e '
const fs = require("node:fs");
const path = require("node:path");
const catalogDir = process.env.I18N_CATALOG_DIR;
const outDir = process.env.I18N_OUT_DIR;
const problems = [];
const readMessages = (file) => Object.keys(JSON.parse(fs.readFileSync(file, "utf8")).messages).sort();
const locales = JSON.parse(fs.readFileSync(path.join(catalogDir, "locales.json"), "utf8"));

for (const {tag, name, dir} of locales) {
  if (typeof name !== "string" || !name.trim()) {
    problems.push(`locales.json: "${tag}" needs its native name`);
  }
  if (dir !== "ltr" && dir !== "rtl") {
    problems.push(`locales.json: "${tag}" directory must be "ltr" or "rtl"`);
  }
  const source = path.join(catalogDir, `${tag}.json`);
  if (!fs.existsSync(source)) {
    problems.push(`locales.json lists ${tag} but ${source} is missing`);
    continue;
  }
  const catalog = JSON.parse(fs.readFileSync(source, "utf8"));
  if (catalog.locale !== tag) {
    problems.push(`${source} declares locale ${JSON.stringify(catalog.locale)}, not ${tag}`);
  }
  if (!fs.existsSync(path.join(outDir, `${tag}.js`))) {
    problems.push(`missing built catalog ${path.join(outDir, `${tag}.js`)} (run esbuild.sh)`);
  }
  const keys = readMessages(source);
  const english = readMessages(path.join(catalogDir, "en.json"));
  const missing = english.filter((key) => !keys.includes(key));
  const extra = keys.filter((key) => !english.includes(key));
  if (missing.length > 0 || extra.length > 0) {
    problems.push(
      `${tag}.json key set differs from en.json (missing: ${missing.join(", ") || "none"}; extra: ${extra.join(", ") || "none"})`,
    );
  }
}

// The switcher list: same entries, same order, same three fields.
const switcher = path.join(outDir, "locales.js");
const projection = locales.map(({tag, name, dir}) => ({tag, name, dir}));
if (!fs.existsSync(switcher)) {
  problems.push(`missing generated locale list ${switcher} (run esbuild.sh)`);
} else {
  const match = /window\.LQOS_I18N_LOCALES = (\[[^\]]*\]);/.exec(fs.readFileSync(switcher, "utf8"));
  if (!match) {
    problems.push(`${switcher} must assign window.LQOS_I18N_LOCALES`);
  } else {
    const emitted = JSON.parse(match[1]);
    const expected = JSON.stringify(projection);
    if (JSON.stringify(emitted) !== expected) {
      problems.push(`${switcher} must be the locales.json projection ${expected}; found ${JSON.stringify(emitted)}`);
    }
  }
}

for (const problem of problems) process.stderr.write(`[FAIL] ${problem}\n`);
process.exit(problems.length > 0 ? 1 : 0);
'; then
  errors=$((errors + 1))
fi

# The extractor is the coverage gate for the catalogs: a typo'd or dead key has
# to fail here, not only when someone remembers to run it by hand.
if command -v node >/dev/null 2>&1 && ! node "${SCRIPT_DIR}/i18n/extract.mjs" --check; then
  errors=$((errors + 1))
fi

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

# Every served page kind loads the UI catalog from the same placeholder, inside
# <head> so it is in place before any page bundle runs.
for template_file in template.html login.html first-run.html; do
  if ! grep -q '<html lang="%%LANG%%" dir="%%DIR%%"' "${STATIC_DIR}/${template_file}"; then
    fail "${template_file} must render <html lang> from the locale placeholders"
  fi
  if ! sed -n '1,/<\/head>/p' "${STATIC_DIR}/${template_file}" | grep -q '%%I18N_SCRIPT%%'; then
    fail "${template_file} must load the i18n catalog inside <head>"
  fi
done

# The switcher writes the language cookie from JavaScript and the server reads
# it in Rust; neither side can import the other, so a rename on one side would
# silently disable "the cookie wins" with every test green. The Rust constant is
# private, so this pairing check is the only thing tying the two together.
COOKIE_NAME_JS="$(sed -n 's/^const COOKIE_NAME = "\([^"]*\)";$/\1/p' "${SRC_DIR}/i18n/locale.mjs")"
if [[ -z "${COOKIE_NAME_JS}" ]]; then
  fail "js_build/src/i18n/locale.mjs must declare one COOKIE_NAME literal"
elif ! grep -F -q "\"${COOKIE_NAME_JS}\"" "${SCRIPT_DIR}/../locale.rs"; then
  fail "locale.rs must read the language cookie name js_build/src/i18n/locale.mjs writes: ${COOKIE_NAME_JS}"
fi

# The runtime takes its fallback strings from the English catalog, so the shell
# loads en.js ahead of the resolved catalog. With both loaded, a key the served
# catalog has not translated renders English; without en.js it renders the key.
if ! I18N_OUT_DIR="${I18N_OUT_DIR}" I18N_RUNTIME="${SRC_DIR}/i18n/index.js" node --input-type=module -e '
const problems = [];
const {readFileSync} = await import("node:fs");
const {pathToFileURL} = await import("node:url");

globalThis.window = globalThis;
const evaluate = (file) => new Function("window", readFileSync(file, "utf8"))(globalThis);
evaluate(`${process.env.I18N_OUT_DIR}/en.js`);
evaluate(`${process.env.I18N_OUT_DIR}/es.js`);
const runtime = await import(pathToFileURL(process.env.I18N_RUNTIME).href);

const key = "common.nav.tree";
if (globalThis.LQOS_I18N?.locale !== "es") {
  problems.push(`the Spanish catalog did not load (locale ${JSON.stringify(globalThis.LQOS_I18N?.locale)})`);
}
delete globalThis.LQOS_I18N.messages[key];
const withEnglish = runtime.t(key);
if (withEnglish !== "Tree") {
  problems.push(`${key} must fall back to the English text, rendered ${JSON.stringify(withEnglish)}`);
}
delete globalThis.LQOS_I18N_EN;
const withoutEnglish = runtime.t(key);
if (withoutEnglish !== key) {
  problems.push(`the fallback check is blind: without en.js ${key} rendered ${JSON.stringify(withoutEnglish)}`);
}

for (const problem of problems) process.stderr.write(`[FAIL] ${problem}\n`);
process.exit(problems.length > 0 ? 1 : 0);
'; then
  errors=$((errors + 1))
fi

# Undeclared render-helper guard (the cpu_workers.js 'segmentsHtml is not
# defined' class): every `*Html` helper invoked in a template literal must be
# declared/imported in the same source file. The minifier keeps an undefined
# global reference, so this can only be caught at the source level.
REPO_ROOT="$(git -C "${SCRIPT_DIR}" rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "${REPO_ROOT}" ]]; then
  REPO_ROOT="$(cd "${SCRIPT_DIR}/../../../../../../../.." && pwd)"
fi
if ! python3 "${REPO_ROOT}/packaging/tests/check_js_identifiers.py"; then
  errors=$((errors + 1))
fi

if (( errors > 0 )); then
  echo "[FAIL] node_manager build contract check failed with ${errors} issue(s)." >&2
  exit 1
fi

echo "[OK] node_manager build contract is consistent."
