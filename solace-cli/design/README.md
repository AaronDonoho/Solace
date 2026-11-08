# Solace design

The Design page, `<solace-design>`, at `/design` in the shell. It has two tabs,
Architecture and UI/UX. Each tab lists its own designs by title in a flat list
on the left; picking one opens it. Each design has an arrow that expands its
history beneath it without opening the design, and collapses it again. Opening
a design and expanding its history are separate actions. Architecture designs
show their title. A UI/UX design also draws its screens below the title,
stacked down the page. A tab with no designs reads "No designs yet."

Designs come from `GET /designs`, sorted by id, and the page follows live
`{designs}` updates from `/events`, so a design added or changed appears
without a reload. A design that disappears closes. An open UI/UX design is read
whole from `GET /designs/{id}`, so the list stays small.

## History

An expanded history comes from `GET /designs/{id}/versions`, newest first, and
each row shows the version number, when it was saved and who saved it. Clicking
a row shows that version in the canvas and marks the row; the current design is
not changed. Picking the design again shows the current design.

A shown version has a Restore button unless it matches the current design.
Restoring posts the version number to `/designs/{id}/restore`, then shows the
versions the server returns and reads the open design again. A refused restore,
a failed history load and a version with no design to show display the server's
message without its code.

## Screens

The renderer in `render` draws a design's element tree of screens, rows,
columns, text, headings, buttons, inputs and image placeholders. Every drawn
element carries a `data-design-id` attribute and a thin dotted outline, so its
layout can be checked. Widths and heights are `fill`, `hug` or a fixed number
of pixels; fill children can shrink inside their parent. Gaps and padding take
a spacing token (`none`, `xs`, `sm`, `md`, `lg` or `xl`) or a number of pixels,
and default to 0. Alignment starts at the start until the design states
otherwise.

The built-in components draw from the default theme in `render.default_theme`:
colors, spacing, text sizes and corner radii are named tokens, so a design never
stores a token's value.

The tab shown is in the address as `?tab=<name>`, and the open design as
`?design=<id>` beside it; Architecture, the first tab, has no `tab` query, so a
design alone is `?design=<id>`. A missing or unknown tab shows Architecture. An
open design from the other tab, an unknown one, or a malformed id opens nothing
and leaves the page working. A tab click emits `tab` and a design pick emits
`design`, which the shell writes to the history without replacing the page. A
load, Back or Forward sets the page's `tab` and `design` attributes, which
restore them without emitting.

## Standalone page

`index.html` shows the page on its own, from `build/dev/javascript`, after
`gleam build`.

## Tests

```sh
gleam test
```
