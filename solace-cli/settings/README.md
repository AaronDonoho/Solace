# Solace settings

The Settings page, `<solace-settings>`, at `/settings` in the shell, and the
store every page reads its settings from.

## Store and pub-sub

`src/store.mjs` keeps each setting in the browser's `localStorage` under its
key. Storage may be missing or blocked; the pages work without it.

- `settings.load(key)`: the stored value, `""` when unset.
- `settings.save(key, value)`: stores the value (`""` clears it) and publishes
  the change.
- `settings.subscribe(key, handler)`: calls `handler` with each new value,
  whether it changed on this page (a `solace-setting` event on the document)
  or in another window (the `storage` event). It returns the function that
  unsubscribes.

A project that needs a setting depends on `settings` and subscribes to its key.

## Theme

The theme is the first setting: key `solace-theme` (`settings.theme_key`),
`light`, `dark`, or unset to follow the system. Every page's `index.html`
applies it to `document.documentElement.dataset.theme` before first paint, and
each page's dark colours follow `html[data-theme=dark]`. The Dark mode switch
calls `settings.set_theme`, which sets the attribute, stores the value and
publishes it. `settings.register()` makes the page follow a theme chosen in
another window.

## Languages

The Languages section lists Gleam and Rust from `GET /languages`, each with a
switch and whether its toolchain is installed. A switch sends `POST /languages`
with `{"id": ..., "enabled": ...}`; the reply is the new list. Both languages
are on unless the system settings turn one off. A language turned off behaves
as if its toolchain were missing: its projects stay readable, but `submit`,
`check` and `test` are refused and the Workspace page says the language is
turned off (D-22).

## Agent behavior

The Agent behavior section reads the machine-wide choices from
`GET /agent-settings`: `involvement` (`director`, `collaborator`, `engineer`),
`detail` (`concise`, `standard`, `thorough`) and `asking` (`often`, `balanced`,
`rarely`). A choice sends `POST /agent-settings` with
`{"key": ..., "value": ...}`; the reply is the new choices, or a refusal that
keeps the current ones. Each choice explains itself in plain words on hover or
keyboard focus. These choices belong to the server, so they are not kept in
browser storage (D-215).

## Role models

The Role models section lists every agent role from `GET /role-models`, each with
the model it starts with. The reply is an array of `{role, default}`; every role
is always listed, and an empty value is shown as "None (choose on each task)".
The picker lists the same models as the task page, including built-in ones. A
choice sends `POST /role-models` with one role's `{role, default}`; the reply is
the full list, and an unknown role is refused with E7001. Choices are kept in
the machine settings under `[role_models.<role>]`. A model that is not set up
here is kept, so it can return.

## Models

The Models section lists the models set up in `~/.config/solace/config.toml`
(`configured` in `GET /models`), each with why it can't start, and a form to add
or edit an endpoint model with its API key (`POST /models`; `DELETE
/models/<id>` removes one). The key is kept in `keys.json` beside the settings,
readable only by its owner; the page never gets it back, only `has_key`, so the
key field starts empty and a blank key keeps the saved one (D-164).

## Standalone page

`index.html` shows the page on its own, from `build/dev/javascript`, after
`gleam build`.

## Tests

```sh
gleam test           # the view and update
bun test test/store.test.mjs   # the store and pub-sub
```

In Solace, `test --all` and `run js_tests`. The switch in a real browser, a
reload and a change from a second tab need checking by hand.
