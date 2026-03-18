# Package Surface

Use this reference when deciding whether a file is part of the shipped install.

## Source Of Truth

- `src/build_dpkg.sh` defines the `.deb` layout under `/opt/libreqos/src`, `/etc`, and related install hooks.
- `src/build_rust.sh` defines the in-repo build/install layout under `src/bin` and the local `liblqos_python.so` workflow.
- `src/rust/lqosd/copy_files.sh` builds and copies node_manager static assets before they are packaged or used locally.

## Key Copy Paths

- Python/runtime files copied into `/opt/libreqos/src` come from the explicit `LQOS_FILES` list in `src/build_dpkg.sh`.
- Example service files are copied from `src/bin/*.service.example` into `/opt/libreqos/src/bin`, then installed into `/etc/systemd/system` by `postinst`.
- Rust binaries are built in `src/rust/target/release/` and copied into `/opt/libreqos/src/bin`.
- `liblqos_python.so` is copied into `/opt/libreqos/src`.
- Node manager static assets are assembled into `src/bin/static2` and then copied into `/opt/libreqos/src/bin/static2`.
- `rust/remove_pinned_maps.sh` is copied into `/opt/libreqos/src/rust`.
- `update_api.sh` fetches `lqos_api` into the package output during `build_dpkg.sh`.

## Common Drift Points

- Adding a new Python helper or template that works in-repo but is not added to `LQOS_FILES`
- Adding frontend/static assets that are built locally but not copied into the package
- Adding a new binary or renaming an existing one without updating both `RUSTPROGS` and the related build path
- Depending on generated files in `src/bin/` without checking how they get there during packaging
- Changing install/service assumptions in `postinst` without reviewing operator impact
