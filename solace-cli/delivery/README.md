# Solace delivery

The Delivery page (`/delivery`), a Lustre element named `<solace-delivery>`.
Its Build tab starts a build of every project or chosen ones, cancels a running
build, lists the recipe's steps grouped by project with their live status and
duration, opens a step's log, and shows a failed step's summary at the top.
The last build comes from the server's replay when the page opens.

The Build tab follows the root's `/events`: `build` on every change,
`buildLog` while a step runs, and `builds` in the replay.

## Tests

```sh
gleam test --target javascript
```

The Gleam tests cover each step state, the failure summary and the cancel
button. `test/api.test.mjs` runs the page against a fake server for starting,
canceling and following a build live.
