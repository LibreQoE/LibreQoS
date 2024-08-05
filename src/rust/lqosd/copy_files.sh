#!/bin/bash
set -e
echo "Copying static"
DSTDIR=$1
cp -v -R src/node_manager/static2/* ${DSTDIR}
echo "Done"
pushd src/node_manager/js_build || exit
./esbuild.sh
popd || exit
cp -R src/node_manager/js_build/out/* ${DSTDIR}
