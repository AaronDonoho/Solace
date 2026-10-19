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
