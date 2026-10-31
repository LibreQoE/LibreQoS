#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! command -v node >/dev/null 2>&1; then
  echo "Missing required dependency: node (used only for built-in node_manager contract tests, no node_modules required)." >&2
  exit 1
fi

echo "[test-node-manager] Running frontend contract tests..."
node --test \
  "${SCRIPT_DIR}/src/helpers/builders.test.mjs" \
  "${SCRIPT_DIR}/src/circuit_rate_display.test.mjs" \
  "${SCRIPT_DIR}/src/circuit_packet_capture_dom.test.mjs" \
  "${SCRIPT_DIR}/src/circuit_admin_actions.test.mjs" \
  "${SCRIPT_DIR}/src/tree_limit_reason.test.mjs" \
  "${SCRIPT_DIR}/src/tree_stormguard.test.mjs" \
  "${SCRIPT_DIR}/src/tree_detail_tabs.test.mjs" \
  "${SCRIPT_DIR}/src/config/shaped_device_identity.test.mjs" \
  "${SCRIPT_DIR}/src/config/shaped_device_wire.test.mjs" \
  "${SCRIPT_DIR}/src/config_radius_accounting_contract.test.mjs" \
  "${SCRIPT_DIR}/src/scheduler_summary_reason.test.mjs" \
  "${SCRIPT_DIR}/src/scheduler_progress.test.mjs" \
  "${SCRIPT_DIR}/src/config/config_interface_mtu.test.mjs" \
  "${SCRIPT_DIR}/src/config/queues_hygiene.test.mjs" \
  "${SCRIPT_DIR}/src/config/dataplane_hygiene.test.mjs" \
  "${SCRIPT_DIR}/src/config/tuning_hygiene.test.mjs" \
  "${SCRIPT_DIR}/src/config/dpdk_status.test.mjs" \
  "${SCRIPT_DIR}/src/config/local_api_token.test.mjs" \
  "${SCRIPT_DIR}/src/config/network_mode_storage.test.mjs" \
  "${SCRIPT_DIR}/src/config/ssl_redirect.test.mjs" \
  "${SCRIPT_DIR}/src/dashlets/world_map_assets.test.mjs" \
  "${SCRIPT_DIR}/src/dashlets/world_map_dashlet_state.test.mjs" \
  "${SCRIPT_DIR}/src/dashlets/world_map_model.test.mjs" \
  "${SCRIPT_DIR}/src/flow_globe_hexbins.test.mjs" \
  "${SCRIPT_DIR}/src/flow_globe_countries.test.mjs" \
  "${SCRIPT_DIR}/src/graphs/cake_tin_order.test.mjs" \
  "${SCRIPT_DIR}/src/lq_js_common/dashboard/layout_prune.test.mjs" \
  "${SCRIPT_DIR}/src/site_map_bbox.test.mjs" \
  "${SCRIPT_DIR}/src/site_map_config.test.mjs" \
  "${SCRIPT_DIR}/src/site_map_field.test.mjs" \
  "${SCRIPT_DIR}/src/site_map_hexbin.test.mjs" \
  "${SCRIPT_DIR}/src/pubsub/ws.test.mjs" \
  "${SCRIPT_DIR}/src/template_ui_requests_contract.test.mjs" \
  "${SCRIPT_DIR}/src/cpu_workers_model.test.mjs" \
  "${SCRIPT_DIR}/src/executive_heat_labels.test.mjs" \
  "${SCRIPT_DIR}/src/scheduler_system_health_contract.test.mjs"

echo "[test-node-manager] Building bundles..."
"${SCRIPT_DIR}/esbuild.sh"

echo "[test-node-manager] Verifying node_manager build contract..."
"${SCRIPT_DIR}/test-build-contract.sh"
