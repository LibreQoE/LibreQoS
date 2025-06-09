#!/bin/bash
set -e
scripts=( index.js template.js login.js first-run.js shaped-devices.js tree.js help.js unknown-ips.js configuration.js circuit.js flow_map.js all_tree_sankey.js asn_explorer.js lts_trial.js config_general.js config_anon.js config_tuning.js config_queues.js config_stormguard.js config_lts.js config_iprange.js config_flows.js config_integration.js config_spylnx.js config_uisp.js config_powercode.js config_sonar.js config_interface.js config_network.js config_devices.js config_users.js config_wispgate.js )
for script in "${scripts[@]}"
do
  echo "Building {$script}"
  esbuild src/"$script" --bundle --minify --sourcemap --target=chrome58,firefox57,safari11 --outdir=out || exit
done
