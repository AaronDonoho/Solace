# Solace design

The Design page, `<solace-design>`, at `/design` in the shell. It has two tabs,
Architecture and UI/UX. Each tab lists its own designs by title in a flat list
on the left; picking one opens it. Each design has an arrow that expands its
history beneath it without opening the design, and collapses it again. Opening
a design and expanding its history are separate actions. Architecture designs
draw their flow diagram below the title; a UI/UX design draws its screens,
stacked down the page. A Preview button beside the title shows one screen at a
time; Exit preview returns to the stacked screens. A tab with no designs reads
"No designs yet."

Designs come from `GET /designs`, sorted by id, and the page follows live
`{designs}` updates from `/events`, so a design added or changed appears
without a reload. A design that disappears closes. An open design is read whole
from `GET /designs/{id}`, so the list stays small.

## History

An expanded history comes from `GET /designs/{id}/versions`, newest first, and
each row shows the version number, when it was saved and who saved it. Clicking
a row shows that version in the canvas and marks the row; the current design is
not changed. Picking the design again shows the current design.

A shown version has no Preview button.

A shown version has a Restore button unless it matches the current design.
Restoring posts the version number to `/designs/{id}/restore`, then shows the
versions the server returns and reads the open design again. A refused restore,
a failed history load and a version with no design to show display the server's
message without its code.

## Architecture

An Architecture design's top-level `flow` holds `node` and `edge` elements.
Each node is a box showing its label and description; each edge is an arrow
from its `from` node to its `to` node, with its label if it has one. Nodes are
placed by the Contract view's dagre layout. Every drawn node and edge carries a
`data-design-id`, so an agent can name it later. Clicking a node or edge selects
it by that id; Ctrl-click (Cmd-click on a Mac) adds or removes it, and dragging
across the diagram selects every node and edge wholly inside the box. A click on
empty diagram clears the selection. Selected nodes and edges have the same
orange treatment as screens. While a proposal is open, the Proposed/Current
switch and the hide-changes control work as they do for screens, marking added,
removed, changed and moved nodes and edges.

## Screens

The renderer in `render` draws a design's element tree of screens, rows,
columns, text, headings, buttons, inputs, image placeholders, component
stand-ins, tabs, tab items, scroll areas and split panes. Every drawn element
carries a `data-design-id`
attribute and a thin dotted outline, so its layout can be checked. Widths and
heights are `fill`, `hug` or a fixed number of pixels; fill children can shrink
inside their parent. Gaps and padding take a spacing token (`none`, `xs`, `sm`,
`md`, `lg` or `xl`) or a number of pixels, and default to 0. Alignment starts
at the start until the design states otherwise.

A tabs element holds tab items only, and its `current` property names one of
them. It draws a strip of tab labels with the current one marked, then only the
current tab's children. A scroll area holds its children in a box of its own
size and scrolls them inside it, so the rest of the screen stays put. A split
pane holds exactly two regions, side by side when its `direction` is `row` and
one above the other when it is `column`, with a divider between them.

A component stand-in names a component placed in the page and shows that name
in a dashed box of its own size. It holds no children and doesn't draw the
component's inside.

Outside Preview, clicking a drawn element selects it by its `data-design-id`.
A plain click selects only that element; Ctrl-click (Cmd-click on a Mac) adds
it, or removes it when it is already selected. A selected element has a thick
solid orange outline. Escape or a click on empty canvas clears the selection.
Opening another design, tab or history version clears it, as do entering or
leaving Preview, switching Proposed/Current and a live reload that removes a
selected element.

Outside Preview and edit mode, dragging across a drawn canvas selects every
element wholly inside the box shown while dragging. An element only partly
inside is not selected; when a parent and its children are all wholly inside,
every one is selected. A plain drag replaces the selection, and Ctrl-drag
(Cmd-drag on a Mac) adds the region's elements. A drag that covers nothing
clears the selection. Only the current tab's children are drawn, so a region
can select those but never the contents of another tab.

## Zoom and pan

The canvas has a zoom bar at its bottom-right: −, the current level, + and
Reset. Reset returns the canvas to 100% at the top-left of the drawing. Ctrl +
the mouse wheel zooms toward the pointer, and a plain wheel still scrolls.
Holding Space and dragging, or dragging with the middle button, pans the
canvas; a plain drag still selects a region or, in edit mode, moves an element.
Each bar button shows its tip after two seconds of hovering; clicking it first
hides the tip. The zoom level is a viewing setting: it never changes or saves
into the design.

Zoom and pan work on both tabs and in Preview. Elements are measured on screen,
so clicking, region selection, note placement, moves and resizes give the same
results at any zoom level and pan position. A resize saves the design's own
size, not the size it is drawn at.

## Window conditions

A screen may carry a `conditions` list that changes its layout for a window
shape or size. Each condition has a `when` bound and a `changes` list. A bound
tests the window's width-to-height `ratio`, its `width` or its `height`, with
`at_least` or `at_most` and a positive number. Each change names an element
inside that screen and either gives a `row` or `column` a new `direction`
(`row` or `column`), or sets one `property` to a `value` the element's type
accepts. Conditions apply in list order; when several match, a later condition
overrides an earlier one. Only a screen may carry conditions.

On the UI/UX tab, each screen is drawn in a window frame. Drag the frame's
bottom-right handle to change the window's size; the frame's size is the page's
own and does not follow Solace's window. The drawing applies every condition
that matches that size. The frame works with zoom and pan, so a window larger
than the canvas can be viewed by zooming out or panning.

A design agent adds a condition with
`{"op":"set","id":<screen>,"field":"conditions","value":<condition>}`, replaces
one by giving its `index`, and removes one with that `index` and a null `value`.

## Notes

With one or more elements selected outside Preview and edit mode, on either
tab, a small note box sits beside them. Typing a note and pressing Add note
stores the open design's id, the selected element ids and the text. The note
waits on the page and is never saved into the design.

Waiting notes appear in the agent panel in the order they were added, each with
its elements, its text and an × to remove it. The open design's noted elements
carry numbered markers that match that list. A note keeps its design id, so
switching designs, tabs or versions does not lose it; only the open design's
markers show.

The next message to the agent carries every waiting note as `notes`, whether it
starts a run or continues one. A successful reply clears the notes it sent; a
refused one keeps them and shows the server's message. Notes added while a
message is in flight wait for the next message. The Strategy page's panel has no
notes.

The built-in components draw from the default theme in `render.default_theme`:
colors, spacing, text sizes and corner radii are named tokens, so a design never
stores a token's value.

An element may carry a `style` object that overrides its component's look and
variant. It has a closed set of properties: `fill`, `border` and `text_color`
take a color token or a hex color (`#rgb` or `#rrggbb`), `size` takes a text-size
token or a positive number of pixels, and `padding` takes a spacing token or a
non-negative number of pixels. The renderer draws overrides after the component
and its variant, so an override wins. Any other property or a value outside
these forms is refused.

## Editing

Edit mode on either tab decides when a human can change the saved current
design. The Edit button beside the design's title turns it on, and the same
button then reads Done editing. Clicking selects an element in both modes.
Outside edit mode, dragging across the canvas selects the elements inside the
box, notes can be added, and the Edit tab lists the selected element's
properties read-only. In edit mode, dragging an element moves it, and no note
box is offered. A design with waiting changes opens in edit mode.

On either tab, click one element in the saved current design to select it. On
the UI/UX tab, the right panel's Edit tab lists the properties that element's
type holds, plus its style overrides. In edit mode, changing a field redraws
the design at once and adds a waiting change; nothing is sent to the server
yet. Choices use dropdowns, and colors offer a swatch picker beside a box for a
theme color name or a hex color. Clearing a style value clears that override.

On the Architecture tab, a node lists Label and Description, and an edge lists
Label. Changes wait for Save like any other edit, and the diagram redraws with
the new text before Save. Node positions come from the automatic layout, so
there is nothing to drag into place.

On the UI/UX tab in edit mode, a selected element shows small square handles on
its sides and corners. Dragging the left or right handle changes its width, the
top or bottom handle its height, and a corner changes both. A drag always sets
a fixed size, rounded to whole pixels; Fill and Hug are set in the Edit tab.
The size waits for Save like any other edit. A drag that ends at zero or less
is dropped, and the element goes back to the size it had before the drag. A
size the server refuses when you save shows its message and keeps the waiting
changes.

On the UI/UX tab in edit mode, drag an element to move it earlier or later
among its siblings or into another screen, row, column, tab, scroll area or
split pane. A faded copy follows the pointer while the other elements move
aside, so the design shows how it will look after the drop.
The element goes where the pointer is: over the near or far half of a sibling
it goes before or after it, and over a container's free space it joins that
container. Screens only move among screens. Tabs hold only tab items, a tab
cannot leave its tabs element, and a split pane takes exactly two regions.
Dropping adds one waiting move, `{"op":"move","id","parent","index"}`, where
`parent` is `null` for the top level and `index` counts the parent's children
without the moved element. A drop the design rules refuse, such as a screen
inside a row, a third region in a split pane, a non-tab in a tabs element, or
an element outside every screen, adds nothing and the element goes back; so
does a drop where the element already was.

Save sends every waiting change for the open design as one
`POST /designs/{id}/edit`, so it becomes one version. The saved design is drawn
and the waiting changes are cleared. Discard drops them. Waiting changes are
kept per design in the browser and survive switching designs, tabs or versions,
leaving the page, and reloading or closing the app; none of those asks before
leaving.

Press Done editing with waiting changes and Solace asks Save, Discard or Keep
editing. Save sends one version and leaves edit mode once it succeeds. Discard
drops the changes and leaves edit mode. Keep editing closes the prompt and stays
in edit mode. If the server refuses a save, its message shows, the waiting
changes stay and edit mode stays on.

Editing is offered only for the saved current design on both tabs; old
versions, the Proposed view and Preview stay read-only. The panel's Agent tab
keeps the design agent's runs, notes and message box, and the design agent can
change a design whether or not edit mode is on.

There is no text editor for the design's data.

## Preview

Preview starts at the first screen and shows one screen at a time. Clicking an
element follows its `navigates_to` link to that screen in the same design; an
element with only an `action` changes nothing when clicked. A design may put
`action` and `navigates_to` on rows, columns, text, headings, buttons and image
placeholders. `navigates_to` names a top-level screen in the same design, and a
missing or non-screen id is refused with `E7033`, naming the element. Screens
and text inputs take neither property.

Clicking a non-current tab shows that tab's content in Preview. That switch is
preview-only and is never saved; leaving Preview shows the design's current tab
again.

A click in Preview follows links instead of selecting.

Only the current design is previewed. Opening another design, tab or history
version leaves preview, and so does a live reload that drops the shown screen.

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
gleam build && bun test test/api.test.mjs
```
