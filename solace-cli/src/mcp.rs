//! The MCP tool `solace mcp` serves over stdio: one call's commands, their
//! replies, and the D-145/D-157 hints. It keeps the agent-facing layer apart
//! from the CLI (main.rs holds the rest).
use crate::{
    REASON, SHOWN, agent_reference,
};
use crate::cli::{batch, is_question, looks_like_code, panic_reply, read_hint, reset_after_panic, run_options};
use crate::failure::usage;
use crate::reads;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::io::Write;

#[cfg(test)] use crate::cli::with_root;
use crate::board::runs::context;
use crate::board::runs::strategy_context;
/// The one MCP tool `solace mcp` serves. A strategy run's agent only reads
/// and plans (D-93), so it is told only those commands.
pub(crate) fn tool_schema() -> Value {
    if strategy_context().is_some() {
        return json!({
            "name": "solace",
            "description": concat!(
                "Read this project's code and plan with the human; this run changes no code. ",
                "Read with `view`, `inspect`, `refs`, `find`, `grep`, `files` and `read` ",
                "(`--project <folder>` for a project below the root); read the plan with ",
                "`strategy list [--archived]` and `strategy show <task|feature|milestone> <id>...` ",
                "(one id, or several in one reply) (a task's notes in full: ",
                "`strategy show note <task> <notes>`; a task's run log: ",
                "`strategy show run <task> <run> [<steps>|all]`, or `--failed` alone); propose a change with ",
                "`strategy task|feature|milestone [<id>]` or `strategy vision`, alone in its call; ",
                "reply with `strategy note`. Several reads go in one call, one per line, and run in order."
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": concat!(
                            "One or more commands, one per line, e.g. `strategy show feature <id>` ",
                            "or `view app.main --project <folder>`"
                        )
                    },
                    "text": {
                        "type": "string",
                        "description": concat!(
                            "The reply for `strategy note`; a proposal's fields as a JSON object ",
                            "(`help strategy` lists them per kind); the new vision for `strategy vision`"
                        )
                    }
                },
                "required": ["command"]
            }
        });
    }
    json!({
        "name": "solace",
        "description": concat!(
            "Read, change and test this project's code. In a board run, the brief holds the reply to ",
            "`session start --task`: start with the call its `Next:` line names; after a compaction ",
            "notice, ask with `session remember` for the task, its notes and what this run has done. ",
            "Elsewhere, start with `session start --task`: it lists the symbols most likely relevant, ",
            "the workflow and the commands. Several commands go in one call, one per line, and run in ",
            "order: put every independent read in the same call (`inspect a.f b.g --context`, then ",
            "`view c` on the next line), never one read per call, and never send a change alone: put ",
            "its check on the line after it (`test` after `submit`, `run <name>` after `write`). Keep ",
            "text between calls to a line or two: all of it is re-sent on every later call. Once the ",
            "work is done, stop: no recap and no repeated checks."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": concat!(
                        "One or more commands, one per line, e.g. ",
                        "`inspect checkout.pay tax.quote --context`, or `submit` then `test`"
                    )
                },
                "text": {
                    "type": "string",
                    "description": concat!(
                        "For submit: a patch (`*** Begin Patch` / `*** Update: <symbol or module>` / ",
                        "`@@` hunks) or new code in `text`; `command` holds only the command lines ",
                        "(`submit --project <folder>` then `test --project <folder>` for another ",
                        "project); for `write` with no path: a patch of files ",
                        "(`*** Update File: <path>`, `*** Add File: <path>`, `*** Delete File: <path>`); ",
                        "the task for `session start --task`; the record, note, summary or question for ",
                        "`work add` and `task note|done|ask`"
                    )
                },
                "options": {"type": "array", "items": {"type": "string"}, "description": "2–4 options, for `task ask`"},
                "reason": {
                    "type": "string",
                    "description": concat!(
                        "For a settings `write` (solace.toml, AGENTS.md): why the change, ",
                        "shown with the question the human approves"
                    )
                },
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
    // A patch belongs in `text`; `command` holds the command that takes it.
    if patch_in_command(&commands) {
        let sent = commands
            .iter()
            .take_while(|c| !c.starts_with("*** Begin Patch"))
            .copied()
            .find(|c| matches!(c.split_whitespace().next(), Some("submit" | "write")));
        let shape = match sent {
            Some(c) => format!("`command`: `{c}`, `text`: the patch"),
            None => "`command`: `submit` or `write`, `text`: the patch".to_string(),
        };
        out = format!("the patch goes in `text`, not in `command`: {shape}; nothing in this call ran.\n");
        is_error = true;
    }
    // Code sent as command lines rather than in `text` (run 11): one error for the
    // call, and nothing runs.
    if !is_error && let Some(at) = commands.iter().position(|c| looks_like_code(c)) {
        let code = commands[at];
        let sent = commands[..at].iter().copied().find(|c| matches!(c.split_whitespace().next(), Some("probe" | "submit")));
        let shape = match sent {
            Some(c) => format!("`command`: `{c}`, `text`: the code"),
            None => "`command`: `probe` or `submit`, `text`: the code".to_string(),
        };
        out = format!("Not a command: {code}\nThis looks like code: {shape}; nothing in this call ran.\n");
        is_error = true;
    }
    // A shell suffix is not part of a Solace command: name it and run nothing.
    if !is_error && let Some(suffix) = commands.iter().find_map(|c| shell_suffix(c)) {
        out = format!("commands are not shell: drop `{suffix}`; nothing in this call ran.\n");
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
        (out, is_error) = (format!("No command given.\n\n{}", agent_reference(context().is_some())), true);
    }
    // D-157: each command updated the window; the run's reads are saved once.
    if let Some(run) = run_reads {
        run.end_call();
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
    let reply = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)) {
        Ok(reply) => reply,
        Err(p) => {
            reset_after_panic();
            json!({"content": [{"type": "text", "text": panic_reply(&*p, None)}], "isError": true})
        }
    };
    crate::language::rust::clear_spans();
    reply
}

/// `solace mcp`: the same commands as one MCP tool over stdio, so an agent
/// can pass code without shell quoting and needs no shell at all.
pub(crate) fn mcp(budget: usize) {
    crate::store_write::capture_running_binary();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut single_reads = 0;
    // A harness takes its tool list once at startup. Answer the handshake
    // before touching the store: `tools/call` is the first thing that needs
    // the run's reads.
    let mut run_reads: Option<Option<reads::Run>> = None;
    for line in stdin.lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(reply) = dispatch(&msg, budget, &mut single_reads, &mut run_reads) {
            let _ = writeln!(stdout, "{reply}");
            let _ = stdout.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// task:da797576d0cfaaac: a panic outside a command line (the code check,
    /// the re-read hint) is an error reply too, and the tool keeps serving.
    #[test]
    fn a_panic_anywhere_in_the_call_is_an_error_reply() {
        let e = guarded(|| {
            let _: proc_macro2::TokenStream = "fn go() {}\n".parse().unwrap();
            panic!("boom");
        });
        assert_eq!(e["isError"], json!(true));
        assert_eq!(e["content"][0]["text"], json!("Solace hit an internal error: boom; nothing after it ran\n"));
        assert_eq!(crate::testing::rust_span_start(), "bytes(1..2)");
        // A call that doesn't panic is its own reply.
        let ok = guarded(|| json!({"content": [{"type": "text", "text": "fine"}], "isError": false}));
        assert_eq!((ok["isError"].clone(), ok["content"][0]["text"].clone()), (json!(false), json!("fine")));
    }

    #[test]
    fn no_command_is_an_error_reply() {
        let msg = json!({"params": {"arguments": {"command": ""}}});
        let (mut reads, mut run) = (0, None);
        let out = tool_call(&msg, 10_000, &mut reads, &mut run);
        assert_eq!(out["isError"], json!(true));
        assert!(out["content"][0]["text"].as_str().unwrap().starts_with("No command given."));
    }

    #[test]
    fn a_patch_in_command_is_refused() {
        let msg = json!({"params": {"arguments": {"command": "*** Begin Patch\n*** Update: app.main\n*** End Patch"}}});
        let (mut reads, mut run) = (0, None);
        let out = tool_call(&msg, 10_000, &mut reads, &mut run);
        assert_eq!(out["isError"], json!(true));
        let expected = concat!(
            "the patch goes in `text`, not in `command`: ",
            "`command`: `submit` or `write`, `text`: the patch; nothing in this call ran.\n"
        );
        assert_eq!(out["content"][0]["text"], json!(expected));
    }

    #[test]
    fn a_shell_suffix_is_refused_once_with_its_text() {
        let msg = json!({"params": {"arguments": {"command": "view x 2>/dev/null || true\nview y | head"}}});
        let (mut reads, mut run) = (0, None);
        let out = tool_call(&msg, 10_000, &mut reads, &mut run);
        assert_eq!(out["isError"], json!(true));
        assert_eq!(out["content"][0]["text"], json!("commands are not shell: drop `2>/dev/null || true`; nothing in this call ran.\n"));
    }

    #[test]
    fn a_quoted_pipe_in_grep_still_runs() {
        let root = crate::testing::Scratch::new().with(&[("notes.md", "a|b\n")]);
        let msg = json!({"params": {"arguments": {"command": "grep \"a|b\" --in notes.md"}}});
        let (mut reads, mut run) = (0, None);
        let out = with_root(&root, || tool_call(&msg, 10_000, &mut reads, &mut run));
        assert!(!out["content"][0]["text"].as_str().unwrap().contains("commands are not shell"), "{out}");
    }

    #[test]
    fn patch_in_command_is_told_from_a_plain_command() {
        assert!(patch_in_command(&["submit", "*** Begin Patch", "*** End Patch"]));
        assert!(!patch_in_command(&["submit", "*** Update: app.main"]));
    }

    #[test]
    fn shell_suffix_names_the_part_to_drop() {
        assert_eq!(shell_suffix("view x 2>/dev/null || true"), Some("2>/dev/null || true"));
        assert_eq!(shell_suffix("find x | head"), Some("| head"));
        assert_eq!(shell_suffix("make && test"), Some("&& test"));
        assert_eq!(shell_suffix("inspect a.B<'a>::f 2>/dev/null"), Some("2>/dev/null"));
    }

    #[test]
    fn shell_suffix_ignores_a_quoted_pipe() {
        assert_eq!(shell_suffix("grep \"a|b\" src"), None);
        assert_eq!(shell_suffix("find 'x || y'"), None);
    }

    #[test]
    fn shell_suffix_ignores_rust_angles() {
        assert_eq!(shell_suffix("inspect language::rust_refs.Refs<'a>::resolve"), None);
        assert_eq!(shell_suffix("inspect crate.<Failure as From<std::io::Error>>::from"), None);
        assert_eq!(shell_suffix("view crate.<S as From<&&str>>"), None);
        assert_eq!(shell_suffix("grep ->"), None);
        assert_eq!(shell_suffix("grep Vec<u8>"), None);
        assert_eq!(shell_suffix("grep a<b | head"), Some("| head"));
    }

    #[test]
    fn a_rust_name_with_angles_is_not_shell() {
        let root = crate::testing::Scratch::new();
        let msg = json!({"params": {"arguments": {"command": "inspect language::rust_refs.Refs<'a>::resolve"}}});
        let (mut reads, mut run) = (0, None);
        let out = with_root(&root, || tool_call(&msg, 10_000, &mut reads, &mut run));
        assert!(!out["content"][0]["text"].as_str().unwrap().contains("commands are not shell"), "{out}");
    }

    #[test]
    fn a_patch_in_command_names_the_command_it_was_sent_with() {
        let command = "write --project solace-cli\n*** Begin Patch\n*** Update: app.main\n*** End Patch";
        let msg = json!({"params": {"arguments": {"command": command}}});
        let (mut reads, mut run) = (0, None);
        let out = tool_call(&msg, 10_000, &mut reads, &mut run);
        assert_eq!(out["isError"], json!(true));
        let text = out["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("`command`: `write --project solace-cli`, `text`: the patch"), "{text}");
    }

    #[test]
    fn code_in_command_names_the_command_and_text() {
        let msg = json!({"params": {"arguments": {"command": "probe\nlet x = 1"}}});
        let (mut reads, mut run) = (0, None);
        let out = tool_call(&msg, 10_000, &mut reads, &mut run);
        assert_eq!(out["isError"], json!(true));
        let text = out["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("`command`: `probe`, `text`: the code"), "{text}");
    }

    #[test]
    fn a_hunk_line_is_not_named_as_the_command() {
        let command = "*** Begin Patch\n*** Update: a.b\n@@ fn x\n-old\n+new\n*** End Patch";
        let msg = json!({"params": {"arguments": {"command": command}}});
        let (mut reads, mut run) = (0, None);
        let out = tool_call(&msg, 10_000, &mut reads, &mut run);
        assert_eq!(out["isError"], json!(true));
        let text = out["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("`command`: `submit` or `write`, `text`: the patch"), "{text}");
        assert!(!text.contains("@@"), "{text}");
    }

    #[test]
    fn code_before_a_command_is_not_named_as_the_command() {
        let msg = json!({"params": {"arguments": {"command": "let x = 1\nprobe"}}});
        let (mut reads, mut run) = (0, None);
        let out = tool_call(&msg, 10_000, &mut reads, &mut run);
        assert_eq!(out["isError"], json!(true));
        let text = out["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("`command`: `probe` or `submit`, `text`: the code"), "{text}");
        assert!(!text.contains("`command`: `let"), "{text}");
    }

    #[test]
    fn a_trait_impl_name_is_not_shell() {
        let root = crate::testing::Scratch::new();
        let msg = json!({"params": {"arguments": {"command": "view crate.<Failure as From<std::io::Error>>::from"}}});
        let (mut reads, mut run) = (0, None);
        let out = with_root(&root, || tool_call(&msg, 10_000, &mut reads, &mut run));
        let text = out["content"][0]["text"].as_str().unwrap();
        assert!(!text.contains("commands are not shell"), "{out}");
        assert!(!text.contains("Not a command"), "{out}");
    }

    /// The handshake answers without loading the run's reads.
    #[test]
    fn the_handshake_answers_without_loading_the_reads() {
        let (mut single_reads, mut run_reads) = (0, None);
        let init = dispatch(
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}),
            10_000,
            &mut single_reads,
            &mut run_reads,
        )
        .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(init["result"]["serverInfo"]["name"], "solace");
        let list = dispatch(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}), 10_000, &mut single_reads, &mut run_reads).unwrap();
        assert_eq!(list["result"]["tools"][0]["name"], "solace");
        assert!(run_reads.is_none());
    }

    /// Parses that run outside `batch` in a call that succeeds must not grow the
    /// span table for the next call.
    #[test]
    fn a_successful_call_leaves_no_parser_spans() {
        let ok = guarded(|| {
            let _: proc_macro2::TokenStream = "fn go() {}\n".parse().unwrap();
            json!({"content": [{"type": "text", "text": "fine"}], "isError": false})
        });
        assert_eq!(ok["isError"], json!(false));
        assert_eq!(crate::testing::rust_span_start(), "bytes(1..2)");
    }
}

/// Whether the command lines hold a patch, which belongs in `text`.
fn patch_in_command(lines: &[&str]) -> bool {
    lines.iter().any(|line| line.starts_with("*** Begin Patch"))
}

/// The shell part of a command line: its first unquoted redirect or operator,
/// through the end of the line. `None` when the line is a plain Solace command.
fn shell_suffix(line: &str) -> Option<&str> {
    let (suffix, depth) = shell_suffix_pass(line, true);
    if suffix.is_none() && depth > 0 { shell_suffix_pass(line, false).0 } else { suffix }
}

/// One scan for `shell_suffix`; an unbalanced `<` is retried without angle
/// tracking, so a comparison is not mistaken for a trait impl.
fn shell_suffix_pass(line: &str, track_angles: bool) -> (Option<&str>, usize) {
    let bytes = line.as_bytes();
    let mut quote = None;
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match quote {
            Some(q) if bytes[i] == q => quote = None,
            Some(_) => {}
            None => match bytes[i] {
                b'"' => quote = Some(b'"'),
                b'\'' if line[i + 1..].contains('\'') => quote = Some(b'\''),
                b'|' if depth == 0 => return (Some(&line[i..]), depth),
                b'&' if depth == 0 && bytes.get(i + 1) == Some(&b'&') => return (Some(&line[i..]), depth),
                b'2' if depth == 0 && bytes.get(i + 1) == Some(&b'>') && (i == 0 || bytes[i - 1].is_ascii_whitespace()) => {
                    return (Some(&line[i..]), depth);
                }
                b'<' if i == 0 || bytes[i - 1].is_ascii_whitespace() => return (Some(&line[i..]), depth),
                b'>' if depth == 0 && (i == 0 || bytes[i - 1].is_ascii_whitespace()) => return (Some(&line[i..]), depth),
                b'<' if track_angles => depth += 1,
                b'>' if track_angles && depth > 0 => depth -= 1,
                _ => {}
            },
        }
        i += 1;
    }
    (None, depth)
}

/// One MCP message's reply; `None` when no reply is due. The handshake answers
/// before the run's reads are loaded: only `tools/call` needs them.
fn dispatch(msg: &Value, budget: usize, single_reads: &mut usize, run_reads: &mut Option<Option<reads::Run>>) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let reply = match msg["method"].as_str().unwrap_or("") {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "solace", "version": env!("CARGO_PKG_VERSION")}
            }
        }),
        "tools/list" => json!({"jsonrpc": "2.0", "id": id, "result": {"tools": [tool_schema()]}}),
        "tools/call" => {
            let result = guarded(|| {
                let run_reads = run_reads.get_or_insert_with(reads::Run::load);
                tool_call(msg, budget, single_reads, run_reads)
            });
            json!({"jsonrpc": "2.0", "id": id, "result": result})
        }
        "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
        _ => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}}),
    };
    Some(reply)
}
