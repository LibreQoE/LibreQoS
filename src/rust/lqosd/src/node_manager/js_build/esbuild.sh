#!/bin/bash
set -e
scripts=( index.js template.js login.js first-run.js shaped-devices.js tree.js help.js unknown-ips.js configuration.js circuit.js flow_map.js all_tree_sankey.js asn_explorer.js )
for script in "${scripts[@]}"
do
  echo "Building {$script}"
  esbuild src/"$script" --bundle --minify --sourcemap --target=chrome58,firefox57,safari11 --outdir=out
done
