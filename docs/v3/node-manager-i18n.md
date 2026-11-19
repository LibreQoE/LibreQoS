# Web UI language

The `lqosd` dashboard at `http://your_shaper_ip:9123` can show its frame in
English or Spanish.

## Choosing a language

- Use the language selector in the sidebar, directly above `Display`. Before you
  sign in, the same selector is at the top right of the login and first-run
  screens.
- The choice is saved in your browser (a `lqos_lang` cookie, kept for one year).
  It does not change what other operators see.
- To go back to the node-wide default, delete the `lqos_lang` cookie and reload.

## Node-wide default

Set `ui_language` at the top level of `/etc/libreqos/lqos.conf` and restart
`lqosd`:

```toml
ui_language = "es"
```

Browsers without their own choice start in that language. Without
`ui_language`, the page follows the browser's language setting, then falls back
to English. An unsupported value is ignored.

## What is translated

Only the frame is translated: the sidebar, the `Display` menu, the language
selector, the search box, the alert panel, relative times such as `5 min. ago`,
and the login and first-run pages. Page content, tables, charts, help text and
alert messages stay in English.

Adding a language is a build-time change; see
`src/rust/lqosd/src/node_manager/I18N.md` in the source tree.
