 Solace helped once the Gleam project existed, but hindered the greenfield application workflow. Most of the difficult work happened outside Solace.

 Area      Understanding code
 Helped    Symbol names made targeted inspection easy
 Hindered  At only 28 symbols, orientation savings were
           negligible
──────────────────────────────────────────────────────────────
 Area      Editing
 Helped    Batched submission added is_result_row and
           task_decoder transactionally
 Hindered  Shell-based stdin submission was awkward without
           the dedicated MCP tool
──────────────────────────────────────────────────────────────
 Area      Correctness
 Helped    Every submission type-checked immediately; solace
           check and solace test were concise
 Hindered  Dependency/build failures live outside the semantic
           model
──────────────────────────────────────────────────────────────
 Area      Project creation
 Helped    —
 Hindered  Solace cannot create gleam.toml, manage
           dependencies, or bootstrap a new package
──────────────────────────────────────────────────────────────
 Area      Mixed application
 Helped    Gleam backend functions were represented cleanly
 Hindered  HTML, JavaScript, manifest, and WORK.md were
           invisible to Solace
──────────────────────────────────────────────────────────────
 Area      Synchronization
 Helped    The store became a useful model after import
 Hindered  After an external edit, solace import refused
           because a store already existed; I had to repeat
           the changes through solace submit
──────────────────────────────────────────────────────────────
 Area      Running the app
 Helped    Tests worked through Solace
 Hindered  Starting Ewe and checking live HTTP endpoints still
           required the shell

What was genuinely good:

solace import cleanly identified 28 symbols across the backend and test modules.

view is_result_row and find task_decoder answered precise questions without reading the whole module.

The batched submit was the strongest part: it added a new helper and changed its caller atomically, then immediately reported that the entire project still type-checked.

solace test reduced a normal compile/test cycle to a very clean result.

Symbol identity was nicer than line-oriented patching once the code stabilized.

What was hard:

Greenfield bootstrapping is currently outside Solace. I had to create the package, configuration, dependencies, source files, tests, and frontend before importing anything.

The hardest problem—choosing and configuring an HTTP server—was package-level. Mist failed because of a missing rebar3; then we considered the built-in server and moved to Ewe. Solace had no dependency knowledge that could guide or validate that decision before compilation.

External edits and the store diverged. This was the clearest workflow flaw. Solace needs something like import --update or sync that:
Detects projection changes.
Shows the affected symbols.
Reconciles them transactionally.
Refuses only actual conflicts.

A web application is not solely Gleam symbols. The frontend asset, package manifest, lockfile, and documentation remained ordinary files. Solace either needs a concept of managed resources or a deliberately smooth coexistence with file tools.

The protocol currently fits maintenance better than creation. Once symbols exist, it is pleasant. Before they exist, the user falls back to conventional tooling.

My net assessment: Solace improved the final semantic edits and verification, but it added overhead overall for this small greenfield app. The exercise revealed a valuable product boundary: Solace is already useful for maintaining an imported Gleam codebase, but it does not yet own the whole application lifecycle.

The highest-value improvement exposed here is synchronization—not more HTTP knowledge. If Solace can safely absorb external project changes, agents can use ordinary tools for manifests and assets while still getting semantic editing where it matters.
