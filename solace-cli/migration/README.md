# migration

The Migration page of the Solace web app: a Lustre (Gleam, JavaScript target) web component, `<solace-migration>`. It reads no attributes and carries its own styles. It talks to the server's `/migration` JSON endpoints. URLs and the app contract are in D-62 (`DECISIONS.md`).

`migration.register()` returns `Result(Nil, lustre.Error)` and defines the element. The shell (`solace-cli/frontend`) registers it and bundles it with the board and view into one `app.js`. `main()` registers it on its own, for `index.html`.

## Build

```sh
gleam build --target javascript
```

There is no bundle step here; the shell's bundle includes this project.

## Test

```sh
gleam test --target javascript
bun test test/migration.test.mjs test/migration_states.test.mjs   # command: migration-tests
python3 test/browser.py                                           # command: migration-browser (needs a browser)
```

## Projects

The Projects section lists the root's projects from `GET /projects`. Under a
project, `needs` says why Solace can't use its tools: its toolchain is missing,
or the language is turned off in Settings (D-22). Reads still work; changing or
checking code is refused.

## Layout

- `src/migration.gleam`: the component.
- `src/migration/api.gleam`, `src/api.mjs`: requests to `/migration`.
- `index.html`: standalone page.
- `test/`: Gleam tests, Bun tests, browser check (`browser.py`, `component.html`).
