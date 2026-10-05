# Solace: A Programming System for Agents and the Humans Who Direct Them

Proposed Sep 27, 2026 · revised Oct 3, 2026 · @Aaron

**Status:** Design proposal, revision 4. Where this document and the decision records in `DECISIONS.md` (D-1 to D-51) differ, the decisions apply. Phase 0 and the Phase 1a checkpoint are complete, and Phase 1b is in progress (section 14.2).

## 1. Summary

Solace is a programming system in which AI agents write and maintain software and humans direct their work. It keeps a program in a store as a set of *symbols*: the functions, types, state machines and other named parts of the program. Each symbol has a permanent identity, and the store records its contracts, tests, verification status and the decisions made about it. Source text is generated from the store whenever an agent reads or writes code. The text is a view of the program; the store holds the program itself.

Programming languages and their tools were designed for people editing text files. Agents use the same tools and lose much of their time to them. They search and read files to find their way around, their edits fail when the text they were matching has changed, they miss inputs a program reads from its environment, they cannot tell flaky tests from real failures, and they forget everything between sessions. Humans, for their part, cannot read the volume of line-by-line changes agents produce, so review becomes approval without reading.

Solace gives each side its own view of the same program. Agents read and change code by symbol, one or many at a time, through a single tool that checks every change as it arrives. Humans see what the program promises and what it does: its contracts, its side effects and its tests. They direct agents through a task board and read source code only if they want to.

Two goals come first, because without them humans and agents cannot work together on a project at all:

1. **Guardrails for agents.** An agent works on a project only through Solace. It has no shell and no direct file access, so every change it makes is checked, recorded and reversible, and it cannot get around the checks.
2. **An interface for humans.** A human directs agents through tasks, sees the system's behavior and footprint (its APIs, logic, infrastructure and side effects), and answers agents' questions without reading their code.

Efficiency, meaning the tokens and time an agent spends on a task, is a secondary goal. It is measured and improved, but never at the expense of the first two.

Solace is a product that is installed once and wraps a user's project. It keeps three things apart: the user's project files, Solace's settings for that project (`solace.toml`, stored inside the project), and the Solace system itself (its install, the agent configurations it writes, and its environment).

Solace is built in phases (section 14). The early phases wrap an existing language, Gleam, so that the store, the agent tool, the guardrails and the human interface can be tried on real code before a new language exists. The Solace language is built only after the store has shown a measurable improvement over working with plain files. It will be a small functional language that compiles to the BEAM (the Erlang virtual machine) for services and to JavaScript for browser interfaces. Gleam is a stepping stone, so every part of the design that can be independent of the language is kept that way.

## 2. The Problem

### 2.1 Agents and humans are good at different things

Agents write and rewrite code quickly, read large amounts of it, follow explicit instructions and conventions, and correct themselves from automated feedback. They are poor at noticing what they cannot see: implicit behavior, inputs a program reads from its environment, dependencies nobody listed, and anything graphical. They start every session with no memory, and they tend to report success before they have it. Humans are the opposite. They write code slowly, but they know what the software is for, judge quality with their own eyes, decide what "correct" means and weigh tradeoffs.

|  | Agents | Humans |
| --- | --- | --- |
| Good at | Producing and changing code quickly; reading a lot of it; following explicit specifications and conventions; writing tests when told what to test | Stating intent and priorities; judging quality by looking and using; deciding what "correct" means; weighing tradeoffs; handling new kinds of emergency |
| Poor at | Seeing implicit behavior and hidden inputs; judging visual output; remembering across sessions; knowing when they are really done; resisting the urge to weaken a failing check | Keeping up with the volume and speed of agent changes; reviewing large line-by-line diffs; holding a whole system in their head |
| Time lost today | Finding their way (search, open, read, repeat); setting up environments and toolchains; flaky tests; debugging with print statements; edits that fail on text matching; re-deciding conventions every session | Reading diffs full of mechanical changes to find the few decisions that matter; navigating files to understand behavior; re-explaining decisions that were never written down |
| What Solace gives them | Text views of individual symbols, the context for a task in one call, what a change reaches, probes and call traces, explicit side effects, replayable runs, fast checks, work records that persist, and no arbitrary choices to make | A task board for directing agents, contracts and guaranteed behavior, runs with their own data, decision trees, replayed scenarios, a queue of questions to answer, and one click to any definition |

Solace is designed around this split. It asks humans only for what humans alone can judge, and it gives agents the information they cannot find for themselves.

### 2.2 Where the time goes

Current tools cause the following problems, and they get worse when agents do the building and humans have to keep up.

| Problem | Consequence |
| --- | --- |
| Code organized by file | The logic of one behavior is spread across unrelated files. |
| Code identified by its text | Refactoring breaks references, documentation and change history. |
| Implicit state, side effects and hidden inputs | Agents must work out dependencies they cannot see: configuration, environment, time, global state, ordering. |
| Nondeterministic execution | Flaky tests make real failures look like noise. |
| Debugging without history | Agents fall back on print statements and repeated reruns to learn what a program did. |
| Weak verification | Wrong implementations compile and pass incomplete test suites. |
| Checks that can be edited | Agents under pressure weaken tests or assertions instead of fixing behavior. |
| Documentation kept apart from code | Rationale and design decisions go stale or get lost. |
| No memory between sessions | Each session works out the conventions again and loses plans in progress. |
| Arbitrary choices | Formatting, organization, library choice and visual styling vary from session to session. |
| Environment and toolchain setup | Builds depend on the machine they run on, and setup eats agent time. |
| Performance tools kept separate | Functional changes quietly slow the program down or make it use more resources. |
| Screenshot-based UI testing | Visual and interaction regressions are hard to detect reliably. |
| Text-based version control | Changes that are logically independent still produce merge conflicts. |
| Line-by-line review | Humans cannot keep pace with agent output, so review becomes a rubber stamp. |

Existing tools solve parts of this, but the solutions are spread across compilers, editors, linters, test frameworks, documentation generators, profilers and version control. Each works on source files, and none shares a model of what the program means. Solace brings these capabilities together around one model of the program.

## 3. The Proposal

### 3.1 Design principles

1. **Identity independent of text.** Every function, type, contract and other significant element has a permanent identity that does not depend on its name, its position in a file or where it is stored.
2. **Everything explicit.** Dependencies, side effects, state transitions, error conditions, ownership and every input from the outside world are stated where tools can check them.
3. **Deterministic execution.** Given the same inputs and the same responses from the outside world, a program behaves the same way. Time, randomness and I/O are passed in, so tests repeat exactly and any run can be replayed.
4. **Functional by default.** Pure functions, immutable data and algebraic data types are the foundation. Mutable state and side effects are available through explicit, controlled mechanisms.
5. **Verification in every change.** Contracts, generated tests, coverage, mutation testing and performance checks are part of the normal change process.
6. **The system makes meaningless choices.** Where a choice carries no meaning, such as formatting, file organization, which library parses JSON or the spacing in an interface, Solace makes it, so results do not drift between sessions.
7. **Relationships can be queried.** Solace knows how the parts of a program relate and answers questions about them directly, so understanding a program does not depend on text search.
8. **Intent over implementation detail.** Behavior, contracts and interface designs state what is wanted, and the compiler and renderer choose the details within the stated limits.
9. **One model, two views.** Humans and agents work on the same model, with the same information and the same checks. Agents read and write compact text, and humans see contracts, behavior and structure visually. Neither view holds information the other lacks.
10. **Human attention is spent carefully.** Solace asks humans only for decisions that need human judgment, and records every answer so the same question is not asked twice.

### 3.2 Architecture

The store is the single source of truth. Every other component reads from it or writes to it.

```
store           holds identities, revisions, relationships, contracts, metadata,
                verification evidence, decisions, work records, policy, history
agent view     <-> store        read and write
human view     <-> store        read and write
store           -> compiler      types, effects, contracts; under a second, per symbol
store           -> verification  tests, properties, mutations; proven/tested/assumed/unverified
store           -> policy        boundaries, approvals, verification debt
compiler        -> BEAM          actors, supervision, capabilities; effects recorded
compiler        -> browser       JavaScript; state machines and layout
BEAM            -> store         production traces
```

- **Store.** Holds symbols, identities, revisions, relationships, contracts, metadata, verification evidence, decisions, work records, policies and history. It exports to an open, readable text format (section 4.8).
- **Agent view.** Compact text views of symbols, context bundles, and a tool through which agents make checked, all-or-nothing changes (sections 4 and 6).
- **Human view.** A task board, a contract view and review, and later decision trees, replayed scenarios and a queue of questions, with one click to any definition (section 7).
- **Compiler and checker.** Work symbol by symbol: types, effects, boundaries and contracts are rechecked for the changed symbol and its dependents in under a second.
- **Verification engine.** Test generation, property-based tests, mutation testing and performance checks, with the strength of the evidence for each contract recorded (section 8).
- **Policy engine.** Enforces architectural boundaries, approval rules and limits on verification debt for every change (section 9).
- **Runtimes.** The BEAM for services and JavaScript for browser interfaces (section 12). A recorder captures every side effect so any run can be replayed (section 10).
- **Lockdown.** Sorts the project's files into tiers, runs the commands agents are allowed to use, and writes each agent program's configuration so the agent can reach the project only through Solace (section 6.8).
- **Task board.** The channel between humans and agents: tasks, agent runs, questions and approvals, behind a small HTTP API, with a default web board (section 7.1).

Until the Solace language exists, the store wraps a project in an existing language (Gleam) through a language adapter. The adapter supplies symbols, references, type checks, tests and derived facts such as side effects. The graph it produces has a fixed format, so another language can be supported by writing another adapter.

### 3.3 Benefits and drawbacks

**Benefits**

- **Agents cannot bypass the checks.** An agent gets one tool and no shell or file access. Every change goes through Solace's checks, is recorded against its task, and can be reverted.
- **Humans review behavior instead of diffs.** Review shows what a change did to contracts, side effects and tests. Mechanical changes are summarized in one line.
- **Edits do not fail on text matching.** Agents name the symbols they change. A rename rewrites nothing, and an edit that no longer matches the current code is refused before anything runs.
- **Less searching.** One call returns a symbol with its callers, callees, types, tests and open work. A session starts with an index of the project, or with a ranked list of the symbols a task is likely to involve.
- **Knowledge outlives sessions.** Work records, decisions, project instructions and memories are kept with the project and handed to the next agent.
- **Tests are harder to fit to the code.** Expected values are protected, regression tests must fail on the old code, and weakened checks are flagged (section 8.8).
- **Repeatable runs.** Side effects are passed in, so tests are deterministic and any run can be replayed (from Phase 2).
- **No lock-in.** The whole program exports to plain text that can be read without Solace and stored in Git.

**Drawbacks and costs**

- **A language no model has trained on.** Agents have never seen Solace code. The language is kept small and familiar to limit this, and it is built only after the store has proven itself on Gleam.
- **A large system to build.** The store, checker, verification engine, runtimes and two views are a lot of work. Phase checkpoints keep each step small and measured.
- **Agents lose the shell.** An agent can run only the commands Solace runs for it and those humans declare in the settings. A task that needs anything else waits for a human.
- **Code files are not edited by hand.** Solace rewrites code files from the store, so code changes go through Solace.
- **Dependence on outside agent programs.** Solace runs models through programs such as the Codex CLI and Claude Code, whose tools, limits and settings differ.
- **One work run per project at a time**, for now (section 6.9).
- **Some facts are estimates.** Over Gleam, side effects and the reach of a change come from analysis. They are shown as derived facts, never as guarantees.
- **Interface quality still needs a human.** Layout and accessibility can be checked mechanically, but whether an interface is good cannot.

## 4. The Program Store

This section covers how the store identifies symbols, tracks their revisions, and turns code into text and back. Everything else in Solace, including references, history, verification evidence and traces, depends on these rules.

### 4.1 Symbols

Symbols are the unit of identity: functions, types, constructors, state machines, actors, capabilities, interface components, examples, policies and decisions. Each symbol has:

- **An identity:** opaque, assigned by the system, never reused.
- **A qualified name:** unique within its namespace. This is the handle humans and agents use.
- **A revision:** a fingerprint of its normalized content.
- **Metadata:** description, rationale, contracts, history and verification status (section 5).

```
Symbol:
    id: fn:7f3a92c1
    name: physics.calculateBuoyancy
    kind: Function
    revision: sha256:a83f...
    effects: none
```

References between symbols use identities rather than names or file paths. Line numbers exist only within a symbol. Significant expressions inside a symbol get their own stable identifiers, so diagnostics, coverage and attached notes survive ordinary edits.

### 4.2 Names and identities

Humans and agents refer to symbols by qualified name. The tools resolve names to identities, and an unknown or ambiguous name is an error that lists the candidates. Identities appear in exports, history and traces, but nobody ever has to type one, which matters because agents mistype opaque strings.

### 4.3 Identity rules

A symbol's identity follows from the operation performed on it, never from a guess based on how similar two pieces of text look.

| Operation | Result |
| --- | --- |
| Edit a symbol's body, signature or contracts | Same identity, new revision |
| Rewrite a symbol from scratch through its own edit | Same identity, new revision. What the edit targets decides identity, not similarity |
| Rename, or move to another namespace | Same identity, same revision. The name change is recorded in history |
| Extract part of a symbol into a new one | The original keeps its identity. The new symbol gets a new identity, linked "derived from" |
| Merge two symbols | The author names the survivor. The other is retired, linked "merged into", and its references are redirected |
| Copy a symbol | New identity, linked "copied from" |
| Delete a symbol | Refused while references remain. Then retired, never erased, and its identity is never reused |
| Create a symbol with a retired symbol's name | New identity |
| Two branches create equivalent symbols | Distinct identities. The merge flags matching revisions as likely duplicates and offers to unify them |
| Import text edited outside Solace | Identities come from the export's identity headers. A symbol without one is new, with a suggested match that must be confirmed |

### 4.4 Revisions

A revision fingerprint covers a symbol's normalized signature, effects, body, contracts and examples. It leaves out the name, the formatting, prose metadata and the names of local variables (bindings are fingerprinted by position), so a rename or a documentation fix does not invalidate verification.

Verification evidence belongs to revisions. When a revision changes, the evidence for it, and for dependents whose assumptions it touches, becomes stale, and the verification plan (section 8.10) re-establishes it.

The identity spec settles two further questions:

- **Generic instantiations and derived implementations** (section 11.9) get no identities of their own, because they exist only at compile time. Diagnostics, coverage and traces for derived code are attributed to the identity of the type that derives it, with a tag.
- **Significant expressions** are match arms, branches, calls, holes, and contract and example clauses. They are numbered within each revision. A note attached to an expression is carried to the next revision by comparing the two syntax trees. A note that does not map cleanly is marked orphaned and listed in the change's review, never dropped silently.

### 4.5 One text form per symbol

Every symbol has exactly one text form. Agents read and write code as this text, and the store takes care of identities, references and bookkeeping. Text is what agents work in and what a human falls back on in an emergency, so it is fully supported even though the store is the source of truth.

A single formatter defines the text of every revision. It has no options, and every rendering of a revision is byte-for-byte identical, so formatting never shows up in a diff.

Text refers to other symbols by name. When text is submitted, its names are resolved to identities, and the store keeps the identities. A rename therefore rewrites nothing: every view shows the new name the next time it is rendered.

### 4.6 Editing by symbol

The symbol is the unit of identity, but it does not limit how much one call can submit. An agent can submit new text for one symbol or many, a whole namespace, or a new namespace with all its symbols. Splitting the text into symbols is the store's job. In one transaction, the store parses the text, resolves names, adds the imports it needs, runs the fast checks and records the new revisions.

A submission can carry whole symbols, an exact replacement of one passage within a symbol, or a patch in the `apply_patch` format that agents are trained on. A patch names symbols or namespaces rather than files and line numbers. Solace turns it into the whole symbols it changes, so every form goes through the same checks. A patch that does not match the current text is refused before anything runs.

A submission that type-checks but breaks code that depends on it, such as a signature change, is accepted. The broken dependents are tracked and reported, so a large change can be made in steps, and the change cannot be committed while any remain broken. A rejected submission is kept as a draft, which the agent can inspect, patch and resubmit without sending the whole text again. A draft that later edits have overtaken is refused rather than applied on top of them.

### 4.7 Views of several symbols

For reading, the store can render several symbols as one text document: a namespace, a feature, a call path, or the symbols a change touched. Edits to such a view are split into one transaction per symbol and applied all together or not at all.

A namespace view is an outline by default. Each symbol gets one entry with its signature, effects, doc comment and verification status, and no body. Bodies are shown with an explicit flag (`--bodies`) or in a symbol's own view. Agents habitually open whole files, so the view they reach for first should return a table of contents rather than the implementation. From the outline they go straight to the few symbols they need.

### 4.8 Export format

The whole program can be written out as plain text: one block per symbol, each with an identity header, in a fixed order. The export is readable without Solace, stores cleanly in Git, and can be imported again without losing anything.

```
-- @id fn:7f3a92c1 @rev sha256:a83f...
fn calculateBuoyancy(parcel: SmokeParcel, environment: Environment) -> Acceleration
    effects: none
```

## 5. Project Knowledge

Agent sessions start with no memory, so anything a later reader needs to know about a symbol is stored with it as structured metadata: why it exists, what was decided about it, and what is unfinished.

| Metadata | Attached to | Purpose |
| --- | --- | --- |
| Description | Identity | What the symbol is responsible for |
| Rationale | Identity | Why particular design decisions were made |
| Decisions | Identity or namespace | Human answers to agents' questions, with who decided and when |
| Notes | Expression or revision | Local explanations, such as why a bound is unusual or why a formula is written a certain way |
| Relationships | Identity | Links to features, requirements and related symbols |
| Work records | Identity or change | Plans in progress: goal, steps done, steps remaining, blockers |
| History | Identity | Significant changes to implementation and design |
| Verification | Revision | Established guarantees and outstanding debt (section 8) |

Contracts and examples are part of the symbol's signature (section 11.3), because they are enforced rather than merely described.

### 5.1 Intent and implementation

Description, rationale and decisions belong to the identity and carry over from one revision to the next. Notes belong to a specific expression or revision, because they explain one implementation. Metadata can be attached as finely as a single expression, so a local explanation stays next to the code it explains without being mixed into it.

### 5.2 Stale prose

Prose cannot be checked mechanically, so Solace tracks it instead. When a revision changes, the prose attached to the affected symbol and expressions is marked *possibly stale* and listed in the change's review. Before committing, the author confirms or updates it. Project policy decides whether stale prose blocks a commit.

### 5.3 Work records

A work record is an agent's plan, written to the store instead of being lost in a conversation transcript. For example: "Migrating callers of `pricing.quote` to the new signature; 3 of 5 done; `checkout.summary` blocked on decision D-12." Open work records appear in both views. A new session, possibly a different agent, starts with the open records for the symbols it is about to touch, each shown with the context of every symbol it names (section 6.3). A resumed task begins with everything the previous session knew, in one call.

### 5.4 Decisions

When an agent needs human judgment, it raises a question attached to specific symbols, giving the options and their consequences. The human's answer is stored as a decision record. Where possible, it is turned into an enforced policy rule (section 9.3); for example, "never use nested scrolling" becomes a rule the interface checker enforces. Later agents see the decision and must follow it, so the same question is not asked again.

In the current system, an agent asks a question on its task (section 7.1). The task is marked as needing attention, and the agent waits for the answer instead of ending, so its context is not lost. Answering the last open question continues the same agent. Changes to anything humans own, such as the project settings or the project instructions, follow the same path: the agent proposes the change as a question with Approve and Reject buttons. Solace applies the change only if it is approved, and only if the file has not changed since the proposal was made.

### 5.5 Project instructions, memory and skills

Agent runs are isolated from the agent program's own settings, so knowledge that should last beyond one task has to come from the project. Solace provides it the same way to every agent program:

- **Project instructions.** The project's `AGENTS.md` opens every session (section 6.3). Humans own it. Agents read it, and can change it only by proposing a change that a human approves.
- **Memory.** Memories are ordinary files in a project directory named in the settings. A session's brief lists them, one line each, and the agent reads the ones that matter. An agent writes a memory when a human tells it to remember something. Any run may write memory, including read-only runs. Memory edits are not task changes, so they are not reviewed and are not undone when a task is reverted.
- **Skills.** Skills are instruction files in a directory humans own. The brief lists them, and an agent reads one when it needs it. Solace also ships its own skills, the first being the arithmetic skill for working out expected test values (section 8.8).

## 6. How Agents Work

Agents work through one tool with a small number of commands. Each command does a lot and replies in compact text. For an agent this tool is the whole environment, so it is specified as carefully as a public API. Its design targets the ways agents lose time today: finding their way around, edits that fail, lost context and unhelpful errors.

### 6.1 The Solace tool

An agent gets a single tool, `solace`, served over MCP (the Model Context Protocol, the standard way agent programs call outside tools). The tool takes a `command` and optional `text` for code. One call can hold several commands, one per line, which share one reply. The agent has no shell and no file tools, only its own web access. A shell would let it read files without going through Solace, and code passed through shell quoting gets mangled. The same commands are available from the command line for scripts.

### 6.2 Design rules for the tool

- **Fewer calls that each do more.** Every exchange with the tool costs a model turn: one cycle in which the model reads its whole context and replies. Agent programs re-send the whole conversation on every turn, so the number of turns drives cost more than the size of replies. One call should therefore answer a whole question. Commands that take a symbol accept several and return all the answers together.
- **Cheap defaults.** The views agents open out of habit return outlines, and bodies come on request. Benchmark runs showed that changing what a familiar command returns works, while telling agents to use a different command does not reliably change their habits. Improvements therefore go into the replies agents already read, not into new commands they would have to discover.
- **Complete replies within a budget.** A reply prints each signature once and refers back to it. Each agent program has a reply budget set just below the point where that program cuts off tool output. When a reply is over budget, Solace trims it by priority and lists what it left out, with the exact command that shows it. A reply cut off by the agent program would make the agent ask for the same thing again.
- **Replies based on the store, not on the conversation.** Replies are shortened by comparing against the store, for example by not repeating errors already reported for a broken symbol. Solace does not track what it has already sent, because agent programs shorten and summarize their conversations.
- **Text by default.** Output is compact, readable text. JSON is available for tools.
- **Names in, identities kept internal.** Agents never have to type an identity (section 4.2).
- **The same abilities as humans.** Anything a human can do or see in the human view is also available through the tool.

### 6.3 Starting a session

```
solace session start --scope pricing
```

This returns:

- The project's instructions (`AGENTS.md`, section 5.5).
- A short workflow with examples (section 6.5). The full reference is available from `help`, and `help <command>` explains one command. Commands the session cannot use are left out, such as commands reserved for humans and task commands outside a task.
- The language reference version.
- The policy rules that apply to the scope.
- A symbol index: the name of every symbol in scope, grouped by namespace, with the namespace's one-line description. The whole context is re-sent on every turn, so the index is paid for again and again and must stay small. Names alone cost about 3 tokens per symbol (about 1k tokens for a 300-symbol project). Signatures come from `inspect` or from a namespace outline. For large projects, `--scope` narrows the index, and each namespace shows "N more" beyond a limit.
- Open work records for the symbols in scope, each with the context of the symbols it names (section 5.3).
- Decisions made since a session last worked in this scope.

From the index alone, an agent can choose which symbols to look at without opening anything, and a new agent picks up where the last one stopped.

Given the task's text, `session start --task` replaces the index with a task packet of about 2k tokens. The packet lists the symbols most likely to matter, with their signatures and one-line descriptions but never their bodies. Symbols are ranked by fixed rules: first by how well the task's less common words match their names, documentation, code and string literals, then by whether the task names them, then one step outward to their callers, the functions they call, the tests that cover them and the test fixtures those tests use. Where a string literal holds a term from the task, such as a coupon code, the entry quotes the expression around it. A reproduction line names a test that reaches the top symbols and the functions to trace, so the agent can reproduce the reported problem with one probe (section 6.7).

### 6.4 Context in one call

```
solace inspect pricing.quote --context
solace inspect pricing.quote checkout.pay tax.quote --context
```

This returns one text document with the symbol's full text, contracts and effects; its description and rationale; its direct callers and callees with their signatures; the types it uses; its verification status and debt; relevant decisions; and open work records. Given several names, it returns all their bundles in one document, since each extra turn re-sends the agent's growing context. Getting oriented, which today takes a dozen searches and file reads, takes one call after the session starts.

The bundle also answers the questions a change usually raises next. Each caller is shown with the line that makes the call. A type lists the places that construct it, each with its source line, separately from its other users, so a change to the type and to everything that builds it can go in one submission. The smallest test that uses the symbol is shown in full, with the signatures of its helpers, and other tests are named.

### 6.5 Workflow

1. **Inspect** the relevant symbols, contracts and relationships, in one call.
2. **Reproduce** the reported problem with a probe before changing anything, to confirm the old code produces the reported value.
3. **Plan** the change and the verification it needs, and record the plan as a work record.
4. **Change** the code by submitting the tests, then the code they cover, then a test run. One call can carry all three. Code goes in as a patch or as one block per symbol. Tests are written at the level where the task reports the problem, with expected values worked out from the task (section 8.8).
5. **Validate.** Every submission is type-checked on arrival, so a passing test run ends the coding work. Fix only what the reply reports, and read the impact note (section 8.9).
6. **Review** the change in terms of contracts and behavior, along with any stale prose and remaining verification debt.
7. **Commit** the change with its evidence, or ask a question on the task and wait.

Every session starts with this workflow and an example of each step.

### 6.6 Diagnostics

Every error and warning has a stable code, a location given as a symbol and an expression, and a plain explanation. Where the fix is mechanical, a suggested fix is attached. The root cause is reported first, with the errors that follow from it grouped beneath it, so a single mistake produces a single report.

```
E0412 effect not declared
  at pricing.quote, expression #17: clock.now()
  pricing.quote uses Clock.read but declares effects: none
  fix: add Clock.read to effects and a clock: Clock parameter
  affects 3 callers: checkout.summary, cart.preview, api.quote
```

Replies report what changed rather than repeating everything. After a submission, the reply names the broken symbols it fixed, and gives one line for each symbol that is still broken with errors already shown. New errors are printed in full once per message, and repeats of the same message take one line each. A full check prints everything. When a submission fails only because it calls, with the wrong arguments, a function the agent is also changing, the reply says to submit both together.

### 6.7 Probes and tracing

`probe` runs throwaway code against the project and returns its output. It saves nothing and records no change, so read-only runs can use it too. A probe can start from a test's setup and add statements that use the test's values, so reproducing a reported problem takes one call. A probe that is a single expression prints its value. With `--trace`, a probe shows each call to the named functions in order, nested, with their arguments and results. Tracing is for finding causes. It is never a source of expected test values (section 8.8).

### 6.8 Guardrails

Each agent session runs with limited permissions: the namespaces it may change, the side effects its test runs may use, and limits on time and compute. Sessions are isolated from each other and from production. Every operation is recorded in a history tied to the session and its work records.

The lockdown spec builds this for projects in an existing language. Solace sorts the project's files into categories, named for what Solace does with each (D-52):

| Category | Contents | What an agent may do |
| --- | --- | --- |
| Graph | Code, held as symbols | Read and change through the symbol commands only, never as files |
| Watched | Every other text file under a size limit, such as HTML, YAML, SQL, infrastructure definitions, test fixtures, documentation and the project manifest (`gleam.toml`) | Read and write through Solace, checked by any validator declared for that kind of file. The manifest has a built-in validator and can't be removed; later it becomes a typed record changed through commands, with the file generated from it |
| Protected | `solace.toml` and `AGENTS.md`, owned by humans | Read, and propose changes that a human approves on the board (section 5.4) |
| Unwatched | Version-control data, build and dependency output, files ignored by `.gitignore` or listed in the settings, and binary or oversized files | Read and write through Solace. Left out of search |
| Internal | Solace's own data (`.solace/`, including the task board's data) | Nothing; out of reach |

Solace keeps no history for files outside the graph, since history is version control's job. It does keep a content hash for each file, so it can refuse an edit made on top of a change it has not seen. Agents run only the commands Solace runs for them: check, test, build, format, and any commands humans declare in the settings, each with a timeout. Every edit a task makes is recorded and can be reverted, except memory edits (section 5.5). The same goes for changes a command makes to any tier except ignored files. Humans do not edit tier 1 files on disk, because Solace rewrites them from the store whenever it needs to. The human view shows each file's tier and what that tier lets agents do.

Solace runs models through outside agent programs, such as the Codex CLI and Claude Code, and stores no credentials. For each run it writes a separate configuration for the agent program, with no user settings, plugins, memory or other tools. The configuration includes what that program needs to use the Solace tool well, such as its limit on tool output. Investigation and review runs are read-only: they can read, test and probe, but they cannot change the project.

### 6.9 Transactions and parallel work

A change declares which symbols it may modify and the revisions it started from. If any of those revisions changed in the meantime, the change is rejected with the difference, never merged silently. Agents can then work at the same time on independent symbols without text conflicts. Conflicts in meaning are still caught when two changes touch different but related symbols, such as one agent changing a contract while another adds a caller that relies on the old one.

That is the target design. For now, one work run changes a project at a time, and its edits land in the project as they are made. Solace records what each task changed and the state before the task first changed it, so the task can be reverted (section 7.1). Running tasks in parallel will give each task its own open change in the store, merged symbol by symbol when approved, with the conflict checks above. The recorded before-states will then serve as each change's starting revisions. Git stays optional, and committing an approved task to a branch can be added as an integration. Neither step changes the board's API or workflow.

## 7. How Humans Work

The human view shows what the system guarantees and how it behaves, as fast as agents change it, and lets a person get to any level of detail within seconds when something goes wrong. Humans mostly care about contracts and guaranteed behavior, so the view leads with those and keeps the implementation one click away.

The human view is a web app, and nothing a human does in it depends on the command line. It has two parts: the task board, where humans direct agents (section 7.1), and the contract view, where they see and check the system (section 7.2). Humans supply the creative input and the judgment. Agents plan, write code, debug and write tests. A human reviewing an agent's work sees what changed in contracts, side effects and tests rather than a diff of source code.

The task board, the contract view, review and the question queue exist today (sections 7.1 to 7.4). Sections 7.5 to 7.7 describe the full human view planned for Phase 3, which depends on the Solace language's contracts and decision trees.

### 7.1 The task board

Humans direct agents through tasks on a board, and do not need to watch them while they work (specified in `specs/15-board-api.md`). Solace owns each task's state, starts and stops agents, and enforces the workflow. A board only displays tasks and passes on what the human asks for. It talks to Solace through a small HTTP API that Solace defines and that is not modeled on any existing issue tracker. Solace ships a default web board, and another board, or an adapter for one, can replace it. Task data is kept as local files in the project, out of agents' reach.

- **Columns and runs.** By default a task moves from backlog (in priority order), to planning, to assigned, then to blocked or needs review, and finally to complete. Tasks have no type; what a run may do depends on its kind. An *investigate* run in planning explores, asks questions and reports a finding or a plan, and changes nothing. A *work* run in assigned makes the change. A *review* run assesses the task's changes. Only work runs write. A task that only needs an answer ends after planning, with the report as its result. Otherwise the report goes into the work run's brief.
- **Configurable workflow.** The columns, the actions allowed in each, and the approvals a column requires come from the project settings, and Solace ships a default. The complete column requires an approval. Moving a task back out of an approval-gated column by any other route removes the approval.
- **Actions.** From needs review, a human can approve, ask for another review, send the task back to a work run with the review's feedback (keeping the changes), start over (reverting the changes), or cancel. Any task can be paused and resumed. Cancel and complete ask for confirmation first.
- **Questions.** An agent that asks a question waits for the answer, and the task is marked as needing attention. Answering the last open question continues the same agent. A human can instead start a new agent on the task.
- **Models.** The human picks the model and how much reasoning effort it uses when assigning a task. Solace ships presets for the current major models, and the system configuration can add custom endpoints compatible with OpenAI's or Anthropic's API, with keys read from environment variables. A model whose agent program, sign-in or key is missing is shown as unavailable, with the reason, and cannot be assigned.
- **Following a run.** The board shows when an agent is working. A human can follow a run live or read it afterwards: the agent's messages, its reasoning, and each tool call with its input and output. This view is read-only. A finished run is marked as finished until the next action. Notes are written in Markdown and can include images, which agents receive with their brief. Humans can delete notes.
- **Continuity.** The agent that started a task keeps its conversation through investigation, work and fixes after review, as long as the agent program and model stay the same. Otherwise Solace hands over in a brief: the task's changes, new notes and answers, recent outcomes, drafts and the last test results, each marked stale if the project has changed since. Every review starts with a fresh agent.
- **Undo.** Solace is not tied to version control. A task's work lands in the project as it is made. Solace records what the task changed and the state before the task first changed it, so cancel and start over can revert it. A revert is refused, with nothing changed, if a later task has since changed any of the same symbols or files. These records are dropped when the task reaches a final column, so they are not a history.

### 7.2 The contract view

The contract view shows a human what the system does and what it touches, without source code (specified in `specs/16-contract-view.md`). Its facts come from the store through the language adapter: entry points, logic, side effects, tests and files. The system is drawn as an interactive graph. Nodes are colored by kind, and the human can search, expand nodes and follow connections. Facts found by analysis, such as the side effects a function can reach (worked out from a table of known functions with side effects, plus every call into foreign code), are marked as derived and never shown as guarantees. Public functions that nothing else calls are shown as entry points, which covers projects without a `main`.

- **Runs.** A human can run any public function with data they choose, entered through forms built from its types, and can chain runs together. Before a run, the view lists the function's side effects. Effects that reach outside the scratch copy the run uses, such as network access or other processes, need the human's confirmation. Results show as fields, changes to state as before and after, and files open in the browser's own viewers. A run is for understanding and manual checking, and is not saved as a test (section 10.3).
- **Tests** are shown as data in and data out, one line per assertion. The rest of a test is source code, shown on request.
- **Files** are shown with their tier and what that tier lets agents do (section 6.8).

### 7.3 Reviewing a change

A change is reviewed in order of importance:

1. Contract and policy changes, with any weakening flagged, and any flags raised on tests (section 8.8).
2. Behavior changes: altered decision trees, new or removed states, changed examples, replays that now turn out differently.
3. Verification evidence and any new debt, including uses the change reaches that no test checks (section 8.9).
4. Interface changes, rendered at each screen size and in each state.
5. Implementation text, shown only on request.

Mechanical changes that alter no contract, behavior or interface are summarized in one line.

Review is the contract view filtered to one task, linked from the task on the board. The task carries a one-line summary, such as "2 effects added, 1 test unfinished, 3 untested uses". The view lists the effects, signatures, tests and files the task changed, runs the task's tests, and replays the human's earlier runs that reach the change, on the code before and after the task (section 10.3). The tests shown are those the task changed, plus those that reach a changed symbol or read a field the change flows into. Approval is given by the human on the board. Agent reviewers start from the task's diff, which shows each changed item against its state before the task.

### 7.4 Questions waiting for a human

Agents' open questions appear in one queue. Each shows the question, the options and their consequences, and the affected symbols, ordered by how much work each one is holding up. Answers become decision records (section 5.4).

On the task board today, the queue is the set of tasks marked as needing attention. Each shows the reason, such as an agent's question, a proposed settings change or a failed run, until a human acts on it.

### 7.5 System overview and guarantees

The top level is a map of features, their components and the symbols inside them, marked by verification status and recent activity. Beside it is a summary of recent changes written in terms of behavior, such as "refunds now require manager approval above $500", rather than a list of edited symbols.

For each feature, the view lists what the system promises (its contracts, refined types, state-machine rules and policies) with the strength of the evidence for each (section 8.1). Assumptions and verification debt are shown alongside, so it is clear which promises have been checked and which have not.

### 7.6 Decision trees and scenario replay

Every function's branching logic is drawn as a decision tree, from conditions to outcomes. It is generated from the code, so it is always current. State machines are drawn as state diagrams. Selecting a branch shows the examples, tests and recorded scenarios that exercise it, and marks any branch nothing exercises.

Any recording or saved scenario (section 10) can be watched. The path through each decision tree lights up as the run proceeds, messages move between actors, and interface states appear as they were shown. A timeline can be moved forward and back, and any value can be inspected at any point.

The path shown is the program's decisions in business terms, which come from the Solace language's contracts and decision trees. It is not a call-by-call trace, since debugging is the agent's job. Until the language exists, a run shows data in, data out and side effects (section 7.2), and a human who wants to know why asks the agent on the task.

### 7.7 From a failure to its cause

Any node, whether a feature, a branch, a message or an interface element, opens the definition behind it: its text, contracts, description, rationale and history in one place. Navigation follows relationships such as callers, callees, data flow and which component owns which state, never files.

A production failure opens directly onto a path: the failing contract, the symbol and revision, the recording of the failure, and the recent changes to every symbol involved, each with the agent session and work record that made it. A human can go from an alert to an understanding of the cause without knowing where anything lives, and can make a fix directly through the same transactions agents use.

## 8. Verification

A successful build in Solace establishes more than correct syntax and types. For every contract, it states how much is known about whether the contract holds. Verification is the main check on agents' tendency to report success early, so it is designed to be hard to get around.

### 8.1 Evidence levels

| Level | Meaning |
| --- | --- |
| Proven | Shown to hold for all inputs without running the code |
| Tested | Held for every generated, property-based and example input tried. The evidence records how many and of which kinds |
| Assumed | Declared true by a human, typically about an external system, with who assumed it and why |
| Unverified | Nothing establishes it yet. Recorded as verification debt |

Every contract, refined type and state-machine rule has one of these levels, and both views show it.

### 8.2 Contracts

Preconditions, postconditions and invariants are proven without running the code where that is feasible, and otherwise checked at run time and by generated tests. Proof is in scope from the start for a core of questions that a machine can decide: refined types, arithmetic bounds, preconditions and state-machine rules, using an automatic prover (an SMT solver). The language is designed to make this possible, with defined numeric behavior and no shared mutable state.

### 8.3 Generated tests

The compiler analyzes each symbol to find behavior no test exercises: branches, pattern alternatives, boundary conditions, invalid inputs, state transitions, error handling and contract conditions. For example, a comparison against a threshold of 65 produces tests with inputs just below 65, at 65 and just above.

A generated test needs something to tell it what the right answer is. In Solace that comes from contracts, refined types, examples and state-machine rules. Where none of these exist, a generated test can only detect crashes, contract violations and unfinished code. Recording what the code currently does as the expected result is allowed only as an explicitly labeled characterization test, which pins down current behavior to protect a refactoring. It never counts as evidence that the code is correct.

Generated tests cannot show that a specification is right. Their purpose is to expose gaps between what was declared and what was implemented.

### 8.4 Property tests and state exploration

Property-based testing generates many random inputs and searches for one that breaks a declared rule, using generators that respect refined types. A failing input is reduced to its simplest form and added to the symbol as a regression example. State machines and actors are explored up to a stated depth and message count, since exploring every possible state is infeasible for real systems.

### 8.5 Mutation testing

Mutation testing checks whether the existing tests would notice real mistakes. The engine makes small deliberate changes to the code, such as altering a comparison, removing a condition, swapping an operator or changing error handling, and reports any change the tests fail to catch as a weakness in that symbol's tests.

### 8.6 Verification debt

Behavior that cannot yet be verified is recorded as debt on the affected symbol, and is never treated as complete.

```
Verification: INCOMPLETE
Symbol: payments.gateway.submit
Unverified:
    - Timeout recovery
    - Duplicate transaction handling
    - Disconnection during commit
Reason:
    Requires external service integration.
```

Some debt is recorded automatically: unfinished code (holes, section 11.6), unverified contracts and mutations the tests did not catch. Some, like the example above, can only be written by someone who understands the behavior, so the author must list known unverified behavior before committing. Project policy can forbid any increase in debt without a human's approval.

### 8.7 Protecting the checks

Agents under pressure to pass tend to weaken the check instead of fixing the behavior. Solace makes this visible and requires approval for it:

- **Contract changes are their own class of change.** They are shown first in every review and need human approval under the default policy.
- **Weakening is detected.** A removed postcondition, a loosened refined type, a deleted example or a narrowed property is flagged as weakening, separately from changes that strengthen a check.
- **Removing debt needs evidence.** A debt entry disappears only when a proof or tests cover it, or a human accepts it as an assumption.
- **Tests are protected too.** Changing an existing test's expected value is flagged (section 8.8).

### 8.8 Tests written by agents

Until contracts and generated tests exist, agents' own tests are the specification, so Solace stops them from being fitted to the code. A test written after the code tends to assert whatever the code produces. Test-driven development prevents this, but following it step by step costs extra turns. Solace keeps the purpose and enforces it directly:

1. **Admitted versus accepted.** Every submission within a task must parse and type-check. A task is accepted, and can go to review, only when no test fails and no unfinished code is left. Failing tests are allowed inside a task, never in the accepted program.
2. **Tests first.** Tests, with holes as placeholders for new functions, are submitted before the code they cover. The code follows in one batch or several. Solace records each test's result as unfinished, wrong or passing. A test covers the symbols it references.
3. **Expected values are protected.** Changing an existing test's expected value while the function it tests has a body is flagged, in any task and whoever wrote the test.
4. **Bug fixes.** The regression test is linked to the task. Where the task reports a wrong value, the old code must produce it. A new regression test that passes on the old code is flagged.
5. **Declared refactors.** Any edit to a test or to a public signature is flagged. Symbols no test covers need characterization tests first, and these must pass on the old code (section 8.3).
6. **Flags rather than refusals, at first.** The agent that caused a flag can never clear it, and every resolution is recorded. Policy may later turn flags into refusals.
7. **Blind review of flagged values.** The reviewer works out the expected value from the task before seeing the author's change, then compares. Only a disagreement, or a question about these rules, goes to a human.
8. **The arithmetic skill.** Expected values are written as plain arithmetic from the stated inputs, including rounding steps. They are never taken from running the code under test, and never changed to match the output without being worked out again. An assertion whose two sides read the same local value gets a note, because a value taken from the result cannot catch a wrong result.
9. **Tests at the reported level.** The workflow asks for a test at the level where the task reports the problem (a checkout ticket gets a checkout test), and one test for each behavior the change shifts, such as a cap or a boundary.

### 8.9 Reach and impact

One analysis of the reference graph answers two questions: which tests could change their result, and where a change goes.

- **Test reach** is everything a test uses, directly or through other functions. It decides which tests run (section 8.10). "Covered by a test" keeps its narrower meaning (what the test references, including through its own helpers), so a test that runs the whole program does not count as testing everything.
- **Data flow** follows a change from the changed lines: through local variables, into record fields, to callers at the point of the call, to code that reads a changed field, and through helpers that write fields. A use is *certain* when the field belongs to only one type, and *possible* when another type has a field with the same name.
- **Impact.** After each submission, a short note says how many uses the change reaches, which fields it flows into, the nearest certain uses that no test of their own checks, and where data leaves the program. The full list is one command away, with certain uses first and nearest first. Review shows the same facts to the human (section 7.3).

Reach and data flow are derived facts, not guarantees.

### 8.10 When checks run

Not every check runs after every edit. From the dependency graph and the changed revisions, the engine works out a plan:

| Stage | When | What runs | Target time |
| --- | --- | --- | --- |
| Fast check | Every symbol edit | Parsing, types, effects, boundaries, refined types, the symbol's examples | Under 1 second |
| Focused | On request, and before commit | Generated and property tests for changed symbols and their direct dependents | Under 1 minute |
| Full | Before a change is accepted | Mutation testing, state exploration, performance checks on everything affected | As long as needed |

Fast feedback is a hard requirement. While iterating, a sub-second answer about the symbol just edited matters more than a deeper check.

Tests are selected by reach (section 8.9). An ordinary test run runs the tests whose reach has changed since they last passed. It also runs every test that failed or never ran, and every test whose reach includes another process, the network or foreign code, which static analysis cannot see into. Naming symbols runs the tests that reach them, and a full run is always available. Test suites outside the ordinary test runner, such as end-to-end tests, are declared in the project settings. Solace says when a change reaches one, and never runs it unasked.

### 8.11 Performance budgets

Functions and components may declare resource budgets: allocation, memory, message counts, latency and throughput.

Regression checks during ordinary changes use measures that do not vary from run to run: BEAM reduction counts (the VM's unit of work), allocated memory and message counts on representative workloads. Because these do not depend on machine load, a change either stays within budget or it does not. Wall-clock latency and throughput are measured in scheduled benchmark runs that allow for noise, warm-up and hardware differences and compare results statistically against earlier baselines. A regression is traced to the symbols and dependency paths responsible.

## 9. Changes and Policy

Changes are recorded by what they mean, classified by what they affect, and admitted according to project policy. A human is asked to judge only where policy says one must.

### 9.1 Changesets

A changeset records which symbols were added, removed, modified or renamed. It also records changes to contracts, dependencies, effects, decisions and verification status, along with the evidence gathered for it. A symbol's implementation may change while its identity and contract stay the same, and the changeset shows that directly.

Today a task's changes play this role. They are the symbols, module headers and files the task changed, each with its state before the task, and they feed review (section 7.3) and revert (section 7.1).

### 9.2 Classes of change

| Class | Examples | Admitted by default |
| --- | --- | --- |
| Implementation only | Refactoring or optimization; contracts, behavior and interface unchanged; evidence kept | Automatically, once the full verification plan passes |
| Contract strengthened | New postcondition, tighter refined type, added example | Automatically; shown in review |
| Behavior changed within the contracts | Different decision tree, new state, changed scenario replay | Automatically; summarized in the human view |
| Interface change | Layout, component or interaction change | Human review of the rendered states, per policy |
| Contract weakened | Removed postcondition, loosened refined type, deleted example | Human approval |
| More verification debt | New unverified behavior | Human approval |
| Capability granted | A namespace or actor gains access to a side effect | Human approval |
| Stored-type migration | Breaking change to a `stored` or `wire` type | Human approval |
| Policy change | Any edit to project policy | Human approval |

### 9.3 Policy

Project policy is written in Solace and enforced by the policy engine on every change. It is how a human judgment, once made, keeps being applied.

```
policy project {
    require approval for: contract.weaken, debt.increase, capability.grant,
                          migration, policy.change
    forbid: ui depends on storage
    constraint ui: no nested scrolling
    stale prose: blocks commit
    accept after: full verification
}
```

### 9.4 History and version control

The store keeps the full history of every symbol. It exports to the open text format of section 4.8, which is readable without Solace and can be imported again without loss. A program can be archived, inspected and recovered without any hosted service. During early development, Git stores the export, so no new version-control system is needed up front.

Solace does not depend on any version-control system. Version control is the humans' job, and Solace neither requires it nor drives it. Over an existing language, version control stores the source files Solace generates. The task board's data and reports are not versioned for now.

## 10. Recording and Replay

Every side effect passes through a capability (section 11.4), and everything else a program computes is pure. Recording what crossed those capability boundaries is therefore enough to reproduce any run exactly. Replay replaces print-statement debugging for agents, and lets humans watch what the system actually did.

### 10.1 Recording

The runtime records each side effect's request and response at the capability boundary, plus the order in which messages reached each actor. Nothing else needs recording: given those inputs, the pure parts of the program compute the same results again.

### 10.2 Replay and queries

A recording can be replayed locally, stepped forward and backward, and queried. Agents query it as text:

```
solace replay run-4821 --until-contract-failure
solace replay run-4821 --value state --at payments.ledger.apply
solace replay run-4821 --first "result.balance < 0"
solace replay run-4821 --messages SimulationRunner --before crash
```

Humans watch the same recording as an animated scenario (section 7.6).

Until recording exists, agents reproduce a case with probes (section 6.7): throwaway code run against the project, optionally starting from a test's setup. A call tracer shows each call to chosen functions with its arguments and result. A call-by-call trace is a debugging tool, and debugging is the agent's work. Humans see a run's data in, data out and side effects instead (section 7.2).

### 10.3 Scenarios

Any recording can be saved as a named scenario. Replaying it against a new revision shows exactly where behavior changes. It also serves as a demonstration humans can watch, and as a starting point for reproducing a bug.

The runs a human makes in the contract view are remembered the same way. In review, Solace replays the runs that reach what a task changed, on the code before the task and on the current code, and shows what differs. The human judges the difference; it is not reported as a pass or a fail. Human runs are not saved as tests, and there is no test folder owned by humans. A run whose result is wrong becomes a task that carries the input, what the program did and what it should have done, and an agent writes the test.

### 10.4 Recording in production

- **Continuous but bounded.** Production keeps a rolling buffer of recent side effects and saves the full recording around any failure.
- **Sensitive data is controlled by type.** Values of type `Secrets` are never recorded. Types marked `sensitive` are redacted or replaced with tokens according to policy.
- **Retention and sampling** are set by project policy.

### 10.5 Tracing production back to symbols

Every log entry, error and metric carries the identity and revision of the symbol that produced it. A production failure links directly to the symbol, its current revision, the contract involved, its verification evidence and the recording. Traces flow back into the store, so both views show how each symbol behaves in the running system as well as in tests.

## 11. The Solace Language

Solace is a small, strict, functional language in which everything that can go wrong, and everything the outside world supplies, appears in a type or a signature. The syntax below is illustrative; the language core spec (`specs/04-language-core.md`) fixes the syntax and the subset built in Phase 1b.

The language is the most expensive and least novel part of the system, so it is built last in Phase 1 (section 14.1), and only after the store and agent tool have shown a measurable improvement with an existing language. Gleam, the language used until then, already has much of what this section describes: algebraic data types, exhaustive pattern matching, no null, `Result` and `Option`, labeled arguments and no macros. It also compiles to both the BEAM and JavaScript.

### 11.1 Language goals

- **Small and plain.** The complete reference must be short enough for an agent to read in full at the start of a session. A feature that is left out cannot be misused.
- **Familiar syntax.** Braces, `fn`, `match`, angle-bracket generics and `name: Type` annotations, so knowledge of widely used languages carries over from the first day.
- **One way to do each thing.** Explicit, slightly verbose code is preferred over concise, clever code.
- **Signatures are always written.** Types are inferred inside function bodies, but every top-level signature is written out, because signatures are what readers and the human view rely on.

### 11.2 Types

Algebraic data types, records and exhaustive pattern matching are the foundation. There is no null: absence is `Option`, and failure is `Result`. Beyond these, Solace makes it impossible to write a value that has the right shape but the wrong meaning:

```
type AccountId = distinct Int
type Money = Decimal where it >= 0
type Percent = Int where 0 <= it and it <= 100

unit Meters, Seconds
type Acceleration = Float<Meters / Seconds^2>

type WithdrawalError =
    | InsufficientFunds(available: Money)
    | AccountFrozen
```

- **Distinct types** share a representation but never mix: an `AccountId` is not an `OrderId`.
- **Refined types** restrict values with a condition. The condition is checked at compile time where possible and when the value is created otherwise, and it feeds the contract system.
- **Units of measure** are checked at compile time and erased at run time.

### 11.3 Functions and contracts

Functions declare their effects, preconditions, postconditions and examples in their signatures, using ordinary expressions rather than a separate specification language:

```
fn withdraw(account: Account, amount: Money) -> Result<Account, WithdrawalError>
    effects: none
    requires:
        amount > 0
    ensures success:
        result == account with { balance: account.balance - amount }
    ensures failure InsufficientFunds:
        amount > account.balance
    examples:
        withdraw(account: acct(balance: 100), amount: 30) => Ok(acct(balance: 70))
        withdraw(account: acct(balance: 10), amount: 30) => Err(InsufficientFunds(available: 10))
{
    ...
}
```

- **Postconditions on the whole result.** Record-update syntax makes it easy to state the entire result. "The balance went down and nothing else changed" takes one line, instead of a list of fields that quietly leaves some out.
- **Examples run.** Each example is both documentation and a regression test.
- **Argument labels are required** at call sites: `withdraw(account: a, amount: 50)`. A label may be left out only when the argument is a variable with the same name as the label: `withdraw(account, amount: 50)`.
- **Crashes are declared.** A violated contract and an explicit `fail` are the only ways to crash. A function that can reach `fail` lists `Fail` among its effects, and on the BEAM a crash is handled by the supervisor (section 11.12).
- **Contracts are checked from the start.** Where the checker can decide a contract, it is checked at compile time, and otherwise at run time.
- **Errors are explicit.** `try expr` returns early with the error. `expr else value` replaces an error or a missing value with a default, and is allowed only on `Option` and `Result`. Values are never treated as true or false implicitly, and `?` is reserved for holes. A fallback with `else` swallows an error in plain sight: it appears as a branch in the decision tree and in test generation, and policy can limit its use per namespace.

### 11.4 Side effects as capabilities

A function can affect the outside world only through a capability: a value, passed in as an argument, that grants access to one kind of side effect. A function with no capability parameters is pure.

```
fn recordReading(sensor: SensorId, value: Celsius, db: Database, clock: Clock)
        -> Result<Unit, DbError>
    effects: Database.write, Clock.read
{
    ...
}
```

- **Declared and checked.** The author writes the `effects:` line and the compiler checks it: it must list everything the function and the functions it calls use. A declared effect that is never used gets a warning and is compiled out, so callers are not charged with it.
- **Handed out at the entry point.** The program's entry point receives the capabilities for the outside world and hands each component only what it needs.
- **Narrowed.** A capability can be restricted before it is passed on: `fs.readOnly("/data")`, `http.only("api.example.com")`.
- **Replaced in tests.** Tests pass fakes, such as `Clock.fixed(...)` or `Random.seeded(42)`, so every test is deterministic.
- **A security boundary.** A task an agent runs with only `Clock` and `Log` cannot touch the network or the file system, and the compiler can show it.

The standard capabilities are `Clock`, `Random`, `FileSystem`, `Http`, `Database`, `Log`, `Config`, `Secrets`, `Spawn` (for actors) and `Dom` (for browser interfaces).

### 11.5 State machines

Logic that is really a state machine is written as one, so it is never flattened into flags and conditionals:

```
machine Order {
    states: Draft, Submitted, Approved, Shipped, Cancelled
    initial: Draft
    transitions:
        submit:  Draft -> Submitted
        approve: Submitted -> Approved      requires: order.total > 0
        ship:    Approved -> Shipped
        cancel:  Draft | Submitted -> Cancelled
    terminal: Shipped, Cancelled
}
```

States are constructors and may carry data. Phase 1b builds a narrower form, in which states carry no data and transitions have no conditions; whether to widen it is decided during Phase 1b. The compiler checks that every transition is handled, flags unreachable states, and rejects transitions out of final states. The verification engine explores reachable states (section 8.4), and the human view draws every machine as a diagram (section 7.6). Interface interactions (section 12.5) and actor lifecycles (section 11.12) use the same construct.

### 11.6 Typed holes

A `?` marks an unfinished part of a program, called a hole. The rest of the program still type-checks and runs its tests.

```
fn shippingCost(order: Order, zone: Zone) -> Money
    effects: none
{
    match zone {
        Domestic -> order.weight * domesticRate
        International(country) -> ?rate "Flat rate, or a per-country table?"
    }
}
```

The compiler reports each hole's expected type and the values in scope that fit it. A test that reaches a hole fails as *unfinished*, which is reported separately from a wrong result. Every hole is recorded as verification debt. A hole with a question goes to the human question queue, and the answer is recorded as rationale (section 5).

Holes let a program with failing tests still build: tests and placeholders for new functions can be submitted before the code, and every submission still type-checks (section 8.8). Over Gleam, a hole is Gleam's `todo`, which type-checks and whose failure the test runner reports separately from a failed assertion.

### 11.7 Local mutation

Mutable variables are allowed inside a function body as long as they cannot leave it: they cannot be captured, returned or sent. Performance-sensitive loops can be written plainly, and the function is still pure from the outside.

```
fn total(values: List<Float>) -> Float
    effects: none
{
    var sum = 0.0
    for v in values { sum = sum + v }
    sum
}
```

### 11.8 Numbers

- `Int` has arbitrary precision, which the BEAM supports natively. Fixed-size integers check for overflow.
- `Decimal` is exact and is the type for money.
- `Float` is an IEEE 754 double, with defined comparison rules for NaN.
- Integer division declares `divisor != 0` as a precondition, proven where possible and checked otherwise.

No numeric operation has undefined behavior, so a generated test at a numeric boundary always has a defined expected result.

### 11.9 Derived code instead of macros

There are no user-defined macros and no reflection, because both hide behavior. Common boilerplate is derived from a fixed list:

```
type Reading derives Eq, Order, Hash, Show, Json, Generator
```

`Generator` produces random values that respect refined types, for property-based tests.

### 11.10 Versioned types and migrations

Types that are stored or sent between processes are marked `stored` or `wire`. A breaking change to such a type requires a migration from the previous version, and the compiler detects breaking changes from the type's revision history.

```
type Customer stored version 2 {
    id: CustomerId
    name: Text
    email: Option<Email>
}

migrate Customer from 1 (old) {
    Customer(id: old.id, name: old.fullName, email: None)
}
```

### 11.11 Namespaces and boundaries

Namespaces declare what they may depend on and which capabilities they may hold. The compiler enforces these rules, so the architecture is written down once instead of being worked out again in every session.

```
namespace simulation.core
    may depend on: math, simulation.model
    may hold: Clock, Log
```

Interfaces (behavior shared across types) are supported, with each type's implementation declared explicitly. There is no inheritance and no implicit lookup chain to trace.

### 11.12 Actors and concurrency

Concurrency uses actors: processes that own their state and communicate through typed messages. An actor's handler takes the current state and a message and returns the next state, plus commands for the runtime to carry out.

```
actor SimulationRunner {
    state: SimulationState
    receives:
        Step(elapsed: Duration)
        Snapshot(replyTo: Address<SimulationState>)
    holds: clock: Clock, log: Log
}

fn handle(state: SimulationState, message: SimulationRunner.Message)
        -> Next<SimulationState>
    effects: none
```

Because handlers are pure, every step can be tested on its own and replayed exactly. Supervision, restart strategies, timeouts and cancellation are declared rather than written by hand. Browser interfaces use the same state-plus-message model (section 12.2), so the whole system has one model of change.

### 11.13 What Solace leaves out

Inheritance; overloading; implicit conversions; null; exceptions; user-defined macros, reflection and custom operators; global mutable state; and more than one syntax for the same construct. Each omission makes code more predictable to read and harder to write incorrectly.

## 12. Runtimes and Interfaces

Solace compiles to the BEAM for services and to JavaScript for browser interfaces. Pure code compiles to both, so types, contracts and validation logic are written once and shared.

### 12.1 The BEAM

The BEAM fits Solace's model closely: lightweight isolated processes, message passing, supervision trees, native arbitrary-precision integers and mature support for running across machines. The compiler emits Erlang source, compiled by `erlc`, with Core Erlang (the compiler's intermediate language) available if finer control is needed. Emitting Gleam or Elixir instead would put another type checker or a macro system between Solace and the runtime. Solace actors compile to BEAM processes, and supervision declarations compile to supervisors. Existing Erlang and Elixir libraries are reachable through foreign declarations (section 13.3).

The BEAM is slow at sustained number crunching. CPU-heavy code, such as a simulation's inner loop, can use native code through the BEAM's native-function interface. Such code is treated as foreign: its signature and effects are declared, and its behavior is unverified until tests establish it.

### 12.2 The browser

Browser interfaces follow the Elm architecture: the interface has a state, receives messages, updates its state with a pure function, and draws its view from the state. This is the same state-plus-message model Solace actors use (section 11.12). Solace compiles this directly to JavaScript with a small runtime instead of generating Elm code, for three reasons:

- Generating Elm would put a second compiler and type checker between Solace and the browser, with its own error messages to translate.
- Elm's interoperation with JavaScript is deliberately narrow, and the language has not had a release in years.
- Solace needs its own runtime hooks for recording, replay and layout inspection (sections 10 and 12.6), which a generated Elm program could not provide.

### 12.3 Shared code and platform capabilities

A namespace declares which platforms it targets. Pure namespaces target both. Capabilities are specific to a platform: `Dom` exists only in the browser, and `Database` and `FileSystem` only on the BEAM. The compiler rejects code that uses a capability on the wrong platform.

Types marked `wire` get generated serializers and a typed protocol, so messages between browser and server are checked at both ends. Changing a `wire` type requires a migration (section 11.10), which keeps old clients and new servers compatible during a deployment.

### 12.4 Layout by intent

Agents can produce interface markup quickly but cannot see whether the result is coherent, usable or consistent. Solace interfaces therefore declare layout intent, interaction states and design constraints. The system works out the exact geometry, checks it mechanically, and sends the judgments a machine cannot make to a human.

Interfaces describe relationships rather than coordinates and fixed sizes:

```
layout Application {
    Sidebar:
        width: preferred(280)
        collapsible: true
    Content:
        fill: remaining
        priority: primary
    compact:
        Sidebar becomes Drawer
}
```

The layout engine computes the geometry within the declared constraints. Typography, spacing, color, sizing, density and the look of components come from the design system, so agents make no arbitrary visual choices. Low-level styling is still possible through explicit escape hatches, and each use is flagged for human review.

### 12.5 Interaction as state machines

Components declare their states and transitions with the language's state-machine construct (section 11.5). The verification engine explores the reachable states and reports interactions that are invalid, unreachable or lead nowhere. Tests refer to events and component identities, never to screen coordinates or text selectors.

### 12.6 Checking presentation mechanically

The renderer exposes its layout model to the tools, so agents can check geometry, visibility, clipping, contrast, accessibility, focus order, scrolling and behavior at different screen sizes directly, without screenshots.

```
solace ui inspect checkout
solace ui validate checkout
solace ui diff checkout --baseline main
```

Validation reports each broken constraint with the screen sizes and interaction states where it occurs. The first renderer targets HTML and CSS in the browser. Solace does not need its own rendering engine to prove this approach.

### 12.7 Human judgment of interfaces

Mechanical checks cannot show that an interface is good. For that, the human view shows every component at every screen size and in every interaction state, lays alternatives side by side, and highlights what changed since the last approval. The human's choice is recorded as a decision (section 5.4). Where possible it becomes an enforced constraint: "related totals must stay visible while the form scrolls" is then checked on every later change.

## 13. Dependencies, Builds and Interop

A Solace project builds the same way on any machine with one command, and every outside dependency enters through a declared boundary that can be checked. Environment and toolchain problems are one of the largest wastes of agent time today, and this design is meant to remove them.

### 13.1 Dependencies

Solace packages are exported stores, referenced by identity and revision and pinned exactly. Nothing is resolved from version ranges at build time. Updating a dependency is an ordinary change: its contract changes are reviewed and its verification plan runs like any other change.

Over an existing language, dependencies are that language's packages. Agents will see their signatures, read-only, through `inspect` and `find`, without the packages entering the store, so an agent neither reads package source nor guesses at an API. Managing packages is deferred to Phase 6.

### 13.2 Reproducible builds

The toolchain version is pinned in the project, and the toolchain ships as a single executable. A build depends only on the store, the pinned toolchain and the pinned dependencies, so the same inputs always produce the same outputs. Setting up a machine means installing one program.

Today Solace is one program with the task board built in, installed from source. A Gleam project also needs Gleam and Erlang/OTP, and agents need the Codex CLI or Claude Code. Prebuilt releases and fewer separate installs will come later.

### 13.3 Foreign code

Erlang, Elixir, JavaScript and native code are reached through foreign declarations. Each must declare its signature and effects. Foreign code is unverified until tests establish its behavior or a human records an assumption about it. Contracts on a foreign declaration are checked at run time, at the boundary.

```
foreign erlang crypto.hash(algorithm: HashAlgorithm, data: Bytes) -> Bytes
    effects: none
    ensures: length(result) == algorithm.digestSize
```

### 13.4 Standard library

The standard library is broad, with one obvious choice for each common need: collections, text, time and durations, decimal arithmetic, units, JSON, HTTP client and server, database access, logging, testing and generators, and interface components with a design system. Agents never have to choose between competing libraries for a routine task.

## 14. Plan

### 14.1 Phases

Agents build Solace in seven phases, with the first main phase split in two. Each phase ends at a gate: a checkpoint with a measurable criterion, judged by the benchmark (section 15). A phase that misses its gate is revised before the next one begins.

| Phase | Scope | Gate |
| --- | --- | --- |
| 0 Specify | Specs for identity, text views, the agent tool and the language core; benchmark tasks and baselines | Specs accepted; benchmark runs end to end with conventional tools (met Sep 27, 2026) |
| 1a Store and agent tool over Gleam | Store, identities, text views and agent tool wrapped around plain Gleam; lockdown, task board and a first contract view | A measurable improvement over plain files in the cost of getting oriented and in the reliability of edits (met Oct 3, 2026) |
| 1b Language core | Parser, formatter, type and effect checker, Erlang backend, export; replaces Gleam in the store | Agents build and maintain the pilot through the Solace tool alone; typical tasks reach their first edit within 3 model turns (median); export and re-import lose nothing |
| 2 Verification and replay | Contracts checked by an automatic prover, test generation, property tests, mutation testing, verification debt, protection of checks, recording and replay | Bugs planted in the pilot are caught before acceptance; replay reproduces recorded failures |
| 3 Human view | Overview, guarantees, decision trees, scenario replay, question queue, review by behavior | A human traces an injected failure to its cause without navigating files |
| 4 Browser interfaces | JavaScript backend, state-plus-message runtime, layout by intent, interface checks | The pilot's interface passes the checks; recorded design decisions are enforced |
| 5 Actors, performance, production | Supervision, performance budgets, recording in production, tracing production back to symbols | The pilot runs under production-like load with recording on and budgets enforced |
| 6 Collaboration and ecosystem | Many agents changing one project at once, packages, broad interop, history beyond Git | — |

Phase 1 is the first usable system and the central experiment: is a store of symbols, read and written through text views, a better foundation for agents than plain files? The phase is split so that the experiment runs before the language exists. Phase 1a compares the store and agent tool over Gleam against plain Gleam files. Phase 1b, the language, starts only after that comparison shows a measurable improvement.

The guardrails and the human interface are the primary goals (section 1), so within Phase 1a they came ahead of further measurement, in this order: lockdown, the task board, then a basic contract view. The basic contract view is an interactive graph of the system's behavior and footprint, in which a human can search, follow connections and run code with their own data. The full human view remains Phase 3.

### 14.2 Status

As of Oct 3, 2026:

- **Phase 0 is complete.** The four core specs are accepted, and the benchmark runs end to end, comparing agents working through Solace with agents working on plain files.
- **Phase 1a's gate is met.** The store and agent tool over Gleam, lockdown, the task board with its default web board, and a first contract view are built. The benchmark runs to date were judged to show the required improvement. Results are recorded in `WORK.md`.
- **Phase 1b, the language core, is next.** Whether to widen the narrowed state machines (section 11.5) is decided during this phase.

### 14.3 How Solace is built

- **Specs are the contracts.** Each component is built against its written spec, and a spec changes only through a recorded decision.
- **Solace's practices before Solace's tools.** Until the store can hold them, work records and decisions are kept in structured files in the repository (`WORK.md` and `DECISIONS.md`), in the formats of section 5, so they can be imported later.
- **Solace is used to build Solace as early as possible.** From Phase 2, new Solace components are written in Solace where the language allows, and the Phase 3 human view is itself a Solace browser application. Problems agents hit while building Solace are treated as defects in Solace.
- **Toolchain.** The compiler, store and checker are written in Rust and ship as a single executable, for fast checks and simple distribution. Runtime support for the BEAM is written in Erlang. The Phase 1a store and agent tool are also written in Rust.
- **What ships.** Solace, its default board and its contract view ship as a product. The benchmark and the pilot applications are development tools and do not ship.

### 14.4 Specifications

| # | Specification | Needed by |
| --- | --- | --- |
| 1 | Identity and revisions (section 4) | Phase 0 |
| 2 | Text views and export format (section 4) | Phase 0 |
| 3 | Agent tool and diagnostics (section 6) | Phase 0 |
| 4 | Language core: types, functions, effects, holes, state machines (section 11) | Phase 0 |
| 5 | Metadata: descriptions, rationale, decisions, work records (section 5) | Phase 1 |
| 6 | Side-effect runtime, recording and replay (section 10) | Phase 2 |
| 7 | Verification, evidence levels and debt (section 8) | Phase 2 |
| 8 | Classes of change and the policy language (section 9) | Phase 2 |
| 9 | Human view (section 7) | Phase 3 |
| 10 | Interface language and browser runtime (section 12) | Phase 4 |
| 11 | Actors, supervision and performance budgets (sections 11.12 and 8.11) | Phase 5 |
| 12 | Tracing production back to symbols (section 10.5) | Phase 5 |
| 13 | Dependencies, builds, interop and standard library (section 13) | Phase 1, extended in Phase 6 |
| 14 | Lockdown: file tiers, project settings and agent commands (section 6.8) | Phase 1a |
| 15 | Task board (section 7.1) | Phase 1a |
| 16 | Contract view (section 7.2) | Phase 1a |

Specs 1 to 4 were accepted in Phase 0, and Specs 14 to 16 when the guardrails and human interface were brought forward. Specs live in `specs/` and change only through a decision.

### 14.5 The first usable system (Phase 1)

- A small functional language: algebraic data types, records, distinct and refined types, exhaustive matching, `Option` and `Result`, required labels, examples, typed holes and state machines.
- Side effects as capabilities, with test fakes for `Clock`, `Random` and `Log`.
- The store, with permanent identities, revisions and the identity rules of section 4.3.
- One text form per symbol, editing by symbol, and the export format.
- The fast checker with structured diagnostics, meeting the sub-second target.
- The agent tool: session start, context bundles, transactions, work records and questions to humans.
- Examples run as tests, with basic verification reporting and automatic debt for holes.
- The BEAM backend.
- Lockdown, the task board and a basic contract view (Phase 1a).

The interface system, full verification, actors, performance budgets and the full human view follow once this foundation has proven itself. In Phase 1, humans direct agents through the task board and review their changes in the contract view filtered to each task (section 7.3).

### 14.6 Pilot applications

The pilot is a small order-management web service with stored data, a checkout state machine and capabilities for the database and the clock, and later a browser interface and actors. It exercises most of Solace without heavy numeric work. A second pilot, a smoke-plume simulation, follows in Phase 5 to test native numeric code and performance budgets.

The order service exists in two sizes, both written in plain Gleam before the benchmark tasks that use them: a small one of about 2.5k tokens of source code and a large one of about 30k tokens. In the small one, the fixed cost of each call dominates and reading code costs almost nothing, so it cannot show a saving. Having both sizes shows how the results change as a project grows.

## 15. Evaluation

Solace's value has to be shown by measurement, starting in Phase 0 and repeated at every gate. Measurement follows the priorities in section 1: guardrails and the human interface first, efficiency second. The main question is not whether agents write correct code on the first attempt. It is how reliably and efficiently agents deliver, verify, change and maintain working software over time, and how well humans keep up with them.

### 15.1 Benchmark

The benchmark was built in Phase 0 and runs at every gate. It compares Solace with conventional languages and tools on equivalent tasks, with the same models, hardware and budgets. Tasks include new features, bug fixes, refactoring across a codebase, performance work, maintaining an unfamiliar codebase and refining an interface. Agents are evaluated both with no prior exposure to Solace and with the full reference, examples and tools.

Each comparison runs the same agent program and model on the same task twice. One run works on plain files with the agent program's own tools. The other works through Solace alone, with the single Solace tool and no shell or file tools, so that a run that gets stuck shows a gap in Solace rather than falling back to files. The Solace runs use the agent configuration Solace ships, so the benchmark measures what users actually get. Efficiency is measured in tokens; counts of tool calls are used only to explain the token counts. The current tasks include features, planted bugs, refactors that touch many callers, state-machine changes, rule changes that force test updates, and resuming from a work record. Two more kinds of task are planned: building a new module from scratch, which is the building half of the Phase 1b gate, and a task in which an existing expected test value really is wrong, so that legitimate test corrections are measured as well as tampering.

| Measure | What it shows |
| --- | --- |
| Correctness | How often hidden tests pass and contracts are met |
| Autonomy | Human interventions per successful task |
| Efficiency | Tokens, time and model turns per completed task |
| Cost of getting oriented | Model turns before the first edit (the main measure), and tokens before the first edit |
| Debugging | Time from a failure report to a verified fix |
| Regressions | Working behavior broken by later changes |
| Refactoring | Changes that succeed across all the symbols that depend on them |
| Verification | Bugs caught before acceptance; weakened checks caught |
| Continuity | Work correctly resumed by a new session from work records. The plain-file setup gets the same information as a handoff note in the repository, so the comparison tests where the information lives and how it is shown, not whether it exists |
| Documentation | Prose left stale by changes |
| Performance | Resource regressions that went unnoticed |
| Interfaces | Layout, accessibility and interaction bugs |
| Human keep-up | Time for a human to understand a day's changes; how long questions wait for an answer |
| Human experience | Understanding, confidence and preference |

### 15.2 Testing features one at a time

Experiments turn off one feature at a time, such as stable identities, text views, context bundles, side effects as capabilities, structured diagnostics, replay, work records or automated verification, to measure what each contributes. A feature that measurably helps nobody is a candidate for removal, since the system is meant to stay small.

### 15.3 Gates

Each gate in section 14.1 is judged by the benchmark. Getting oriented is measured in model turns before the first edit, since every turn re-sends the whole context and token use mostly follows the number of turns. That makes the claim that orientation takes one call something the benchmark can confirm or refute. If a gate is missed, the phase is revised, and the next phase does not start until the foundation has been shown to work.

## 16. Risks

The largest risk is that agents have never seen the new language. Most of the other risks concern the size of the engineering effort, which the phase gates are meant to control.

| Risk | Mitigation |
| --- | --- |
| Agents have no training exposure to the Solace language | A small language, familiar syntax, a reference short enough to read in one go, clear diagnostics, many examples |
| The store, compiler, verification engine and views are a large undertaking | Components that can be tested independently, phase gates, a narrow first usable system |
| Identity cases not covered by section 4.3 | Specify before Phase 1; treat any identity surprise agents run into as a defect in the spec |
| Exhaustive verification does not scale | Combine proof, bounded exploration and targeted tests, and report what is uncertain |
| Verified code can still meet a wrong requirement | Human approval of contracts, postconditions on the whole result, examples reviewed by humans |
| Agents weaken checks | Contract changes as a class that needs approval; detection of weakening; evidence required to clear debt |
| Agents fit tests to their code | Tests before code, protected expected values, regression tests that must fail on the old code, blind review of flagged values (section 8.8) |
| Agents work around Solace | No shell or file tools; file tiers; a separate configuration for each agent run; settings and instructions owned by humans (section 6.8) |
| Dependence on outside agent programs whose tools, limits and configuration differ | Solace writes each program's configuration itself, and the benchmark measures that shipped configuration |
| The interim language (Gleam) shapes the design | Designs kept language-neutral wherever possible; Gleam-specific rules marked as such; a language adapter with a fixed graph format |
| Only one work run per project at a time | Before-state records that become starting revisions once tasks are isolated in the store (section 6.9) |
| Interface quality cannot be fully checked by machine | Human judgment through side-by-side review; decisions recorded as enforced constraints |
| Slow numeric code on the BEAM | Native code declared as foreign; the simulation pilot in Phase 5 |
| Recording costs time and may capture sensitive data | Bounded buffers, sampling, redaction by type, secrets never recorded |
| The human view falls behind agent speed | Summaries of behavior, review ordered by importance, mechanical changes collapsed |
| No mature library ecosystem | A broad standard library; Erlang, Elixir and JavaScript reachable through foreign declarations |
| Dependence on Solace itself | An open text export that Git can store; no hosted service required |

## 17. Future Work and Open Questions

Once the foundation is in place, several further capabilities become possible. None of them is required for the phases above.

- **Understanding architecture.** Architecture documentation generated by the compiler, detection of abstractions that are no longer used, and analysis of how behavior changes across revisions.
- **Accumulated evidence.** As verification evidence and production traces build up, later agents can tell well-established properties apart from assumptions that need checking again.
- **More renderers.** Interfaces that keep one specification while adapting to desktop, mobile and other platforms.
- **Teams of agents.** Many agents working on separate parts of a program at once, with a coordinator checking their combined changes.

Ideas raised but not yet decided:

- Reviews that give a verdict and structured findings a human can turn into subtasks.
- A separate test-writing run with a fresh context, if the benchmark shows it catches more bugs.
- Running a fix's new tests against the old code, with a report on the branches and boundaries of the changed lines.
- A score for agents' own tests, such as how many deliberately broken versions of a reference solution they catch.
- Asking a working agent side questions without disturbing it.
- Access to other tool servers for agents through Solace, which would log each call and refuse it in read-only runs unless a human marks the server safe.
