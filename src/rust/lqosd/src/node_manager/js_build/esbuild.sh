#!/bin/bash
scripts=( index.js template.js )
for script in "${scripts[@]}"
do
  echo "Building {$script}"
  esbuild src/"$script" --bundle --minify --sourcemap --target=chrome58,firefox57,safari11,edge16 --outdir=out
done
