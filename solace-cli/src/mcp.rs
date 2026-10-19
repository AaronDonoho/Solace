//! The MCP tool `solace mcp` serves over stdio: one call's commands, their
//! replies, and the D-145/D-157 hints. It keeps the agent-facing layer apart
//! from the CLI (main.rs holds the rest).
use crate::{
    REASON, SHOWN, agent_reference, batch, is_question, looks_like_code, panic_reply, read_hint, reset_after_panic, run_options, usage,
};
use crate::{board, reads};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Write;

/// The one MCP tool `solace mcp` serves. A strategy run's agent only reads
/// and plans (D-93), so it is told only those commands.
pub(crate) fn tool_schema() -> Value {
    if board::strategy_context().is_some() {
        return json!({
            "name": "solace",
            "description": "Read this project's code and plan with the human; this run changes no code. Read with \
                `view`, `inspect`, `refs`, `find`, `grep`, `files` and `read` (`--project <folder>` for a project below \
                the root); read the plan with `strategy list [--archived]` and `strategy show <task|feature|milestone> <id>...` (one id, or several in one reply) \
                (a task's notes in full: `strategy show note <task> <notes>`; a task's run log: `strategy show run <task> <run> [<steps>|all]`); propose \
                a change with `strategy task|feature|milestone [<id>]` or `strategy vision`, alone in its call; reply with \
                `strategy note`. Several reads go in one call, one per line, and run in order.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "One or more commands, one per line, e.g. `strategy show feature <id>` or `view app.main --project <folder>`"},
                    "text": {"type": "string", "description": "The reply for `strategy note`; a proposal's fields as a JSON object (`help strategy` lists them per kind); the new vision for `strategy vision`"}
                },
                "required": ["command"]
            }
        });
    }
    json!({
        "name": "solace",
        "description": "Read, change and test this project's code. In a board run, the brief holds the reply to \
            `session start --task`: start with the call its `Next:` line names; after a compaction notice, ask with \
            `session remember` for the task, its notes and what this run has done. Elsewhere, start with \
            `session start --task`: it lists the symbols most likely relevant, the workflow and \
            the commands. Several commands \
            go in one call, one per line, and run in order: put every independent read in the same call \
            (`inspect a.f b.g --context`, then `view c` on the next line), never one read per call, and never send \
            a change alone: put its check on the line after it (`test` after `submit`, `run <name>` after `write`). \
            Keep text between calls to a line or two: all of it is re-sent \
            on every later call. Once the work is done, stop: no recap and no repeated checks.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "One or more commands, one per line, e.g. `inspect checkout.pay tax.quote --context`, or `submit` then `test`"},
                "text": {"type": "string", "description": "For submit: a patch (`*** Begin Patch` / `*** Update: <symbol or module>` / `@@` hunks) or new code; for `write` with no path: a patch of files (`*** Update File: <path>`, `*** Add File: <path>`, `*** Delete File: <path>`); the task for `session start --task`; the record, note, summary or question for `work add` and `task note|done|ask`"},
                "options": {"type": "array", "items": {"type": "string"}, "description": "2–4 options, for `task ask`"},
                "reason": {"type": "string", "description": "For a settings `write` (solace.toml, AGENTS.md): why the change, shown with the question the human approves"},
                "old": {"type": "string", "description": "For `--replace`: the exact text that `text` replaces"}
            },
            "required": ["command"]
        }
    })
}

/// One `tools/call`: the commands, their replies, and the D-145/D-157 hints.
fn tool_call(msg: &Value, budget: usize, single_reads: &mut usize, run_reads: &mut Option<reads::Run>) -> Value {
    let args = &msg["params"]["arguments"];
    let text = args["text"].as_str().map(String::from);
    let mut input = || match &text {
        Some(t) => Ok(t.clone()),
        _ => usage("This command needs the new code in the `text` argument."),
    };
    // Several commands, one per line, run in order in one call.
    let commands: Vec<&str> = args["command"].as_str().unwrap_or("").lines().map(|c| c.trim()).filter(|c| !c.is_empty()).collect();
    let (mut out, mut is_error) = (String::new(), false);
    SHOWN.with(|s| *s.borrow_mut() = Some(HashSet::new()));
    if let Some(run) = run_reads.as_mut() {
        run.start_call();
    }
    // Code sent as command lines rather than in `text` (run 11): one error for the
    // call, and nothing runs.
    if let Some(code) = commands.iter().find(|c| looks_like_code(c)) {
        out = format!(
            "Not a command: {code}\nThis looks like code. Code (for probe, submit) goes in `text`, \
            not in `command`; nothing in this call ran.\n"
        );
        is_error = true;
    }
    // A question is complete in itself: no note or other action beside it.
    if !is_error
        && commands.len() > 1
        && let Some(q) = commands.iter().find(|c| is_question(c))
    {
        out = format!(
            "E7017 `{q}` must be the only command in its call: put the explanation in the question \
            (`text`, or `reason` for a settings write), not in a note beside it; nothing in this call ran.\n"
        );
        is_error = true;
    }
    REASON.with(|r| *r.borrow_mut() = args["reason"].as_str().map(String::from).filter(|r| !r.trim().is_empty()));
    if !is_error {
        let options: Vec<String> =
            args["options"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        {
            let mut command = |words: &[String], left: usize| {
                let result = run_options(words, &mut input, args["old"].as_str(), left, &options);
                if let Some(run) = run_reads.as_mut() {
                    run.end_command(words);
                }
                result
            };
            (out, is_error) = batch(&commands, budget, &mut command);
        }
    }
    if commands.is_empty() {
        (out, is_error) = (format!("No command given.\n\n{}", agent_reference(board::context().is_some())), true);
    }
    // D-157: each command updated the window; the run's reads are saved once.
    if let Some(run) = run_reads {
        run.end_call(&commands);
    }
    let hint = read_hint(single_reads, &commands, is_error);
    if !hint.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out += hint;
    json!({"content": [{"type": "text", "text": out}], "isError": is_error})
}

/// One call's reply; a panic anywhere in it becomes the reply, so the tool
/// keeps serving (task:da797576d0cfaaac).
fn guarded(call: impl FnOnce() -> Value) -> Value {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)) {
        Ok(reply) => reply,
        Err(p) => {
            reset_after_panic();
            json!({"content": [{"type": "text", "text": panic_reply(&*p, None)}], "isError": true})
        }
    }
}

/// `solace mcp`: the same commands as one MCP tool over stdio, so an agent
/// can pass code without shell quoting and needs no shell at all.
pub(crate) fn mcp(budget: usize) {
    let tool = tool_schema();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut single_reads = 0;
    let mut run_reads = reads::Run::load();
    for line in stdin.lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = msg.get("id").cloned() else {
            continue;
        }; // notifications need no reply
        let result = match msg["method"].as_str().unwrap_or("") {
            "initialize" => json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "solace", "version": env!("CARGO_PKG_VERSION")}
            }),
            "tools/list" => json!({"tools": [tool]}),
            "tools/call" => guarded(|| tool_call(&msg, budget, &mut single_reads, &mut run_reads)),
            "ping" => json!({}),
            _ => {
                let reply = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}});
                let _ = writeln!(stdout, "{reply}");
                let _ = stdout.flush();
                continue;
            }
        };
        let _ = writeln!(stdout, "{}", json!({"jsonrpc": "2.0", "id": id, "result": result}));
        let _ = stdout.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// task:da797576d0cfaaac: a panic outside a command line (the code check,
    /// the re-read hint) is an error reply too, and the tool keeps serving.
    #[test]
    fn a_panic_anywhere_in_the_call_is_an_error_reply() {
        let e = guarded(|| panic!("boom"));
        assert_eq!(e["isError"], json!(true));
        assert_eq!(e["content"][0]["text"], json!("Solace hit an internal error: boom; nothing after it ran\n"));
        // A call that doesn't panic is its own reply.
        let ok = guarded(|| json!({"content": [{"type": "text", "text": "fine"}], "isError": false}));
        assert_eq!((ok["isError"].clone(), ok["content"][0]["text"].clone()), (json!(false), json!("fine")));
    }
}
