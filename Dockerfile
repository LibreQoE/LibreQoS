## Layer 0: Build bpftool

FROM --platform=linux/amd64 quay.io/cilium/image-compilers:5569a29cea6b3ad50aeb03102aaf3dc03841197c@sha256:b15dbedb7c49816c74a765e2f6ecdb9359763b8e4e4328d794f48b9cefae9804 AS builder_bpf

COPY docker/checkout-linux.sh /tmp/checkout-linux.sh
RUN /tmp/checkout-linux.sh

COPY docker/build-bpftool-native.sh /tmp/build-bpftool-native.sh
RUN /tmp/build-bpftool-native.sh

# Layer 1: Build the Rust portions.
# It's a good idea to have run "cargo clean" before building this image. Otherwise,
# it tends to be rather enourmous.

FROM ubuntu:24.10 AS builder_rust
WORKDIR /usr/src/app
RUN apt update
RUN apt install -y python3-pip clang gcc gcc-multilib llvm libelf-dev git nano graphviz curl screen llvm pkg-config linux-tools-common libbpf-dev libssl-dev esbuild mold
RUN curl https://sh.rustup.rs -sSf | bash -s -- -y
RUN echo 'source $HOME/.cargo/env' >> $HOME/.bashrc
ENV PATH="/root/.cargo/bin:${PATH}"
COPY src/. .
COPY ./requirements.txt .
COPY --from=builder_bpf /out/linux/amd64/bin/bpftool /sbin/bpftool
RUN /sbin/bpftool --version
WORKDIR /usr/src/app/rust
RUN cargo build --release --package lqosd
RUN cargo build --release --package lqtop
RUN cargo build --release --package xdp_iphash_to_cpu_cmdline
RUN cargo build --release --package xdp_pping
RUN cargo build --release --package lqusers
RUN cargo build --release --package lqos_map_perf
RUN cargo build --release --package uisp_integration
RUN cargo build --release --package lqos_support_tool
RUN cargo build --release --package lqos_python

# Build the web content
WORKDIR /usr/src/app/rust/lqosd
RUN curl -fsSL https://esbuild.github.io/dl/latest | sh
RUN chmod a+x ./esbuild
RUN ./copy_files.sh

# Layer 2: LibreQoS
FROM ubuntu:24.10
RUN mkdir -p /opt/libreqos/src/bin/static2
RUN mkdir -p /opt/libreqos/src/bin/dashboards
COPY --from=builder_rust /usr/src/app/rust/target/release/lqosd /opt/libreqos/src/bin/lqosd
COPY --from=builder_rust /usr/src/app/rust/target/release/lqos_map_perf /opt/libreqos/src/bin/lqos_map_perf
COPY --from=builder_rust /usr/src/app/rust/target/release/lqos_support_tool /opt/libreqos/src/bin/lqos_support_tool
COPY --from=builder_rust /usr/src/app/rust/target/release/lqtop /opt/libreqos/src/bin/lqtop
COPY --from=builder_rust /usr/src/app/rust/target/release/lqusers /opt/libreqos/src/bin/lqusers
COPY --from=builder_rust /usr/src/app/rust/target/release/xdp_iphash_to_cpu_cmdline /opt/libreqos/src/bin/xdp_iphash_to_cpu_cmdline
COPY --from=builder_rust /usr/src/app/rust/target/release/xdp_pping /opt/libreqos/src/bin/xdp_pping
COPY --from=builder_rust /usr/src/app/rust/target/release/uisp_integration /opt/libreqos/src/bin/uisp_integration
COPY --from=builder_rust /usr/src/app/*.py /opt/libreqos/src/
COPY --from=builder_rust /usr/src/app/bin/static2 /opt/libreqos/src/bin/static2
COPY --from=builder_rust /usr/src/app/requirements.txt /opt/libreqos/src/requirements.txt
COPY --from=builder_rust /usr/src/app/rust/target/release/liblqos_python.so /opt/libreqos/src/liblqos_python.so
RUN apt update
RUN apt install -y python3-pip clang gcc gcc-multilib llvm libelf-dev git nano graphviz curl screen llvm pkg-config linux-tools-common libbpf-dev libssl-dev esbuild mold iproute2
RUN PIP_BREAK_SYSTEM_PACKAGES=1 python3 -m pip install -r /opt/libreqos/src/requirements.txt

CMD ["/opt/libreqos/src/bin/lqosd"]
