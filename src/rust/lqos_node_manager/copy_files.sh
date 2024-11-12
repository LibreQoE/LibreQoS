#!/bin/bash
set -e
echo "Copying static"
cp -v -R src/static2/* ../../bin/static2/
echo "Done"
pushd src/js_build || exit
./esbuild.sh
popd || exit
cp -R src/js_build/out/* ../../bin/static2/
