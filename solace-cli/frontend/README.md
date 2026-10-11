# Solace frontend

The Lustre shell registers the board, contract view, migration, strategy and
settings components, and uses modem for menu navigation and browser
back/forward navigation.

| Page | Component |
| --- | --- |
| / | solace-board |
| /migrate | solace-migration |
| /contracts | solace-contracts |
| /strategy | solace-strategy |
| /settings | solace-settings |
| /p/<dir>/contracts | solace-contracts with base="/p/<dir>" |

Unknown paths fall back to the board. /migration remains the JSON API.

## Task links (D-98)

Markdown on every page draws `task:<id>` and `[label](task:<id>)` as links to
`/task/<id>`. A plain click on one is followed in-page by `src/task_link.mjs`
(the links live in the pages' shadow roots, where modem doesn't route); a
click with a modifier key held, or loading `/task/<id>` directly, reaches the
shell at that address, which resolves it the same way and replaces it in the
history. The shell asks `GET /tasks/find/{id}`:

- on the board (not archived): the Board page shows, and `<solace-board>` gets
  the task in its `focus` property, `{dir, id, n}`;
- archived: the Strategy page shows, and `<solace-strategy>` gets it the same
  way and opens it on its Tasks tab;
- no such task: the page stays, and a toast at the bottom says "Task does not
  exist" for 4 s. A lookup that fails (the network, or another server error)
  says "Couldn't look up the task. Try again." instead, since the task may
  exist.

`n` counts the links followed, so following the same one again is news to
the page. Copying a task link gives its markdown back.
The menu (Board, Migration, Contracts, Strategy, Settings) is on every page and
highlights the current one; a workspace project's contract page highlights
Contracts. The menu is 56px high, and the contract page fills the rest of the
window (`--contracts-height`). Each app owns its styles and lifecycle.
The shell owns the document font, base styles, menu and page-level drag rule.

The theme is chosen on the Settings page (Dark mode). `index.html` applies the
stored choice (`solace-theme`) before first paint and holds every page's dark
colours; registering Settings makes the shell follow a theme chosen in another
window.

## Build

`solace-cli/build.rs` runs this project's `build-release` command as part of
`cargo build`, so `priv/app.js` is rebuilt when the frontend or one of its
pages changes. To build it by hand, run from solace-cli/frontend, with the
sibling board, components, contracts, migration, strategy and settings
projects present:

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

The server task must serve this index.html at /, /migrate, /contracts, /strategy,
/settings, /task/<id> and /p/<dir>/contracts, and priv/app.js at /app.js, all on port 3030.

priv/app.js is generated, not source: `cargo build` builds it through this
project's `build-release` command before embedding it. The server must keep
serving the board's fonts at /fonts/sans-400.woff2, /fonts/sans-700.woff2 and
/fonts/mono-400.woff2 and board/guide.html at /guide, and preserve JSON and SSE
routes, including /migration.
Links inside app shadow roots use full page navigation.

After server integration, check menu navigation and back/forward, direct
loading of each page, and component cleanup when switching apps in a browser.
On Settings, check that Dark mode switches every page, survives a reload, and
that a change in a second tab reaches the first.

Find in page (D-86) needs a DOM, so check it by hand, in a browser and in the
app (`cargo tauri dev` and the AppImage): Ctrl+F opens the bar, also from a
textarea; typing shows "n of m" and a card title is marked; marks follow lane
and page scrolling and are clipped to the lane; cards still click and drag,
and stay unselectable; Enter/Shift+Enter step and scroll; Esc closes and
clears; a query with no matches clears the marks; a live board update and a
switch between pages re-run the search.
