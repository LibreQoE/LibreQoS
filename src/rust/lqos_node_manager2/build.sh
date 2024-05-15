#!/bin/bash
echo "Running esbuild to construct the site"
pushd site_build || exit
./esbuild.mjs
popd || exit
echo "Copying files"
pushd site_build/output || exit
TARGET="../../static"
TARGETS=( "app.js" "style.css" "app.js.map" "style.css.map" )
for target in "${TARGETS[@]}"
do
  cp $target $TARGET/$target
done
cp ../src/main.html $TARGET/index.html
