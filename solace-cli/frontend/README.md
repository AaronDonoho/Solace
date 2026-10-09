# Solace frontend

The Lustre shell registers the board, contract view and migration components,
and uses modem for menu navigation and browser back/forward navigation.

| Page | Component |
| --- | --- |
| / | solace-board |
| /migrate | solace-migration |
| /contracts | solace-contracts |
| /p/<dir>/contracts | solace-contracts with base="/p/<dir>" |

Unknown paths fall back to the board. /migration remains the JSON API.
The menu (Board, Migration, Contracts) is on every page and highlights the
current one; a workspace project's contract page highlights Contracts. The
menu is 56px high, and the contract page fills the rest of the window
(`--contracts-height`). Each app owns its styles and lifecycle.
The shell owns the document font, base styles, menu and page-level drag rule.

## Build

`solace-cli/build.rs` runs this project's `build-release` command as part of
`cargo build`, so `priv/app.js` is rebuilt when the frontend, the board, the
contract view or the migration changes. To build it by hand, run from
solace-cli/frontend, with the sibling board, contracts and migration projects
present:

```sh
bun install
gleam build --target javascript
bun build src/main.mjs --outfile priv/app.js --minify --target browser
```

In Solace, use run install, then run build-release. The frontend needs its own
@dagrejs/dagre dependency because the contract view's JavaScript FFI is copied
into the frontend build tree.

## Tests

```sh
gleam test --target javascript
```

Tests cover URL routing, custom-element tags and base attributes, and the
current menu link. Component registration and modem require a browser.

## Server integration

The server task must serve this index.html at /, /migrate, /contracts and
/p/<dir>/contracts, and priv/app.js at /app.js, all on port 3030.

priv/app.js is generated, not source: `cargo build` builds it through this
project's `build-release` command before embedding it. The server must keep
serving the board's fonts at /fonts/sans-400.woff2, /fonts/sans-700.woff2 and
/fonts/mono-400.woff2 and board/guide.html at /guide, and preserve JSON and SSE
routes, including /migration.
Links inside app shadow roots use full page navigation.

After server integration, check menu navigation and back/forward, direct
loading of each page, and component cleanup when switching apps in a browser.
