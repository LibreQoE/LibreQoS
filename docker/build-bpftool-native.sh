#!/bin/bash

# Copyright 2017-2020 Authors of Cilium
# SPDX-License-Identifier: Apache-2.0

set -o xtrace
set -o errexit
set -o pipefail
set -o nounset

cd /src/linux/tools/bpf/bpftool

make -j "$(getconf _NPROCESSORS_ONLN)"

strip bpftool

mkdir -p /out/linux/amd64/bin
cp bpftool /out/linux/amd64/bin
