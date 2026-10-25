# Default board

The Lustre client uses only the Spec 15 HTTP API. `solace serve` embeds the
built HTML and JavaScript, so an installed binary needs no asset directory.

The board holds the lanes, cards, new tasks and notices; projects are managed
in the Workspace page's Projects section (`../migration`). The open
task's details are `<solace-task>` from `../components`: the board hands it the
task's events, the updates of the tasks it depends on (for its Depends on line,
from any project) and a notice's tab, and follows it when it moves project. A live
update that raises a notice also sends the host `attention` (D-107), which
flags the Board tab while another page shows.

A task link (D-98) reaches the board through the shell's `focus` property,
`{dir, id, n}`: the task opens and its details scroll into view.

The open task's details sit in a band at the bottom of the board. The band's
bar is the task's only heading: its title, which collapses and expands the band,
then **Copy ID**, its column and its status, on the title's line while there's
room. The board sets `<solace-task>`'s `bar` attribute, so the details leave
these out; the Strategy page sets no bar, so its details keep them. When the
window is at least 1.1 times as wide as it is tall, an open band moves to the right
edge. The board narrows beside it down to its columns' smallest width, then
the band covers it. Its handle drags its width there (Left and Right step
it), never below 32rem. Collapsed there, the band is a narrow strip holding
only an expand button; on a narrower window it stays a line at the bottom.
At least 58.5rem wide, the details show in two columns 2rem apart: what the
human does (actions, model, message) on the left, about a third narrower than
it was, and the tabs on the right.
`layout_audit board` opens a task in the band at each of these sizes and
measures it.

A card is dimmed while a task in its `dependsOn` (D-87) is unfinished: not in
a final column of its workflow now, so complete and canceled both count as
finished and a reopened task counts again. The card's tooltip and a short line
on it say "Waiting on" and those tasks' titles. Dimming is only visual: the
card opens as any other. The root's event stream already brings every
project's tasks, so a dependency in another project needs no request of its
own, and the card follows each change live.

A card shows its title, cut to 200 characters with "…" when longer; the whole
title is in the card's tooltip. The card shows no scope line: the task's
Scope tab lists its projects. Its text fills the card; a finished task's
Archive button sits in the upper right corner and the title's first line
wraps around it.

A card names its feature on a line of its own under the title (D-155); the
tooltip spells it out as "Feature: …". The card's stripe takes the feature's
color (D-114).

The board leaves out the `backlog` column and its tasks (D-133): Backlog means
*not active*, so Planning is the first column. Backlog tasks are on the
Strategy page, which plans them. A task of a proposed, upcoming or canceled
feature stays off the board in any column (D-128).

Assigned, Review and Canceled offer **Back to planning**, the workflow's
`to_planning` action (D-133); a complete task's details offer nothing. The task
keeps its changes and loses its approval, and the server refuses it while an
agent is working. From Canceled the task returns with its notes, runs and
links, comes out of the archive, and gets a note saying the human brought it
back; cards that depend on it dim again. A button shows its action's `label`
when the workflow gives one, and the action's name otherwise.

Build after changing the client:

```
gleam build --target javascript
bun build src/main.mjs --outfile priv/app.js --minify --target browser
```

Check it without Solace or an agent:

```
gleam test --target javascript
bun test test/api.test.mjs
```
