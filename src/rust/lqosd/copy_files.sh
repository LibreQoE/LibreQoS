#!/bin/bash
cp -R src/node_manager/static2/* ../../bin/static2
pushd src/node_manager/js_build || exit
./esbuild.sh
popd || exit
cp -R src/node_manager/js_build/out/* ../../bin/static2
