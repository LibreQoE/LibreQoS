#!/bin/bash

# Copyright 2017-2020 Authors of Cilium
# SPDX-License-Identifier: Apache-2.0

set -o xtrace
set -o errexit
set -o pipefail
set -o nounset

triplet="aarch64-linux-gnu"

cd /src/linux/tools/bpf/bpftool

make clean

make -j "$(getconf _NPROCESSORS_ONLN)" ARCH=arm64 CROSS_COMPILE=${triplet}-

${triplet}-strip bpftool

mkdir -p /out/linux/arm64/bin
cp bpftool /out/linux/arm64/bin

