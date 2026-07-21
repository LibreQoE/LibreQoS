# js_build

This assembles the JavaScript for the node_manager site using `esbuild`.

## Source of truth

- Page entrypoints live in `entrypoints.txt`
- Page source files live in `src/`
- Bundled outputs are written to `out/`

## Commands

- `./esbuild.sh`
- `./test-build-contract.sh`

`esbuild.sh` uses `ESBUILD_BIN` when set and matching `ESBUILD_VERSION`. Explicit `ESBUILD_BIN` overrides must already report the expected version. Otherwise it uses an `esbuild` on `PATH` only when its version matches `ESBUILD_VERSION`; if no matching binary is available, it downloads a standalone binary into the default managed cache at `src/rust/target/esbuild`. Set `ESBUILD_INSTALL_DIR` to relocate that cache.

Defaults:

- fallback esbuild version: `0.25.3`
- browser targets: `chrome85,firefox78,safari14`
- fallback cache: `src/rust/target/esbuild`

Optional overrides:

- `ESBUILD_BIN`
- `ESBUILD_VERSION`
- `ESBUILD_TARGETS`
- `ESBUILD_INSTALL_DIR`
