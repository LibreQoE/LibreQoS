#!/bin/bash

# Copyright 2017-2022 Authors of Cilium
# SPDX-License-Identifier: Apache-2.0

set -o xtrace
set -o errexit
set -o pipefail
set -o nounset

# bpftool v7.4.0
rev="14bb1e8c8d4ad5d9d2febb7d19c70a3cf536e1e5"

# git clone git://git.kernel.org/pub/scm/linux/kernel/git/bpf/bpf-next.git /src/linux
# cd /src/linux
# git checkout -b "build-${rev:0:7}" "${rev}

# It is much quicker to download a tarball then a full git checkout,
# Google mirror of kernel.org has bpf-next and supports archives per commit.
curl --fail --show-error --silent --location "https://kernel.googlesource.com/pub/scm/linux/kernel/git/bpf/bpf-next/+archive/${rev}.tar.gz" --output /tmp/linux.tgz
mkdir -p /src/linux
tar -xf /tmp/linux.tgz -C /src/linux
rm -f /tmp/linux.tgz

