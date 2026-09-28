# Spec 3: Agent Protocol and Diagnostics

**Status:** accepted 2026-09-27 · **Implements:** PROPOSAL.md §14 · **Needed by:** Phase 1a

The protocol is the agent's environment. It has few, rich commands that return compact text. Anything a human can see or do is available here (parity).

## 1. Transport

- A CLI, `solace <command>`, run through the agent's shell. Output is plain text by default; `--json` gives the same content as structured data.
- **One process per call, no daemon**, as long as the latency budget (§7) holds.
- **Agents use MCP** (D-9): `solace mcp` serves the same commands as one tool, `solace(command, text)`, so code is passed without shell quoting and the agent needs no shell.
- The store lives at `.solace/store.db` in the project root. `solace` finds it by walking up from the working directory.
- Exit codes: `0` success; `1` diagnostics (the command ran and reported problems); `2` usage error.

## 2. Commands (Phase 1)

| Command | Purpose |
| --- | --- |
| `session start [--scope <ns>…]` | Orientation: language reference version, policy for the scope, pending and recent decisions, and a symbol index: the names of every symbol in scope, grouped by namespace with its one-line description, with "N more" beyond a per-namespace cap (D-12). Open work records are expanded with the context bundles of the symbols they name (D-10) |
| `inspect <name>… [--context]` | Each symbol's text, signature, effects, description, verification status. `--context` adds the bundle in §3. Several names return several bundles in one document (D-10) |
| `view <ns> \| <name>… \| --callers <name> \| --callees <name>` | Multi-symbol document (Spec 2 §3). A namespace is shown as an outline (signature, effects, one-line description, verification status per symbol); `--bodies` gives full text (D-10) |
| `find <query>` | Search names, signatures and descriptions. Ranked, at most 20 hits, one line each |
| `submit <name>` / `submit --new <ns>` / `submit --view` | Text on stdin (Spec 2 §2, §3) |
| `rename`, `copy`, `merge`, `delete`, `extract` | Spec 1 §5 |
| `check [<name>…]` | Fast check; the default is everything changed in the current change |
| `test [<name>…]` | Examples and tests for the named symbols and their dependents (focused plan) |
| `diff` | Semantic diff of the current change: symbols added, modified, renamed and retired; signature, effect and contract changes listed first |
| `commit -m <msg>` | Publish the current change (§4) |
| `work open\|update\|close\|list` | Work records (§5) |
| `decide request\|list\|answer` | Decision requests (§5) |
| `export`, `import` | Spec 2 §4, §5 |

Every command that takes a name resolves it per Spec 1 §4 and fails with the candidate list.

## 3. Context bundle

`inspect <name> --context` returns one document with these sections, always in this order, omitting empty ones:

1. **Symbol:** canonical text, with its kind, qualified name and short identity
2. **About:** description and rationale; possibly-stale prose is marked `[stale]`
3. **Uses:** signatures (not bodies) of every symbol it references, grouped types → constants → functions
4. **Used by:** direct callers with their signatures; at most 20, then `…and N more`
5. **Verification:** examples and tests covering it, last results, verification debt, holes
6. **Decisions:** decision records attached to it or its namespace
7. **Work:** open work records that touch it

The target size is under 2,000 tokens for a typical pilot function. Nothing in a bundle requires a second call to interpret.

## 4. Changes and transactions

- Each session has one **open change**. `submit` and the structural commands apply to it immediately, and each is atomic.
- A submitted symbol must pass its own fast check. Dependents it breaks, such as callers of a changed signature, are tracked as the change's **broken dependents**. They are shown after every submit and in `diff`, and they clear as each one is fixed. This allows refactors one step at a time.
- `commit` is refused (`E5103`) while the change has broken dependents.
- A change records its **base revisions**: the revision of each symbol it touched, as first read.
- `commit` publishes the change to the main line. If another commit changed any base revision in the meantime, the commit is rejected with `E5101`, listing the symbols and a semantic diff of what changed. Nothing is merged silently.
- **Semantic conflicts:** if another commit changed the signature, effects or contracts of a symbol this change *uses*, even one it didn't edit, `commit` re-runs the fast check on the change and rejects it on failure (`E5102`).
- Phase 1 has no write-scope declaration; the base revisions serve that role. Declared scopes arrive with multi-agent work in Phase 6.

## 5. Work records and decisions

- **Work record:** goal, steps done, steps remaining, blockers, and the symbols it touches. `session start` and `inspect --context` show open records. A record belongs to the store, not to the session, so any later session can continue it.
- **Decision request:** a question, 2–4 options each with its consequences, and the affected symbols. It stays `pending` until a human answers. `decide answer` records the chosen option, who chose it, when, and an optional rationale. A hole with a question (Spec 4) raises a decision request automatically.

## 6. Diagnostics

Every diagnostic has:

```
<code> <title>
  at <qualified name>[ #<expr>]: <source excerpt>
  <one or two sentences of explanation, naming the actual and expected>
  fix: <mechanical fix, if there is one>
  affects: <symbols whose checks this blocks>
```

- **Stable codes**, never reused. Ranges: `E00xx` parse and projection, `E10xx` names, `E20xx` types, `E30xx` effects, `E40xx` contracts, examples and holes, `E50xx` store and transactions, `W` prefix for warnings.
- **Root cause first.** Diagnostics caused by an earlier one (for example, a failed type that makes every caller fail) are grouped under it as `caused by above: N more`, not listed separately.
- **Fixes** are either replacement text for the symbol or a command. The agent applies them explicitly; the store never auto-applies.
- `--json` gives the same fields.

## 7. Latency budget

On a store the size of the pilot, run on a developer laptop:

| Operation | Budget |
| --- | --- |
| `inspect`, `view`, `find`, `session start` | 200 ms |
| `submit` of one symbol, including the fast check | 1 s |
| `commit` | 2 s |

## 8. Phase 1a note

Over Gleam, the fast check calls `gleam check` on a generated project materialized from the store. If that misses the 1 s budget, the budget is recorded as missed rather than relaxed. It is a Phase 1b concern, since Solace has its own checker.

## Tests (acceptance)

- A1. Every command in §2 has a golden-output test, in text and `--json`.
- A2. A commit whose base revisions changed is rejected with `E5101`, and nothing is published.
- A3. A contract or signature change to a used symbol causes `E5102` on the other change's commit.
- A4. A work record opened in one session appears in the next session's `session start`.
- A5. A single type error in a type used by 5 functions yields one root diagnostic with `caused by above: 5 more`, not 6 separate ones.
- A6. The latency budgets in §7 are met on the pilot, measured in CI.
- A7. Changing a signature records the new revision and lists its callers as broken dependents; `commit` is refused with `E5103` until they are fixed.
