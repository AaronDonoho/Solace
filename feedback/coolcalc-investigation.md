# Coolcalc investigation run — 2026-09-30

Task `6488f56ee8cec7e0`, run `802668715cb7273c`, model `codex-gpt-6-sol`.
Evidence: `cool_test_app/.solace/board/6488f56ee8cec7e0/802668715cb7273c.jsonl`, the generated `config.toml`, and the adjacent task JSON in Aaron's `ai_playground/solace_testing` project.

## Outcome

The task was an **investigation**, but the agent implemented calculator functions and tests. The log reports five passing tests and a successful build. The board ended blocked, with no questions and no recorded changes, because the agent could not report completion. There were 22 MCP calls, 10 failed calls, and roughly 6 minutes 35 seconds between the board's start/end timestamps. Token usage: 439,329 input, including 412,672 cached input, and 6,845 output.

## Root cause and fix

The launcher exports `SOLACE_TASK` and `SOLACE_RUN` to Codex. The generated MCP configuration did not forward them to the Solace server. Codex supports an explicit `env_vars` allowlist ([OpenAI Docs](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)).

In `run_options`, absent task context bypasses all three of these together:

- Routing of `task ask`, `task note`, and `task done`.
- Read-only investigation/review enforcement in `agent_guard`.
- Task snapshots and change recording used for revert.

This explains the observed combination: unknown task commands, successful investigation edits, and an empty changes list. The MCP process environment was not captured in the logs; the diagnosis follows from the generated configuration, documented forwarding behavior, and the dispatch code.

Changes made in Solace:

- Forward `SOLACE_TASK`, `SOLACE_RUN`, `SOLACE_SESSION`, and `SOLACE_CONFIG` in generated Codex MCP configuration. The last preserves a custom system settings path.
- Tell agents explicitly when the configured task type prohibits edits. Clarification is conditional; the original agent interpreted “Ask with task ask” as requiring a question.
- Have the fake agent launch the configured MCP command and forward Solace variables only through the generated allowlist. Existing acceptance scenarios now exercise this boundary for questions, completion, write refusal, change recording, and revert.
- Verify investigation instructions and unchanged source/resources in the existing read-only acceptance scenario.

These source changes apply after rebuilding/reinstalling Solace and starting a new run. They do not repair the original run's missing before-state records. Its empty changes list cannot undo the calculator edits through board revert.

## Next tooling improvements, in order

| Finding | Evidence | Smallest useful improvement |
| --- | --- | --- |
| New source modules cannot be created through the current protocol. | Calls `item_16`–`item_18`: `submit --new coolcalc`, `submit coolcalc`, and `write src/coolcalc.gleam` all fail. The last suggests a command that already failed. | Add explicit module creation with type checking and task revert support; give an actionable diagnostic until it exists. This is needed for implementation tasks, not investigations. |
| No end-to-end CLI verification. | Tests call `evaluate`, `parse_number`, and `overview`; none execute `main`. The project has no `solace.toml`, hence no declared run commands. | During project setup, declare a bounded `gleam run` command accepting arguments, then exercise no arguments, arithmetic, powers, negative numbers, and error exit status. |
| README is read-only by default. | `files` labels it tier 4; `item_19` is rejected. | Have the human declare `README.md` as a resource in `solace.toml`. Keep the existing tier policy; make setup expose it clearly. |
| Passing function tests leave runtime boundaries unchecked. | `arguments()` declares Erlang `init:get_plain_arguments` as `List(String)`; `power` calls `math:pow` directly. | Verify the argv representation at runtime and test invalid power domains/overflow. These are code-review risks, not failures demonstrated by this log. |
| Failure status is hard to read. | The task attention reason embeds a truncated tail of raw JSON. | Extract the last agent message for board display and retain the full log link. |

The successful symbol submissions, compiler checks, and test runner show that the core edit loop worked. The immediate priority is preserving task identity and enforcing its permissions; expanding shell access is unnecessary for the observed failures.

## Validation

`cargo test --manifest-path solace-cli/Cargo.toml`: 21 unit tests passed. The sandbox initially refused the local board HTTP listener; all 11 acceptance scenarios passed when rerun outside it. These use a simulated Codex environment boundary and the real Solace MCP server; a new real-Codex run remains untested. No paid agent or provider call was made, and the supplied test project was only read.
