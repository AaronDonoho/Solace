# Solace frontend

The Lustre shell registers the board, contract view, migration, strategy and
settings components, draws its own Knowledgebase page (the guide, D-140), and
uses modem for menu navigation and browser back/forward navigation.

| Page | Component |
| --- | --- |
| / | solace-board |
| /workspace | solace-migration (the Workspace page, D-146); /migrate lands here |
| /contracts | solace-contracts |
| /strategy | solace-strategy |
| /settings | solace-settings |
| /knowledgebase | the shell's own page, `knowledgebase.gleam` (the guide); /guide lands here |
| /p/<dir>/contracts | solace-contracts with base="/p/<dir>" |

Unknown paths fall back to the board. /migration remains the JSON API, and
/projects the projects API.

## Task, feature and milestone links (D-98, D-122)

Markdown on every page draws `task:<id>` and `[label](task:<id>)` as links to
`/task/<id>`, and `feature:<id>`, `milestone:<id>` the same way, to
`/feature/<id>` and `/milestone/<id>`. A plain click on one is followed in-page
by `src/task_link.mjs` (the links live in the pages' shadow roots, where modem
doesn't route); a click with a modifier key held, or loading the address
directly, reaches the shell at that address, which resolves it the same way
and replaces it in the history. For a task the shell asks `GET /tasks/find/{id}`:

- on the board (not archived, in a project): the Board page shows, and
  `<solace-board>` gets the task in its `focus` property, `{dir, id, n}`;
- archived, or with no project of the workspace (D-129): the Strategy page
  shows, and `<solace-strategy>` gets it the same way and opens it on its
  Tasks tab;
- no such task: the page stays, and a toast at the bottom says "Task does not
  exist" for 4 s. A lookup that fails (the network, or another server error)
  says "Couldn't look up the task. Try again." instead, since the task may
  exist.

For a feature or milestone it asks `GET /features/find/{id}` or
`GET /milestones/find/{id}`; a found one opens on the Strategy page, whose
`focus` property names its `kind`: a feature shows on the Features tab, a
milestone on the Roadmap tab, its card scrolled into view and marked. An
unknown one toasts "Feature does not exist" or "Milestone does not exist".

The `focus` property is `{kind, dir, id, n}`; `n` counts the links followed, so
following the same one again is news to the page. A bare link shows its
record's title once the server gives it (kept 30 s), marked `data-bare`;
copying any of these links gives its markdown back, a bare one still bare.
The menu (Workspace, Strategy, Board, Contracts, Knowledgebase, Settings) is on every page and
highlights the current one; a workspace project's contract page highlights
Contracts. The menu is 56px high, and the contract page fills the rest of the
window (`--contracts-height`). The menu is sticky, always visible while a page
scrolls, above the pages' layers and below the find bar and toasts. Each app
owns its styles and lifecycle.
A page's `attention` event (D-107) flags its menu tab, unless it is the page
shown: the tab flashes (lit without motion under reduced motion) and reads
"(needs attention)" to screen readers until the page is shown. The board and
Strategy are open from the start, hidden, so they hear of what needs attention.
The shell owns the document font, base styles, menu and page-level drag rule.

The theme is chosen on the Settings page (Dark mode). `index.html` applies the
stored choice (`solace-theme`) before first paint and holds every page's dark
colours; registering Settings makes the shell follow a theme chosen in another
window.

The Settings page also turns Gleam and Rust on or off for the whole computer
(D-22); a project of a language turned off shows that on the Workspace page,
while reads still work and changes, checks and tests are refused.

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
into the frontend build tree. The `@codemirror/*` packages are there for the
same reason: they are the markdown editor, `<solace-md-editor>`, in
components' `md_editor.mjs`.

## Tests

```sh
gleam test --target javascript
```

Tests cover URL routing, custom-element tags and base attributes, and the
current menu link. Component registration and modem require a browser.

## Server integration

The server task must serve this index.html at /, /workspace, /migrate, /contracts, /strategy,
/settings, /knowledgebase, /task/<id>, /feature/<id>, /milestone/<id> and /p/<dir>/contracts, and
priv/app.js at /app.js, all on port 3030.

priv/app.js is generated, not source: `cargo build` builds it through this
project's `build-release` command before embedding it. The server must keep
serving the board's fonts at /fonts/sans-400.woff2, /fonts/sans-700.woff2 and
/fonts/mono-400.woff2, redirect /guide to /knowledgebase, and preserve JSON and SSE
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
switch between pages re-run the search; collapsing a lane, closing a note,
transcript entry, plan transcript or New body, and a task's details closing
clear their marks and the count.

The markdown editor needs a DOM too. Check it by hand on a task's description
and note box and on Strategy's vision, a feature, a milestone and the plan box:
typing styles headings, bold, code, quotes, lists and links, and their marks
show only on the cursor's lines; each toolbar tool works on a selection and on
none; pasting, dropping and picking an image uploads it once and shows it in
place; undo works; Ctrl+Enter sends in Plan without a new line; Ctrl+F opens
the find bar from the editor; Tab leaves the editor; a screen reader reads its
label; dark mode colours it; the cursor stays put while the board's live
updates arrive; and text can be dragged within the editor.
