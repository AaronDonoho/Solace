# pilot

Order-management service in plain Gleam: the conventional-tooling baseline for the benchmark (PROPOSAL.md §14.6, §15.1). Deliberately ordinary Gleam, so it's a fair comparison.

- `order`: pure order state machine (Draft → Submitted → Approved → Shipped; cancel from Draft/Submitted)
- `pricing`: quote with discount, shipping and tax rules
- `store`: immutable in-memory persistence
- `checkout`: service operations; time comes in via `clock.Clock`

`gleam test` runs the visible tests.
