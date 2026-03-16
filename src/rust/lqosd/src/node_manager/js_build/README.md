# Site_build

This assembles the JavaScript for the node_manager site using `esbuild`.

## Source of truth

- Page entrypoints live in `entrypoints.txt`
- Page source files live in `src/`
- Bundled outputs are written to `out/`

## Commands

- `./test-build-contract.sh`
- `./esbuild.sh`

`esbuild.sh` prefers a locally installed `esbuild` binary when available. If none is found, it falls back to downloading a standalone binary into `/tmp/esbuild`.
