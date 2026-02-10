#!/bin/bash
set -e
#ESBUILD_BIN="$(command -v esbuild || true)"
#if [[ -z "$ESBUILD_BIN" ]]; then
  mkdir -p /tmp/esbuild
  pushd /tmp/esbuild
  curl -fsSL https://esbuild.github.io/dl/latest | sh
  popd
  chmod a+x /tmp/esbuild/esbuild
  ESBUILD_BIN="/tmp/esbuild/esbuild"
#fi
scripts=( index.js template.js login.js first-run.js shaped-devices.js tree.js help.js unknown-ips.js configuration.js circuit.js flow_map.js all_tree_sankey.js asn_explorer.js lts_trial.js config_general.js config_tuning.js config_queues.js config_stormguard.js config_lts.js config_iprange.js config_flows.js config_integration.js config_splynx.js config_netzur.js config_uisp.js config_powercode.js config_sonar.js config_interface.js config_network.js config_devices.js config_users.js config_wispgate.js chatbot.js cpu_weights.js cpu_tree.js executive_worst_sites.js executive_oversubscribed_sites.js executive_sites_due_upgrade.js executive_circuits_due_upgrade.js executive_top_asns.js executive_heatmap_rtt.js executive_heatmap_retransmit.js executive_heatmap_download.js executive_heatmap_upload.js)
for script in "${scripts[@]}"
do
  echo "Building {$script}"
  "$ESBUILD_BIN" src/"$script" --bundle --minify --sourcemap --target=chrome58,firefox57,safari11 --outdir=out || exit
done
