---
name: libreqos-node-manager-frontend
description: Shared LibreQoS workflow for node_manager frontend work. Use when changing src/rust/lqosd/src/node_manager/js_build/**, static2/**, esbuild.sh, copy_files.sh, dev_build.sh, static_pages.rs, or the overall Bootstrap 5 and FontAwesome-based UI.
---

# LibreQoS Node Manager Frontend

Use this skill for `node_manager` frontend work.

## Scope

- Bundled page JS lives under `src/rust/lqosd/src/node_manager/js_build/src/`.
- Static HTML, CSS, and vendored assets live under `src/rust/lqosd/src/node_manager/static2/`.
- Served-page routing lives in `src/rust/lqosd/src/node_manager/static_pages.rs`.
- Copy/build helpers live in `src/rust/lqosd/copy_files.sh` and `src/rust/lqosd/dev_build.sh`.

## Build Contract

The frontend is page-oriented, not a SPA:

- one HTML page body per page in `static2/`
- one JS entrypoint per page in `js_build/src/`
- one bundled output per entrypoint in `js_build/out/`
- shared template shell in `static2/template.html`
- shared styling in `static2/node_manager.css`
- vendored third-party assets in `static2/vendor/`

`js_build/esbuild.sh` reads `js_build/entrypoints.txt`, builds each listed page entrypoint, and writes bundled `.js` plus sourcemaps into `js_build/out/`.

## Dependency Rules

- Bootstrap 5 is the UI framework.
- FontAwesome is the icon system.
- Vendored assets under `static2/vendor/` are the default.
- Do not infer frontend dependencies from `src/package.json`; it is not the source of truth for this UI.
- Avoid introducing new CDN or npm dependencies unless explicitly required. Existing CDN fallback behavior should be treated as an exception, not the default.

## Style Guide

- Preserve the existing shell: left sidebar, top nav tabs, rounded cards, dense data tables, badges, and dark/light theme support.
- Use `static2/template.html` and `static2/node_manager.css` as the baseline visual language.
- Favor Bootstrap 5 components and utility classes over ad hoc widget systems.
- Use FontAwesome icons that fit the existing navigation and dashboard vocabulary.
- Keep new pages consistent with the screenshots and existing pages: operational dashboard first, not marketing UI.
- Respect `data-bs-theme`, existing theme helpers, and the current dark/light behavior.

## Page Checklist

When adding or renaming a page, update all of:

1. `js_build/src/<page>.js`
2. `js_build/entrypoints.txt`
3. `static2/<page>.html`
4. `static_pages.rs`

If the page is served through the templated/authenticated router, use `%CACHEBUSTERS%` on its local bundle reference.

## Validation

- Run `src/rust/lqosd/src/node_manager/js_build/test-build-contract.sh`
- Run `src/rust/lqosd/src/node_manager/js_build/esbuild.sh`
- For broader local iteration, run `src/rust/lqosd/dev_build.sh`

## Notes

- The build-contract test checks page list alignment, required vendored assets, entrypoint/source alignment, and bundle output presence.
- `copy_files.sh` and `dev_build.sh` should stay aligned with the same node_manager asset pipeline.
