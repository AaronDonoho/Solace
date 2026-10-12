# Default board

The Lustre client uses only the Spec 15 HTTP API. `solace serve` embeds the
built HTML and JavaScript, so an installed binary needs no asset directory.

The board holds the lanes, cards, new tasks, projects and notices. The open
task's details are `<solace-task>` from `../components`: the board hands it the
task's events, the updates of the tasks it depends on (for its Depends on line,
from any project) and a notice's tab, and follows it when it moves project. A live
update that raises a notice also sends the host `attention` (D-107), which
flags the Board tab while another page shows.

A task link (D-98) reaches the board through the shell's `focus` property,
`{dir, id, n}`: the task opens and its details scroll into view.

A card is dimmed while a task in its `dependsOn` (D-87) is unfinished: not in
a final column of its workflow now, so complete and cancelled both count as
finished and a reopened task counts again. The card's tooltip and a short line
on it say "Waiting on" and those tasks' titles. Dimming is only visual: the
card opens as any other. The root's event stream already brings every
project's tasks, so a dependency in another project needs no request of its
own, and the card follows each change live.

A canceled task's details offer **Back to backlog**, the workflow's
`to_backlog` action on the `cancelled` column (D-108); a complete task's offer
nothing. The task returns to the backlog with its notes, runs and links, comes
out of the archive, and gets a note saying the human brought it back; cards
that depend on it dim again. A button shows its action's `label` when the
workflow gives one, and the action's name otherwise.

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
