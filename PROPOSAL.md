# Project Proposal: Solace — A Semantic Programming System

Sep 27, 2026 · @Aaron

**Working title:** Solace — a semantic programming system for humans and agents

**Status:** Design proposal, revision 2 (supersedes the "Axiom" initial concept proposal)

**Purpose:** Design and build a programming language and environment in which agents do the construction and humans supply intent, judgment and perception — with the system removing the churn that wastes each of their time.

## 1. Executive Summary

Solace is a programming system where agents build software and humans direct it, each through a view designed for what they do best. Modern languages and tools were designed around humans writing and organizing text: files, directories, names and line numbers, maintained through textual editing, documentation, testing and code review. Agents have inherited these conventions despite having fundamentally different strengths and limitations.

Agents generate and transform code quickly, reason across many dependencies, and correct themselves using automated feedback. They are weak at noticing what they cannot see: implicit behavior, hidden inputs, undiscovered dependencies, and graphical output. They start each session without memory, and they tend to declare success early. Humans are the reverse: slow to produce code, but they hold intent, judge quality through their senses, and decide what "correct" means.

Solace is built on one premise:

> A program should be a structured, verifiable semantic model, rather than a collection of source files.

Its components have persistent identities, explicit relationships, mechanically enforced contracts and discoverable metadata. Source text is one projection of that model, not its organizing structure. Agents read and write that text one symbol at a time; humans see contracts, behavior, decision trees and replayed scenarios, and can drop into any definition when they need to.

The environment combines a small functional language with effects as capabilities, typed holes and first-class state machines; semantic navigation; transactional editing; integrated verification; deterministic record and replay; and a declarative interface system. It compiles to the BEAM for services and to JavaScript for browser interfaces.

The objective is not to eliminate human engineering or guarantee perfect software. It is to substantially increase how much software agents can build, verify and maintain reliably, while keeping humans in control of intent at the speed agents work.

## 2. Roles: Humans and Agents

Agents do the building; humans supply requirements, perception and discernment. Every part of Solace is designed around that division, and around eliminating the time each side currently wastes.

|  | Agents | Humans |
| --- | --- | --- |
| Strong at | Producing and transforming code quickly; reading large amounts of it; following explicit specifications and conventions; writing tests when told what to test | Stating intent and priorities; judging quality through their senses; deciding what "correct" means; weighing tradeoffs; handling novel emergencies |
| Weak at | Seeing implicit behavior and hidden inputs; judging visual output; remembering across sessions; knowing when they are truly done; resisting the urge to weaken a failing check | Keeping up with the volume and speed of agent changes; reviewing large line-level diffs; holding a whole system in their head |
| Time wasted today | Orientation (search, open, read, repeat); environment and toolchain setup; flaky tests; debugging by print statements; editing failures on text matching; re-deciding conventions each session | Reading diffs full of mechanical changes to find the few decisions that matter; navigating files to understand behavior; re-explaining decisions that were never recorded |
| What Solace gives them | Symbol-level text views, context in one call, explicit effects, deterministic replay, fast checks, persistent work records, and no arbitrary decisions | Contracts and guaranteed behavior, visual decision trees, replayed scenarios, a decision queue, and one-click descent into any definition |

The guiding rule: route human attention only to what only humans can judge, and give agents everything they cannot see for themselves.

## 3. Problem Statement

Contemporary development imposes limitations that become acute when agents do the building and humans must keep up.

| Problem | Consequence |
| --- | --- |
| File-oriented organization | Logical behavior is fragmented across unrelated physical locations. |
| Textual code identity | Refactoring disrupts references, documentation and modification history. |
| Implicit state, side effects and hidden inputs | Agents must reconstruct invisible dependencies: configuration, environment, time, global state, ordering. |
| Nondeterministic execution | Flaky tests make real failures indistinguishable from noise. |
| Debugging without history | Agents fall back on print statements and repeated reruns to learn what a program did. |
| Weak verification guarantees | Incorrect implementations compile and pass incomplete test suites. |
| Editable checks | Agents under pressure weaken tests or assertions instead of fixing behavior. |
| Disconnected documentation | Rationale and design decisions become stale or undiscoverable. |
| No memory between sessions | Each session re-derives conventions and loses in-progress plans. |
| Arbitrary decisions | Formatting, organization, library choice and visual styling vary between sessions. |
| Environment and toolchain churn | Builds depend on machine state; setup consumes agent time. |
| Separate performance tooling | Functional changes silently introduce resource regressions. |
| Screenshot-based UI testing | Visual and interaction regressions are difficult to identify reliably. |
| Text-based version control | Independent semantic changes produce incidental merge conflicts. |
| Line-level review | Humans cannot keep pace with agent output, so review becomes a rubber stamp. |

Existing tools address parts of these problems, but their answers are spread across compilers, editors, linters, test frameworks, documentation generators, profilers and version control. They operate on source files rather than a shared model of what the program means. Solace integrates these capabilities around one semantic model.

## 4. Foundational Design Principles

Ten principles govern every design decision in Solace.

**4.1 Semantic identity over textual location.** Every function, type, contract, event and other significant element has a persistent identity independent of its name, source position or storage location.

**4.2 Explicitness over implicit behavior.** Dependencies, side effects, state transitions, error conditions, ownership and every input from the outside world are represented explicitly.

**4.3 Determinism by construction.** Given the same inputs and the same effect responses, a program behaves identically. Time, randomness and I/O are supplied from outside, so tests are repeatable and executions can be replayed.

**4.4 Functional programming by default.** Pure functions, immutable data, algebraic data types and composable transformations form the foundation. Mutable state and external effects remain available through explicit, controlled mechanisms.

**4.5 Verification as part of development.** Contracts, test generation, coverage, mutation testing and performance validation belong to the ordinary change lifecycle, not an optional afterthought.

**4.6 No arbitrary decisions.** Wherever a choice carries no meaning — formatting, file organization, which library parses JSON, spacing in an interface — the system makes it. Consistency comes from the system, not from the memory of whoever edited last.

**4.7 Discoverability by construction.** The environment understands the relationships between program elements and exposes them directly. Understanding an unfamiliar program does not depend on textual search.

**4.8 Intent over implementation detail.** Behavior, contracts and interface design express intent, leaving compilers and renderers to choose implementation details within declared constraints.

**4.9 One model, separate views.** Humans and agents operate on the same semantic model with access to the same information and verification. Each gets a view shaped for how they work: agents read and write compact text; humans see contracts, behavior and structure visually. Neither view holds privileged information.

**4.10 Human attention is the scarcest resource.** The system spends human attention only on decisions that require human judgment, and records every such decision so it is never asked twice.

## 5. System Architecture

Solace has one source of truth, the semantic store; every other component reads from it or writes to it.

```
semantic-store  holds identities, revisions, relationship graph, contracts, metadata,
                verification evidence, decisions, work records, policy, history
agent-view     <-> semantic-store   read and write
human-view     <-> semantic-store   read and write
semantic-store  -> compiler         types, effects, contracts; sub-second, per symbol
semantic-store  -> verification     tests, properties, mutation; proven/tested/assumed/unverified
semantic-store  -> policy           boundaries, approvals, verification debt
compiler        -> beam             actors, supervision, capabilities; effects recorded
compiler        -> browser          JavaScript; state machines and semantic layout
beam            -> semantic-store   production traces
```

The compiler turns the store into BEAM services and browser interfaces, and production traces flow back to the symbols that produced them.

- **Semantic store.** Holds symbols, identities, revisions, relationships, contracts, metadata, verification evidence, decisions, work records, policies and semantic history. It exports deterministically to an open, human-readable format (section 16).
- **Agent view.** Compact text projections of symbols, context bundles and a structured protocol for transactional changes (sections 7 and 14).
- **Human view.** Contracts, behavior, decision trees, replayed scenarios and a decision queue, with descent into any definition (section 15).
- **Compiler and checker.** Incremental and symbol-scoped: types, effects, boundaries and contracts are rechecked for the changed symbol and its dependents in under a second.
- **Verification engine.** Test generation, property search, mutation testing and performance checks, with each property's evidence level recorded (section 10).
- **Policy engine.** Enforces architectural boundaries, approval requirements and verification-debt limits on every change (section 16).
- **Runtimes.** BEAM for services and JavaScript for browser interfaces (section 12). A recorder captures every effect so any execution can be replayed (section 11).

## 6. Identity and Revision Model

Every symbol has one stable identity and a sequence of revisions; identity follows the operation that was performed, never a guess based on textual similarity. Everything else in Solace — references, history, verification evidence, traces — depends on these rules.

### 6.1 Symbols

Symbols are the unit of identity: functions, types, constructors, state machines, actors, capabilities, interface components, examples, policies and decisions. Each symbol has:

- **Identity:** opaque, system-assigned, never reused.
- **Qualified name:** unique within its namespace; the handle humans and agents use.
- **Revision:** a fingerprint of its normalized content.
- **Metadata:** description, rationale, contracts, history and verification status (section 9).

```
Symbol:
    id: fn:7f3a92c1
    name: physics.calculateBuoyancy
    kind: Function
    revision: sha256:a83f...
    effects: none
```

References between symbols use identities, not names or paths. Line numbers exist only within a symbol. Significant expressions receive stable internal identifiers, so diagnostics, coverage and attached notes survive ordinary edits.

### 6.2 Names are the handle; identities are internal

Humans and agents refer to symbols by qualified name. The tools resolve names to identities and fail loudly on an ambiguous or unknown name, listing the candidates. Identities appear in exports, history and traces, but nobody is required to type one; agents in particular mistype opaque strings.

### 6.3 Identity rules

| Operation | Result |
| --- | --- |
| Edit a symbol's body, signature or contracts | Same identity, new revision |
| Rewrite a symbol from scratch through its own edit | Same identity, new revision; the edit target decides identity, not similarity |
| Rename or move to another namespace | Same identity, same revision; the name change is recorded in history |
| Extract part of a symbol into a new one | Original keeps its identity; the new symbol gets a new identity linked "derived from" |
| Merge two symbols | The author names the survivor; the other is retired, linked "merged into", and its references are redirected |
| Copy a symbol | New identity linked "copied from" |
| Delete a symbol | Refused while references remain; then retired, never erased, identity never reused |
| Create a symbol with a retired symbol's name | New identity |
| Two branches create equivalent symbols | Distinct identities; the merge flags matching revisions as likely duplicates and offers to unify them |
| Import text edited outside Solace | Identities come from the export's identity headers; a symbol without one is new, with a suggested match that must be confirmed |

### 6.4 Revisions

A revision fingerprint covers a symbol's normalized signature, effects, body, contracts and examples. It excludes the name, formatting and prose metadata, so a rename or a documentation fix does not invalidate verification.

Verification evidence attaches to revisions. When a revision changes, evidence for it and for dependents whose assumptions it touches becomes stale, and the verification plan (section 10.8) re-establishes it.

### 6.5 Open questions

- Do generic instantiations and derived implementations (section 8.9) get identities of their own, or inherit their definition's?
- Which expressions are "significant" enough to receive internal identifiers: every branch and call, or only those with attached notes, diagnostics or coverage?

## 7. Text Projection

Every symbol has exactly one canonical text form; agents read and write that text one symbol at a time, and the store handles identity, references and bookkeeping. Text is the agent's native medium and the human's emergency medium, so it stays first-class even though it is not the source of truth.

### 7.1 One canonical form

A single formatter defines the text of every revision. There are no formatting options, and every projection of a revision is byte-identical. Formatting never appears in a diff.

### 7.2 Symbol-scoped editing

The unit of editing is a symbol. An agent submits new text for an existing symbol, or text for a new symbol in a namespace. The store parses it, resolves names, runs the fast checks and records a new revision in one transaction. There is no find-and-replace inside files, so edits never fail because of whitespace or a match in the wrong place.

### 7.3 Names in text

Text refers to other symbols by name. On submission, names resolve to identities; the stored program holds identities. A rename therefore rewrites nothing: every projection shows the new name the next time it is rendered.

### 7.4 Multi-symbol views

For reading, the store can project several symbols as one text document: a namespace, a feature, a call path, or the symbols touched by a change. Edits to such a view are split into per-symbol transactions and applied atomically.

A namespace view is an outline by default. Each symbol gets one entry: its signature, effects, one-line description and verification status, with no body. Bodies come with an explicit flag (`--bodies`) or from symbol views. Agents habitually open whole units, so the view they reach for first should return a table of contents, not the implementation. From the outline they go straight to the few symbols they need.

### 7.5 Export format

The whole program serializes to plain text: one block per symbol, each with an identity header, in a deterministic order. The export is readable without Solace, stores cleanly in Git, and re-imports without loss.

```
-- @id fn:7f3a92c1 @rev sha256:a83f...
fn calculateBuoyancy(parcel: SmokeParcel, environment: Environment) -> Acceleration
    effects: none
```

## 8. The Solace Language

Solace is a small, strict, functional language in which everything that can go wrong, and everything the outside world supplies, appears in a type or a signature. The syntax below is illustrative; the principles are not.

### 8.1 Language goals

- **Small and boring.** The complete reference must be short enough for an agent to read in full at the start of a session. Every feature left out is one nobody can misuse.
- **Familiar syntax.** Braces, `fn`, `match`, angle-bracket generics and `name: Type` annotations, so knowledge of widely written languages transfers on day one.
- **One way to do each thing.** Explicit and slightly verbose beats concise and clever.
- **Signatures are always written.** Types are inferred inside bodies, but every top-level signature is explicit, because signatures are what readers and the human view rely on.

### 8.2 Types

Algebraic data types, records and exhaustive pattern matching are the foundation. There is no null: absence is `Option`, failure is `Result`. Beyond these, Solace makes "right shape, wrong meaning" errors impossible to write:

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
- **Refinements** restrict values with a predicate. They are checked statically where possible and at construction otherwise, and they feed the contract system.
- **Units of measure** are checked at compile time and erased at runtime.

### 8.3 Functions and contracts

Functions declare effects, preconditions, postconditions and examples in their signatures, using the ordinary expression language rather than a separate specification language:

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

- **Whole-result postconditions.** Record-update syntax makes it cheap to state the entire result. "The balance decreased and nothing else changed" is one line, instead of a list of fields that silently omits the rest.
- **Examples are executable.** Each example is documentation and a regression test at once.
- **Argument labels are required** at call sites: `withdraw(account: a, amount: 50)`. A label may be omitted only when the argument is a variable with the label's name: `withdraw(account, amount: 50)`.
- **Unrecoverable failure is declared.** Violated contracts and explicit `fail` are the only ways to crash. A function that can reach `fail` lists `Fail` among its effects, and on the BEAM a crash is handled by the supervisor (section 8.12).

### 8.4 Effects as capabilities

A function can affect the outside world only through a capability value it was given. A function with no capability parameters is pure.

```
fn recordReading(sensor: SensorId, value: Celsius, db: Database, clock: Clock)
        -> Result<Unit, DbError>
    effects: Database.write, Clock.read
{
    ...
}
```

- **Declared and checked.** The `effects:` line is written by the author and verified by the compiler: it must list exactly what the function and its callees use.
- **Granted at the entry point.** The program's entry point receives the world's capabilities and hands out only what each component needs.
- **Attenuated.** A capability can be narrowed before it is passed on: `fs.readOnly("/data")`, `http.only("api.example.com")`.
- **Replaced in tests.** Tests pass fakes, such as `Clock.fixed(...)` or `Random.seeded(42)`, so every test is deterministic by construction.
- **A security boundary.** An agent-run task given only `Clock` and `Log` provably cannot touch the network or the file system.

The standard capabilities are `Clock`, `Random`, `FileSystem`, `Http`, `Database`, `Log`, `Config`, `Secrets`, `Spawn` (actors) and `Dom` (browser interfaces).

### 8.5 State machines

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

States are constructors and may carry data. The compiler checks that every transition is handled, flags unreachable states, and rejects transitions out of terminal states. The verification engine explores reachable states (section 10), and the human view renders every machine as a diagram (section 15). Interface interaction models (section 13) and actor lifecycles (section 8.12) use the same construct.

### 8.6 Typed holes

A `?` marks an unfinished part of a program. The rest of the program still type-checks and runs its tests.

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

The compiler reports each hole's expected type and the values in scope that fit it. A test that reaches a hole fails as *unfinished*, distinct from a wrong result. Every hole is automatically recorded as verification debt. A hole with a question is routed to the human decision queue, and the answer is recorded as rationale (section 9).

### 8.7 Local mutation

Mutable variables are allowed inside a function body when they cannot escape it: they cannot be captured, returned or sent. Performance-sensitive loops need no contortions, and the function remains pure from the outside.

```
fn total(values: List<Float>) -> Float
    effects: none
{
    var sum = 0.0
    for v in values { sum = sum + v }
    sum
}
```

### 8.8 Numerics

- `Int` is arbitrary precision, which the BEAM supports natively; sized integers check for overflow.
- `Decimal` is exact and is the type for money.
- `Float` is IEEE 754 double precision with defined comparison rules for NaN.
- Integer division declares `divisor != 0` as a precondition, proven statically where possible and checked otherwise.

No numeric operation has undefined behavior, so generated tests at numeric boundaries always have a defined expected outcome.

### 8.9 Derivation instead of metaprogramming

There are no user-defined macros and no reflection; both hide behavior. Common boilerplate is derived from a fixed set:

```
type Reading derives Eq, Order, Hash, Show, Json, Generator
```

`Generator` produces random values that respect refinements, for property-based tests.

### 8.10 Versioned types and migrations

Types that are persisted or cross a process boundary are marked `stored` or `wire`. A breaking change to such a type requires a migration from the previous version, and the compiler detects breaking changes from the type's revision history.

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

### 8.11 Namespaces and boundaries

Namespaces declare what they may depend on and which capabilities they may hold. The compiler enforces these rules, so architecture is written down once rather than rediscovered each session.

```
namespace simulation.core
    may depend on: math, simulation.model
    may hold: Clock, Log
```

Interfaces (shared behavior across types) are supported, with implementations declared explicitly per type. There is no inheritance and no implicit resolution chain to trace.

### 8.12 Actors and concurrency

Concurrency uses actors that own their state and communicate through typed messages. An actor's handler is a transition: it takes the current state and a message and returns the next state plus commands for the runtime to perform.

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

Because handlers are pure, every step is testable in isolation and replayable exactly. Supervision, restart strategies, timeouts and cancellation are declared rather than hand-written. Browser interfaces use the same state-plus-message model (section 13), so there is one model of change across the whole system.

### 8.13 What Solace leaves out

Inheritance; overloading; implicit conversions; null; exceptions; user-defined macros, reflection and custom operators; global mutable state; and multiple syntaxes for the same construct. Each omission makes code more predictable to read and harder to write incorrectly.

## 9. Structured Metadata

Everything a future reader needs to know about a symbol — why it exists, what was decided, what is unfinished — is stored as structured metadata attached to its identity. This is how knowledge survives between agent sessions that start with no memory.

| Metadata | Attached to | Purpose |
| --- | --- | --- |
| Description | Identity | The symbol's responsibility |
| Rationale | Identity | Why particular design decisions were made |
| Decisions | Identity or namespace | Human answers to decision requests, with who decided and when |
| Notes | Expression or revision | Local explanations: why a bound is unusual, why a formula is written a particular way |
| Relationships | Identity | Links to features, requirements and related symbols |
| Work records | Identity or change | In-progress plans: goal, steps done, steps remaining, blockers |
| History | Identity | Significant implementation and design changes |
| Verification | Revision | Established guarantees and outstanding debt (section 10) |

Contracts and examples live in the symbol's signature (section 8.3) because they are enforced, not just described.

### 9.1 Intent versus implementation

Description, rationale and decisions attach to identity and persist across revisions. Notes attach to a specific expression or revision, because they explain one implementation. Metadata can attach at any granularity, down to a single expression, so local explanations stay next to the code they explain without being mixed into it.

### 9.2 Staleness

Prose cannot be verified mechanically, so Solace tracks it instead. When a revision changes, prose attached to the affected symbol and expressions is marked *possibly stale* and listed in the change's review. Before commit, the author confirms or updates it; project policy decides whether stale prose blocks the commit.

### 9.3 Work records

A work record is an agent's plan written to the store rather than lost in a conversation transcript. For example: "Migrating callers of `pricing.quote` to the new signature; 3 of 5 done; `checkout.summary` blocked on decision D-12." Open work records appear in both views. A new session, possibly a different agent, starts with the open records for the symbols it is about to touch. Session start shows each open record expanded: the record itself plus the context bundle of every symbol it names (section 14.2). A resumed task begins with everything the previous session knew, in one call.

### 9.4 Decisions

When an agent needs human judgment, it raises a decision request attached to specific symbols: the question, the options and their consequences. The human's answer is stored as a decision record. Where possible it compiles into an enforceable policy (section 16). For example, "never use nested scrolling" becomes a rule the interface validator checks. Future agents see the decision and must respect it, so the same question is never asked twice.

### 9.5 Access

```
solace inspect pricing.quote
solace docs pricing.quote
solace rationale pricing.quote
solace history pricing.quote
solace work --open
solace decisions --pending
```

## 10. Integrated Verification

In Solace a successful build establishes far more than syntax and types: it states, for every contract, how much is known about whether it holds. Verification is the check on agents' tendency to declare success early, so it is designed to be hard to bypass.

### 10.1 Evidence levels

| Level | Meaning |
| --- | --- |
| Proven | Established statically for all inputs |
| Tested | Held for every generated, property-based and example input tried; the evidence records how many and which kinds |
| Assumed | Declared true by a human, typically about an external system; recorded with who assumed it and why |
| Unverified | Nothing yet establishes it; recorded as verification debt |

Every contract, refinement and state-machine rule carries one of these levels, and both views display it.

### 10.2 Executable contracts

Preconditions, postconditions and invariants are proven statically where feasible and checked at runtime and by generated tests otherwise. Static proof is in scope from the start for a decidable core: refinements, arithmetic bounds, preconditions and state-machine rules, using an automated solver. The language is designed to make this possible, with defined numeric semantics and no shared mutable state.

### 10.3 Test generation

The compiler analyzes each symbol to find behavior that no test exercises: branches, pattern alternatives, boundary conditions, invalid inputs, state transitions, error propagation and contract constraints. A comparison against a threshold of 65 generates inputs just below, at and just above 65.

Generated tests need an oracle, something that says what the right answer is. In Solace the oracles are contracts, refinements, examples and state-machine rules. Where none exist, a generated test can only detect crashes, contract violations and unfinished holes. Recording current behavior as "expected" is allowed only as an explicitly labeled characterization test, used to protect a refactoring; it never counts as evidence of correctness.

Generated tests cannot show that a specification is right. Their purpose is to expose gaps between an implementation and what was declared.

### 10.4 Properties and state exploration

Property-based testing searches for counterexamples to declared invariants, using derived generators that respect refinements. A counterexample is minimized and added to the symbol as a regression example. State machines and actors are explored within stated bounds of depth and message count, since exhaustive exploration is infeasible for real systems.

### 10.5 Mutation testing

The engine checks whether existing tests can detect meaningful errors. It introduces controlled changes — altered comparisons, removed conditions, swapped operators, modified error handling — and reports any mutation the tests fail to catch as a verification weakness on that symbol.

### 10.6 Verification debt

Behavior that cannot yet be verified is recorded as explicit debt on the affected symbol, never silently treated as complete.

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

Some debt is recorded automatically: typed holes, unverified contracts and surviving mutations. Some, like the example above, can only be written by whoever understands the behavior, so the author is required to enumerate known unverified behavior before commit. Project policy can prohibit any increase in debt without human authorization.

### 10.7 Protecting the checks

Agents under pressure to pass are prone to weakening the check instead of fixing the behavior. Solace makes that visible and gated:

- **Contract changes are a separate change class.** They are shown first in every review and require human approval under default policy.
- **Weakening is detected.** A removed postcondition, a loosened refinement, a deleted example or a narrowed property is flagged as weakening, distinct from strengthening.
- **Debt removal needs evidence.** A debt entry disappears only when proof or test evidence covers it, or a human accepts it as an assumption.

### 10.8 Incremental verification plan

Not every technique runs after every edit. Using the dependency graph and the changed revisions, the engine computes a plan:

| Stage | When | What runs | Target time |
| --- | --- | --- | --- |
| Fast check | Every symbol edit | Parse, types, effects, boundaries, refinements, the symbol's examples | Under 1 second |
| Focused | On request, and before commit | Generated and property tests for changed symbols and direct dependents | Under 1 minute |
| Full | Before a change is accepted | Mutation testing, state exploration, performance checks on everything affected | As long as needed |

Fast feedback is a hard requirement: during iteration, a sub-second answer on the symbol just edited matters more than depth.

### 10.9 Performance contracts

Functions and components may declare resource budgets: allocation, memory, message counts, latency and throughput.

Regression checks in the ordinary change lifecycle use deterministic measures: BEAM reduction counts, allocated memory and message counts on representative workloads. These do not vary with machine load, so a change either stays within budget or it does not. Wall-clock latency and throughput are measured in scheduled benchmark runs that account for noise, warm-up and hardware differences, with statistical comparison against baselines. Regressions are attributed to the responsible symbols and dependency paths.

## 11. Execution, Replay and Observability

Because every effect passes through a capability and every other computation is pure, recording what crossed the capability boundaries is enough to reproduce any execution exactly. Replay replaces print-statement debugging for agents and gives humans a way to watch what the system actually did.

### 11.1 Recording

The runtime records each effect's request and response at the capability boundary, plus the order in which messages reached each actor. Nothing else needs recording: given those inputs, the pure parts of the program recompute identically.

### 11.2 Replay and queries

A recording can be replayed locally, stepped forward and backward, and queried. Agents query it as text:

```
solace replay run-4821 --until-contract-failure
solace replay run-4821 --value state --at payments.ledger.apply
solace replay run-4821 --first "result.balance < 0"
solace replay run-4821 --messages SimulationRunner --before crash
```

Humans watch the same recording as an animated scenario (section 15).

### 11.3 Scenarios

Any recording can be saved as a named scenario. A scenario is a regression test, because replaying it against a new revision shows exactly where behavior diverges. It is also a demonstration humans can watch, and a starting point for reproducing a bug.

### 11.4 Production recording

- **Continuous but bounded.** Production keeps a rolling buffer of recent effects and saves the full recording around any failure.
- **Sensitive data is controlled by type.** Values of `Secrets` are never recorded. Types marked `sensitive` are redacted or tokenized according to policy.
- **Retention and sampling** are set by project policy.

### 11.5 Observability

Every log entry, error and metric carries the symbol identity and revision that produced it. A production failure links directly to the symbol, its current revision, the contract involved, its verification evidence and the recording. Traces flow back into the store, so both views show how each symbol behaves in the running system, not just in tests.

## 12. Execution Platforms

Solace compiles to the BEAM for services and to JavaScript for browser interfaces, using Elm's architecture for the browser without compiling to Elm itself. Pure code compiles to both, so types, contracts and validation logic are defined once and shared.

### 12.1 The BEAM

The BEAM matches Solace's model closely: lightweight isolated processes, message passing, supervision trees, native arbitrary-precision integers and mature distribution. Solace actors compile to BEAM processes, and supervision declarations compile to supervisors. Existing Erlang and Elixir libraries are reachable through foreign declarations (section 17).

The BEAM is weak at sustained numeric computation. CPU-heavy kernels, such as a simulation's inner loop, can use native code through the BEAM's native-function interface. Such code is treated as foreign: its signature and effects are declared, and its behavior is unverified until tests establish it.

### 12.2 The browser

Browser interfaces follow the Elm architecture: state, messages, a pure update function and a view derived from state. This is the same state-plus-message model Solace actors use (section 8.12). Solace compiles this directly to JavaScript with a small runtime rather than generating Elm source, for three reasons:

- Generating Elm would put a second compiler and type checker between Solace and the browser, with its own error messages to translate.
- Elm's interop with JavaScript is deliberately narrow, and the language has not had a release in years.
- Solace needs its own runtime hooks for recording, replay and semantic layout inspection (sections 11 and 13), which a generated Elm program could not expose.

### 12.3 Shared code and platform capabilities

A namespace declares which platforms it targets. Pure namespaces target both. Capabilities are platform-specific: `Dom` exists only in the browser, while `Database` and `FileSystem` exist only on the BEAM. The compiler rejects a platform-crossing capability at build time.

Types marked `wire` generate their serializers and a typed protocol, so messages between browser and server are checked on both ends. Changing a `wire` type requires a migration (section 8.10), which keeps old clients and new servers compatible during deployment.

## 13. Interfaces

Solace interfaces declare layout intent, interaction states and design constraints; the system chooses the geometry, checks it mechanically, and routes the judgments machines cannot make to a human. Agents can generate markup quickly but cannot see whether the result is coherent, usable or consistent, so the interface system is built to supply the missing sight.

### 13.1 Semantic layout

Interfaces describe relationships rather than coordinates and hand-assigned dimensions.

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

The layout engine computes concrete geometry within the declared constraints. Typography, spacing, color, sizing, density and component appearance come from design-system primitives, so agents make no arbitrary visual decisions. Low-level styling remains available through explicit escape hatches, and each use is flagged for human review.

### 13.2 Interaction as state machines

Components declare their states and transitions using the language's state-machine construct (section 8.5). The verification engine explores reachable states and reports invalid, inaccessible or dead-end interactions. Tests address semantic events and component identities, never screen coordinates or text selectors.

### 13.3 Machine-verifiable presentation

The renderer exposes its layout model to the tools. Agents inspect geometry, visibility, clipping, contrast, accessibility, focus order, scrollability and responsive behavior directly, without relying on screenshots.

```
solace ui inspect checkout
solace ui validate checkout
solace ui diff checkout --baseline main
```

Validation reports each violated constraint along with the viewports and interaction states where it occurs.

### 13.4 Human judgment of experience

Mechanical checks cannot establish that an interface is good. For that, the human view renders a matrix of every component across viewports and interaction states, shows alternatives side by side, and highlights what changed since the last approval. The human's choice is recorded as a decision (section 9.4). Where possible it becomes an enforced constraint: "related totals must remain visible while the form scrolls" is checked on every future change.

### 13.5 Rendering

The first renderer targets HTML and CSS in the browser. Replacing browser rendering is not a prerequisite for validating the model.

## 14. The Agent View

Agents work through a protocol of few, rich operations that return compact text; for agents, this protocol is the environment and is specified as carefully as a public API. It is designed around the ways agents actually lose time: orientation, fragile edits, missing memory and uninformative errors.

### 14.1 Protocol design rules

- **Fewer, richer calls.** Every round trip costs time and context, so one call should answer a whole question. Commands that take a symbol accept several, and return all the answers together.
- **Cheap defaults.** The views agents reach for by habit return outlines, and bodies are shown on request. Changing what the familiar command returns works; asking agents in the reference to use a different command doesn't reliably break the habit.
- **Text by default.** Output is compact, readable text; structured JSON is available for tools.
- **Names in, identities internal.** Agents never need to type an identity (section 6.2).
- **Parity.** Anything a human can do or see in their view is available through the protocol.

### 14.2 Starting a session

```
solace session start --scope pricing
```

This returns:

- The language reference version.
- The policy rules that apply to the scope.
- A symbol index: every symbol in scope on one line, with its name, signature and one-line description. At about 15–25 tokens per symbol, a 300-symbol project costs about 6k tokens once, far less than the search-and-read turns it replaces. For large projects, `--scope` narrows the index, and each namespace shows "N more" beyond a cap.
- Open work records for the symbols in scope, each expanded with the context bundles of the symbols it names (section 9.3).
- Decisions made since the last session touched the scope.

From the index alone, an agent can choose which symbols to inspect without opening anything. A new agent picks up the thread where the last one stopped.

### 14.3 Context in one call

```
solace inspect pricing.quote --context
solace inspect pricing.quote checkout.pay tax.quote --context
```

This returns, as one text document: the symbol's full text with contracts and effects; its description and rationale; direct callers and callees with their signatures; the types it uses; its verification status and debt; relevant decisions; and open work records. Given several names, it returns all their bundles in one document, because every turn re-sends the agent's growing context, and turns, not the size of answers, dominate orientation cost. Orientation that takes a dozen searches and file reads today becomes one call after session start.

### 14.4 Workflow

1. **Inspect** the relevant symbols, contracts and relationships.
2. **Plan** the affected behavior and required verification, and record it as a work record.
3. **Modify** by submitting symbol text in a transaction.
4. **Validate** with fast checks on every edit and the focused plan before commit.
5. **Review** the semantic diff, stale prose and remaining verification debt.
6. **Commit** the change with its evidence, or raise a decision request and wait.

### 14.5 Transactions

A change declares its write scope, the set of symbols it may modify, and the revisions it was based on. If any of those revisions changed in the meantime, the transaction is rejected with the difference, never silently merged. Agents work concurrently on independent symbols without textual conflicts. Semantic conflict detection still catches incompatible changes to different but interdependent symbols, such as one agent changing a contract while another adds a caller that relies on the old one.

### 14.6 Diagnostics

Every diagnostic has a stable code, a location as symbol plus expression, and a plain explanation. Where the fix is mechanical, a suggested fix is attached. The root cause is reported first and cascading errors are grouped beneath it, so one mistake never appears as forty.

```
E0412 effect not declared
  at pricing.quote, expression #17: clock.now()
  pricing.quote uses Clock.read but declares effects: none
  fix: add Clock.read to effects and a clock: Clock parameter
  affects 3 callers: checkout.summary, cart.preview, api.quote
```

### 14.7 Permissions and isolation

Each agent session runs with capability-based permissions: the namespaces it may write, the capabilities its test runs may use, and resource limits on time and compute. Sessions are isolated from each other and from production. Every operation is recorded in an auditable history tied to the session and its work records.

## 15. The Human View

The human view shows what the system guarantees and how it behaves, at the speed agents change it, and lets a person drop to any level of detail within seconds when something goes wrong. Humans mostly care about contracts and guaranteed behavior, so the view leads with those and keeps implementation text one click away.

### 15.1 System overview

The top level is a map of features, their components and the symbols inside them, marked by verification status and recent change activity. Beside it, a summary of recent changes is written in behavioral terms, such as "refunds now require manager approval above $500", not as a list of edited symbols.

### 15.2 Guarantees

For each feature, the view lists what the system promises — its contracts, refinements, state-machine rules and policies — with the evidence level of each (section 10.1). Assumptions and verification debt are shown alongside, so it is always clear what is known and what is merely hoped.

### 15.3 Decision trees

Every function's branching logic is rendered as a decision tree, from conditions to outcomes, generated from the code so it is always current. State machines render as state diagrams. Selecting a branch shows the examples, tests and recorded scenarios that exercise it, and marks any branch nothing exercises.

### 15.4 Scenario replay

Any recording or saved scenario (section 11) can be watched. The path through each decision tree lights up as execution proceeds, messages move between actors, and interface states appear as they were rendered. A timeline can be scrubbed forward and back, and any value can be inspected at any point.

### 15.5 Descent into detail

Any node — a feature, a branch, a message, an interface element — opens the definition behind it: its text, contracts, description, rationale and history in one place. Navigation follows relationships such as callers, callees, data flow and state ownership, never files.

For emergencies, a production failure opens directly into a path: the failing contract, the symbol and revision, the recording of the failure, and the recent changes to every symbol involved, with the agent session and work record that made each change. A human can go from alert to understanding without knowing where anything lives. They can also make a fix directly through the same transactions agents use.

### 15.6 Decision queue

Pending decision requests from agents appear in one queue. Each shows the question, the options with their consequences, and the affected symbols, ordered by how much work each is blocking. Answers become decision records (section 9.4).

### 15.7 Behavioral review

A change is reviewed in order of what matters:

1. Contract and policy changes, with any weakening flagged.
2. Behavior changes: altered decision trees, new or removed states, changed examples, divergent scenario replays.
3. Verification evidence and any new debt.
4. Interface changes, rendered across viewports and states.
5. Implementation text, shown only on request.

Mechanical changes that alter no contract, behavior or interface are summarized in one line rather than shown in full.

## 16. Change, Review and Policy

Changes are recorded as semantic changesets, classified by what they affect, and admitted according to project policy. Human attention is required only where policy says a human must judge.

### 16.1 Semantic changesets

A changeset records which symbols were added, removed, modified or renamed. It also records changes to contracts, dependencies, effects, decisions and verification status, together with the evidence gathered for it. A symbol's implementation may change while its identity and contract stay the same, and the changeset shows that distinction directly.

### 16.2 Change classes

| Change class | Examples | Default admission |
| --- | --- | --- |
| Implementation only | Refactoring or optimization; contracts, behavior and interface unchanged; evidence maintained | Automatic once the full verification plan passes |
| Contract strengthening | New postcondition, tighter refinement, added example | Automatic; shown in review |
| Behavior change within contracts | Different decision tree, new state, changed scenario replay | Automatic; summarized for the human view |
| Interface change | Layout, component or interaction change | Human review of rendered states, per policy |
| Contract weakening | Removed postcondition, loosened refinement, deleted example | Human approval |
| Verification debt increase | New unverified behavior | Human approval |
| Capability grant | A namespace or actor gains a capability | Human approval |
| Stored-type migration | Breaking change to a `stored` or `wire` type | Human approval |
| Policy change | Any edit to project policy | Human approval |

### 16.3 Policy

Project policy is itself written in Solace and enforced by the policy engine on every change. It is where human judgment becomes durable.

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

### 16.4 History and persistence

The store keeps the full semantic history of every symbol. It exports deterministically to the open text format described in section 7.5, which is readable without Solace and re-imports without loss. A program can be archived, inspected and recovered without any hosted service. During early development, Git stores the canonical export, so no new version-control system is required up front.

## 17. Dependencies, Builds, Interop and Standard Library

A Solace project builds identically on any machine with one command, and every outside dependency enters through a declared, verifiable boundary. Environment and toolchain churn is one of the largest wastes of agent time today; Solace removes it by construction.

### 17.1 Dependencies

Solace packages are exported stores, referenced by identity and revision and pinned exactly. Nothing is resolved from version ranges at build time. Updating a dependency is an ordinary changeset: its contract changes are reviewed and its verification plan runs like any other change.

### 17.2 Hermetic builds

The toolchain version is pinned in the project, and the toolchain ships as a single executable. A build depends only on the store, the pinned toolchain and the pinned dependencies, so the same inputs always produce the same outputs. Setting up a machine means installing one binary.

### 17.3 Interop

Erlang, Elixir, JavaScript and native code are reached through foreign declarations. Each must declare its signature and effects. Foreign code is unverified until tests establish its behavior or a human records an assumption. Contracts on a foreign declaration are checked at runtime at the boundary.

```
foreign erlang crypto.hash(algorithm: HashAlgorithm, data: Bytes) -> Bytes
    effects: none
    ensures: length(result) == algorithm.digestSize
```

### 17.4 Standard library

The standard library is broad, with one obvious choice for each common need: collections, text, time and durations, decimal arithmetic, units, JSON, HTTP client and server, database access, logging, testing and generators, and interface components with a design system. Agents never choose between competing libraries for a routine task.

## 18. Implementation Strategy

Agents build Solace in seven gated phases, and the project adopts its own principles from the first day rather than waiting for the tools to exist. Each phase ends at a gate with a measurable criterion; a phase that misses its gate is revised before the next begins.

| Phase | Scope | Gate |
| --- | --- | --- |
| 0 Specify | Identity, text projection, agent protocol and language core specs; benchmark suite and baselines | Specs accepted; benchmark runs end to end on conventional tooling |
| 1 Core system | Store, identities, projection, language core, fast checker, BEAM backend, agent protocol, export | Agents build and maintain the pilot through the protocol alone; typical tasks reach their first edit within 3 model turns (median); export round-trips losslessly |
| 2 Verification and replay | Solver-backed contracts, test generation, properties, mutation, verification debt, check protection, recording and replay | Seeded pilot defects caught before acceptance; replay reproduces recorded failures |
| 3 Human view | Overview, guarantees, decision trees, scenario replay, decision queue, behavioral review | A human traces an injected failure to its cause without navigating files |
| 4 Browser interfaces | JavaScript backend, state-plus-message runtime, semantic layout, interface validation | Pilot interface passes validation; recorded design decisions are enforced |
| 5 Actors, performance, production | Supervision, performance budgets, production recording, symbol-level observability | Pilot runs under production-like load with recording on and budgets enforced |
| 6 Collaboration and ecosystem | Concurrent multi-agent changes at scale, packages, broad interop, semantic history beyond Git | — |

Phase 1 is the minimum viable system and the central experiment: whether a semantic model with text projection is a better foundation for agents to build on.

### 18.1 Building with agents

- **Specs are the contracts.** Each component is built against its written spec, and the spec is updated through decisions, not silently.
- **Principles before tools.** Until the store exists, work records and decisions are kept in a structured log in the repository, in the format section 9 defines, so they can be imported later.
- **Self-use as early as possible.** From Phase 2, new Solace components are written in Solace where the language allows, and the human view in Phase 3 is itself a Solace browser application. Every gap agents hit while building Solace is a defect in Solace.
- **Toolchain.** The compiler, store and checker are written in Rust and ship as a single executable, for fast checks and hermetic distribution. Runtime support for the BEAM is written in Erlang.

### 18.2 Specifications

| # | Specification | Needed by |
| --- | --- | --- |
| 1 | Identity and revision model (section 6) | Phase 0 |
| 2 | Text projection and export format (section 7) | Phase 0 |
| 3 | Agent protocol and diagnostics (section 14) | Phase 0 |
| 4 | Language core: types, functions, effects, holes, state machines (section 8) | Phase 0 |
| 5 | Metadata schema: docs, rationale, decisions, work records (section 9) | Phase 1 |
| 6 | Effect runtime, recording and replay (section 11) | Phase 2 |
| 7 | Verification model, evidence levels and debt (section 10) | Phase 2 |
| 8 | Change classes and policy language (section 16) | Phase 2 |
| 9 | Human view (section 15) | Phase 3 |
| 10 | Interface language and browser runtime (sections 12 and 13) | Phase 4 |
| 11 | Actors, supervision and performance budgets (sections 8.12 and 10.9) | Phase 5 |
| 12 | Observability (section 11.5) | Phase 5 |
| 13 | Dependencies, builds, interop and standard library (section 17) | Phase 1, extended in Phase 6 |

### 18.3 Minimum viable system (Phase 1)

- A small functional language: algebraic data types, records, distinct and refined types, exhaustive matching, `Option` and `Result`, required labels, examples, typed holes and state machines.
- Effects as capabilities, with test fakes for `Clock`, `Random` and `Log`.
- The semantic store with persistent identities, revisions and the identity rules of section 6.3.
- Canonical text projection, symbol-scoped editing and the deterministic export format.
- The fast checker with structured diagnostics, meeting the sub-second target.
- The agent protocol: session start, context bundles, transactions, work records and decision requests.
- Examples run as tests, with basic verification reporting and automatic debt for holes.
- The BEAM backend.

The interface system, full verification, actors, performance budgets and the full human view follow once the foundation is validated. In Phase 1, humans review changes through a text rendering of the behavioral review.

### 18.4 Pilot application

The pilot is a small order-management web service: stored data, a checkout state machine, capabilities for database and clock, and later a browser interface and actors. It exercises most of Solace without numeric-heavy work. A second pilot, the smoke-plume simulation, follows in Phase 5 to test native numeric kernels and performance budgets.

## 19. Evaluation

Solace's value must be shown by measurement, starting in Phase 0 and repeated at every gate. The principal measure is not first-attempt code generation accuracy; it is how reliably and efficiently agents deliver, verify, modify and maintain working software over time, and how well humans keep up with them.

### 19.1 Benchmark

The benchmark suite is built in Phase 0 and runs at every gate. It compares Solace against conventional languages and tooling using equivalent tasks, models, hardware and budgets. Tasks include new features, defect correction, architectural refactoring, performance work, maintenance of an unfamiliar codebase and interface refinement. Agents are evaluated both with no prior exposure to Solace and with the full reference, examples and tools.

| Metric | What it measures |
| --- | --- |
| Functional correctness | Hidden-test and contract satisfaction rates |
| Agent autonomy | Human interventions per successful task |
| Development efficiency | Tokens, time and model turns per completed task |
| Orientation cost | Model turns before the first edit (primary), and tokens before the first edit |
| Debugging efficiency | Time from failure report to verified fix |
| Regression frequency | Previously working behavior broken by later changes |
| Refactoring reliability | Successful changes across dependent symbols |
| Verification effectiveness | Defects caught before acceptance; weakened checks caught |
| Continuity | Work correctly resumed by a new session from work records. The baseline gets the same information as a handoff note in the repository, so the comparison tests where the information lives and how it is surfaced, not whether it exists |
| Documentation consistency | Prose left stale by changes |
| Performance stability | Undetected resource regressions |
| Interface correctness | Layout, accessibility and interaction defects |
| Human keep-up | Time for a human to understand a day's changes; decision latency |
| Human experience | Comprehension, confidence and preference |

### 19.2 Ablations

Controlled ablations isolate the contribution of individual features: stable identities, text projection, context bundles, effects as capabilities, structured diagnostics, replay, work records and automated verification. A feature that measurably helps nobody is a candidate for removal, in keeping with the goal of a small system.

### 19.3 Gate decisions

Each gate in section 18 is judged against the benchmark. Orientation is judged in model turns before the first edit, since every turn re-sends the whole context and tokens mostly follow turns. That makes the claim of orientation in one call something the benchmark can confirm or refute. If a gate is missed, the phase is revised; the next phase does not start on an unvalidated foundation.

## 20. Risks and Tradeoffs

The largest risk is agents' unfamiliarity with a new language; most other risks are engineering scope, managed by the phase gates.

| Risk | Mitigation |
| --- | --- |
| Language unfamiliarity: agents have no training exposure to Solace | Small language, familiar syntax, a reference that fits in one read, excellent diagnostics, many examples |
| Architectural complexity: store, compiler, verification and views are a large undertaking | Independently testable components, phase gates, a narrow minimum viable system |
| Identity edge cases beyond section 6.3 | Specify before Phase 1; treat any identity surprise found by agents as a spec defect |
| Verification scalability: exhaustive exploration is infeasible | Combine static proof, bounded exploration and targeted tests, and report uncertainty explicitly |
| Specification correctness: verified code can satisfy a wrong requirement | Human approval of contracts, whole-result postconditions, examples reviewed by humans |
| Agents weakening checks | Contract changes as a gated change class; weakening detection; evidence required to clear debt |
| Interface quality cannot be fully checked mechanically | Human judgment through the review matrix; decisions recorded as enforced constraints |
| Numeric performance on the BEAM | Native kernels as declared foreign code; simulation pilot in Phase 5 |
| Recording overhead and sensitive data | Bounded buffers, sampling, type-based redaction, secrets never recorded |
| Human view falling behind agent speed | Behavioral summaries, review ordered by importance, mechanical changes collapsed |
| Ecosystem: no mature libraries | Broad standard library; Erlang, Elixir and JavaScript reachable through foreign declarations |
| Tool dependency | Open, deterministic export; Git-compatible; no hosted service required |

## 21. Longer-Term Research

Once the foundation is established, several further capabilities become possible. They are research directions, not prerequisites.

- **Architectural insight.** Compiler-generated architecture documentation, identification of obsolete abstractions, and analysis of behavioral change across revisions.
- **An evidence base.** Verification evidence and production traces accumulate, so future agents can tell well-established properties from assumptions that need fresh investigation.
- **More renderers.** Interfaces that keep one semantic specification while adapting to desktop, mobile and other platforms.
- **Coordinated agent teams.** Many agents working on separate regions of a program at once, with a verification coordinator evaluating their combined changes.

## 22. Vision

Solace aims to change the fundamental unit of software development. Traditional development treats a program as text that must be interpreted, organized, tested, documented and maintained through many separate tools. Solace treats a program as a connected collection of identifiable, typed, documented and verifiable semantic objects.

Source text is one view of those objects. Contracts specify their expected behavior. Verification establishes what is known about their correctness. Recordings show what they actually did. Decisions record why they are the way they are. Version control records how they evolve.

Agents build, because they are fast and tireless. Humans direct, because they hold intent, perception and judgment. Each works through a view designed for how they think, on one shared model, and neither wastes time on work the other or the system does better.

The intention is not merely a language that agents can write. It is an environment in which software is easier to understand, harder to change incorrectly, and far more amenable to autonomous verification and maintenance — with humans firmly in control of what it is for.
