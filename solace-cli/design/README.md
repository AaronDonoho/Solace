# Solace design

The Design page, `<solace-design>`, at `/design` in the shell. It has two tabs,
Architecture and UI/UX, both empty for now.

The tab shown is in the address as `?tab=<name>`; Architecture, the first tab,
has no query. A missing or unknown tab shows Architecture. A tab click emits
`tab`, which the shell writes to the history without replacing the page. A
load, Back or Forward sets the page's `tab` attribute, which restores the tab
without emitting.

## Standalone page

`index.html` shows the page on its own, from `build/dev/javascript`, after
`gleam build`.

## Tests

```sh
gleam test
```
