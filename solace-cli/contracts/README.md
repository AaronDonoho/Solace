# contracts

The Contract view page of the Solace web app: a Lustre (Gleam, JavaScript target) web component, `<solace-contracts>`. It reads one attribute, `base`, the API prefix of the project it shows (`/p/<dir>` in a workspace, empty for the root project; without one, a page at `/p/<dir>/contracts` asks its own project). It carries its own styles and talks to the server's `/graph`, `/nodes`, `/files`, `/tests`, `/tasks` and `/events` endpoints.

`contracts.register()` returns `Result(Nil, lustre.Error)` and defines the element. The shell (`solace-cli/frontend`) registers it and bundles it with the board and migration into one `app.js`, served at `/contracts` and `/p/<dir>/contracts` below the shell's menu bar. `main()` registers it on its own, for `index.html`.

## Build

```sh
bun install
gleam build --target javascript
```

There is no bundle step here; the shell's bundle includes this project. The drawing's layout uses `@dagrejs/dagre`, so the shell has it as a dependency too.

## Test

```sh
gleam test --target javascript
bun test test/view.test.mjs
```

## Layout

- `src/contracts.gleam`: the component.
- `src/api.mjs`: requests and the event stream, with the project prefix.
- `src/layout.mjs`: node placement (dagre), fitting, keys, focus and the panel resizer.
- `index.html`: standalone page.
- `test/`: Gleam tests and Bun tests.
