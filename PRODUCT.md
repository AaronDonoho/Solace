# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users
Primary: the owner of a software project who directs AI coding agents through Solace. Design for someone who can't read code, without slowing down a technical owner. Either way, they give creative input and judgement, review what agents did, and run the program to see what it does. They do not read source code or use the command line; the browser UI is their whole interface.

Secondary: Aaron, the developer of Solace, reading benchmark results in the bench visualizer.

## Product Purpose
Solace is a semantic programming system that wraps a user's project (currently Gleam). Agents plan and write code; humans see contracts, effects and tests instead of code. Success means a human can direct work, judge its results and catch an agent's mistakes without opening a source file.

## Positioning
The human's view comes from Solace's own store of identified, typed symbols and from the task's recorded before-state, not from what an agent says it did. Review shows what changed in terms of effects, signatures, test results and replayed runs.

## Operating Context
- The **board** (Spec 15): tasks move through columns; the human writes tasks, answers agents' questions and reviews changes.
- The **contract view** (Spec 16): a drawn graph of entry points, logic, effects, tests and files; ad-hoc runs of any public function with forms built from types; task review with test results and before/after replays.
- The **bench visualizer**: benchmark results across runs.
- Live updates come through server-sent events while agents work.

## Capabilities and Constraints
- Local-only, one user: served on localhost by the `solace serve` binary; no accounts.
- The board and view are Lustre (Gleam on the JavaScript target), bundled by bun into one `app.js` and embedded in the binary.
- Every derived fact is labelled as `declared`, `derived` or `observed`; the UI never presents a derived fact as a guarantee.
- Source text appears only on request.

## Brand Commitments
Name only: Solace. No logo or voice set yet.

## Evidence on Hand
Real projects to show: `pilot/`, `pilot-large/`, and benchmark results in `bench/results/`. There are no users, testimonials or external claims; don't invent any.

## Product Principles
1. Humans judge; agents implement. The UI asks for judgement, never for code.
2. Show data in, data out and effects, not how the code got there.
3. Facts come from the store; label how each one is known.
4. A difference isn't a failure. Present the evidence and let the human decide.
5. No command line on the human side.

## Accessibility & Inclusion
WCAG 2.2 AA: full keyboard use, AA contrast in light and dark, and screen-reader labels, including for graph nodes.
