Implemented the Lustre migration. The visualizer is running at http://127.0.0.1:8765.

Browser behavior is now written in Gleam/Lustre: bench/visualizer/frontend/src/bench_visualizer_frontend.gleam:17

Ewe remains the Gleam backend and now serves the generated bundle: bench/visualizer/src/bench_visualizer.gleam:37

The HTML contains only styling and the application mount point—no handwritten JavaScript.

The generated app.js is bundled with Bun and invokes the Lustre entry point.

Build instructions are recorded in WORK.md:3.

The app retains renameable groups, multiple files/folders per group, explicit refresh, the results table, directory browser, and row-selected comparison graphs. I deliberately did not port the optional polling, sorting, or localStorage; the Refresh button satisfies the original directory-update requirement.

Verification completed:

Backend Gleam and Solace checks pass.

Backend test passes.

Frontend Gleam and Solace checks pass.

Frontend decoder test passes.

Generated JavaScript passes node --check.

Live Ewe smoke test served the self-starting Lustre bundle and returned five rows from a real benchmark directory.

What Solace was like here:

Its transactional submission was genuinely useful. An early frontend submission had a syntax error; Solace rejected the entire change and left the stored module valid.

Symbol-level editing was excellent for the later picker correction and the small backend route addition. I could inspect and replace exactly view_picker or handle, with an immediate project-wide type check.

solace check and solace test were concise, fast final validation boundaries.

The authoritative-store behavior was also surprising: after I staged a candidate directly in the managed source file, a rejected submission restored the previous stored version and erased that candidate. I adapted by compiling an untracked candidate module first. That behavior is safe, but Solace should probably warn more explicitly that rejection may overwrite filesystem divergence.

Greenfield setup remains its weak point. Solace could not create gleam.toml, bootstrap an empty package, edit HTML/CSS, or manage dependencies and bundling. Those steps still required normal filesystem tools.

Dependency discovery was outside Solace’s useful context. Since Lustre and RSVP sources are dependencies rather than project symbols, I had to inspect their installed package source conventionally to learn exact APIs.

Once the package existed, Solace became much more valuable. It felt strongest as a semantic editor for an established Gleam graph, not as a complete project/build environment.

Lustre itself was a good fit for the interactive portion: the model-update-view structure maps naturally onto groups, browsing, refresh effects, row selection, and graphs. The main friction was build tooling—Lustre’s official dev build tool required missing rebar3, so I used the already-installed Bun bundler instead.

Yes. The biggest issue was that Solace was excellent once symbols existed, but awkward during a large greenfield replacement. The Lustre migration sat exactly on that boundary.

The highest-impact improvements would be:

A non-destructive validation mode

I needed something like:

solace submit bench_visualizer_frontend --check-only < candidate.gleam

A rejected submission restored the authoritative stored version over my filesystem candidate. That protected consistency, but destroyed the draft I was iterating on. I then had to create a separate temporary module, compile it, and resubmit it.

Better behavior would be either:

Never overwrite the submitted filesystem draft on rejection.
Save rejected text under .solace/rejected/.
Provide submit --check-only or draft check.
Detect filesystem/store divergence and explicitly ask which version wins.

This was probably the largest avoidable source of repeated work.

Dependency-aware inspection

Most uncertainty was about Lustre and RSVP APIs—not our own code:

Exact lustre.application and lustre.start signatures.
Event propagation behavior.
Attribute and element function names.
RSVP response/error types.
Optional JSON decoder syntax.

Solace only indexed project modules, so I had to search generated dependency sources conventionally. A command such as:

solace inspect lustre.application
solace inspect rsvp.expect_json
solace find --deps stop_propagation

would have reduced both browsing and guessed code substantially.

It need not import every dependency into the main store. A read-only dependency index built from package interfaces would be enough.

A better path for creating a large module

Solace works naturally when changing one or two existing functions. Here I had a tiny placeholder and needed to introduce roughly 30 interdependent types and functions.

Submitting the whole module produced a large response and made one syntax error invalidate everything. Submitting symbols incrementally was difficult because private types and functions depended on one another.

Useful additions could be:

solace submit --batch-file frontend.solace
solace module replace bench_visualizer_frontend --from candidate.gleam
solace draft bench_visualizer_frontend

The important property is compiling the entire proposed module transactionally while retaining the draft between attempts.

Diagnostics against the submitted source

Errors were reported by extracted symbol, sometimes with local line numbers rather than the original submission’s line numbers. For a large module, that makes locating the fault harder.

Diagnostics should ideally include:

Original submitted line and column.
The owning symbol.
Whether the error occurred before or after import merging/reconstruction.
A small contextual excerpt.

Store synchronization as an explicit operation

This task mixed Solace-managed Gleam with ordinary files:

gleam.toml
HTML and CSS
generated JavaScript
manifests
build commands

That made filesystem/store divergence likely. A command such as:

solace sync
solace diff --store
solace import --update

would make mixed workflows much safer. Currently import is mainly bootstrap-oriented, while submit assumes Solace already owns the editing path.

Workspace awareness

The visualizer now has two Gleam packages and therefore two Solace stores:

visualizer/           Ewe backend

The effective workflow would be:

Ask Solace for the relevant module/symbol context.
Edit a local draft file with normal editor operations.
Submit that file through Solace.
Let Solace merge imports, reconstruct the module, type-check it, and commit it transactionally.
If validation fails, preserve the draft and return diagnostics against its original line numbers.

Conceptually:

solace submit bench_visualizer_frontend --file /tmp/frontend.gleam

For smaller changes, direct symbol submission remains better. For a large Lustre module, file-backed submission avoids spending model tokens reproducing the entire source inside an MCP argument, makes correction easier, and prevents rejected attempts from being lost.

I’d support both:

solace submit module.symbol --file patch.gleam
solace submit module --file module.gleam
solace submit module --file module.gleam --check-only

For MCP, the tool could accept a workspace-relative draft path rather than embedding 16 KB of source in JSON. Security-wise, it should restrict readable paths to the workspace or an explicit draft directory.
One important distinction: the draft should not itself become authoritative. Solace should read it, validate it transactionally, update the store

The main problem was not Solace.

Most of the time and tokens went into:

Learning unfamiliar Lustre and RSVP APIs.

Resolving frontend build tooling, especially the missing rebar3.

Writing a fairly large UI migration.

My own syntax, event-handling, and bundle-entry mistakes.

Verifying the browser bundle without a browser-testing environment.

Solace added a meaningful but secondary penalty: roughly 15–25k tokens from discarding the draft and forcing reconstruction. It also could not expose dependency APIs, which made learning Lustre slower.

Overall, I’d attribute perhaps 20–30% of the avoidable overhead to Solace and 70–80% to my implementation approach, unfamiliarity with Lustre, and ecosystem/build friction.

Once the module existed, Solace generally helped: targeted symbol inspection, atomic changes, automatic type-checking, and concise final validation all worked well. Its weakness here was supporting the transition from an empty scaffold to a large first implementation—not routine development afterward.
