use super::testing::*;
use super::*;

/// Runs the reference layout check (D-143) on `text`: whether it passed, and its output.
fn reference_layout(text: &str) -> (bool, String) {
    let dir = Scratch::new().with(&[("reference.txt", text)]);
    let out = Command::new("python3")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference_layout.py"))
        .arg(dir.join("reference.txt"))
        .output()
        .unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned())
}
/// A sample reference: "Commands in full" on line 3, a heading, then `lines` from line 5.
fn sample(lines: &[String]) -> String {
    format!("Workflow\n\nCommands in full\nReading\n{}", lines.concat())
}
/// A command row whose description starts at column 19.
fn ref_row(term: &str, description: &str) -> String {
    format!("  {term:<16}{description}\n")
}
/// A line after `spaces` leading spaces.
fn ref_line(spaces: usize, text: &str) -> String {
    format!("{}{text}\n", " ".repeat(spaces))
}
#[test]
fn reference_layout_of_the_embedded_reference_is_clean() {
    let (clean, out) = reference_layout(REFERENCE);
    assert!(clean, "{out}");
}
/// The reference says gleam.toml is written, and that the human never edits files (D-147).
#[test]
fn the_reference_never_hands_a_file_change_to_the_human() {
    assert!(!REFERENCE.contains("(not gleam.toml)"));
    assert!(REFERENCE.contains("gleam.toml: `write` (validated, recorded with the task),\nnever `move` or `remove`"));
    assert!(REFERENCE.contains("The human never edits files or runs commands"));
    assert!(REFERENCE.contains("Never asks the human to change a file"));
    // D-153: a question shows each option's outcome.
    assert!(REFERENCE.contains("The question's `text` gives each option a"));
    assert!(REFERENCE.contains("short example of its outcome"));
    assert!(!agent_reference(false).contains("task ask"), "outside a board run");
}
/// D-148: the reference tells agents to write a record's reference bare, so it
/// renders as a link with the record's title.
#[test]
fn the_reference_names_records_as_links() {
    let tail = "`task:<id>`, `feature:<id>` or `milestone:<id>`, written bare";
    assert_eq!(REFERENCE.matches(tail).count(), 2, "the notes line and the strategy section");
}
/// D-144: a command in the summary table with no entry in the section is
/// named, so `help <command>` can answer one paragraph.
#[test]
fn reference_layout_needs_an_entry_for_every_summary_command() {
    let text = |entry: &str| {
        format!(
            concat!(
                "Commands (help <command> lists them):\n",
                "  drafts                        kept drafts\n\n",
                "Workflow\n\nCommands in full\nReading\n{}",
            ),
            entry
        )
    };
    assert_eq!(
        reference_layout(&text(&ref_row("read <path>", "a file"))),
        (false, "line 2: no entry in \"Commands in full\" for `drafts`\n".into())
    );
    let entry = format!("{}{}", ref_row("read <path>", "a file"), ref_row("drafts", "kept drafts"));
    assert_eq!(reference_layout(&text(&entry)), (true, String::new()));
    // `help` never shows host commands, so an entry there answers nothing.
    let host = format!("{}\nHost commands (humans only)\n{}", ref_row("read <path>", "a file"), ref_row("drafts", "kept drafts"));
    assert_eq!(reference_layout(&text(&host)), (false, "line 2: no entry in \"Commands in full\" for `drafts`\n".into()));
}
#[test]
fn reference_layout_names_a_continuation_one_column_off() {
    let text = |spaces| sample(&[ref_row("read <path>", "a file, or"), ref_line(spaces, "some of its lines")]);
    assert_eq!(reference_layout(&text(18)), (true, String::new()));
    assert_eq!(reference_layout(&text(19)), (false, "line 6: starts at column 20; the descriptions start at column 19\n".into()));
    assert_eq!(reference_layout(&text(17)), (false, "line 6: starts at column 18; the descriptions start at column 19\n".into()));
}
#[test]
fn reference_layout_takes_an_example_block_after_a_colon_at_one_column() {
    let text = |last| {
        sample(&[
            ref_row("write", "a patch, for"),
            ref_line(18, "example:"),
            ref_line(20, "*** Begin Patch"),
            ref_line(last, "*** End Patch"),
            ref_line(18, "and more"),
        ])
    };
    assert_eq!(reference_layout(&text(20)), (true, String::new()));
    assert_eq!(reference_layout(&text(21)), (false, "line 8: starts at column 22; the example starts at column 21\n".into()));
    // One column deeper after a `:` is the slip again, not an example.
    let text = sample(&[ref_row("write", "a patch, for"), ref_line(18, "example:"), ref_line(19, "files")]);
    assert_eq!(reference_layout(&text), (false, "line 7: starts at column 20; the descriptions start at column 19\n".into()));
}
#[test]
fn reference_layout_refuses_deeper_lines_without_a_colon_before_them() {
    // task:e86828d20aaab5a2's slip: two lines one space right of their paragraph.
    let text = sample(&[
        ref_row("strategy task", "a title is the work"),
        ref_line(18, "in 3-7 words (`extract"),
        ref_line(19, "files`); details"),
        ref_line(19, "go in body"),
    ]);
    assert_eq!(
        reference_layout(&text),
        (
            false,
            concat!(
                "line 7: starts at column 20; the descriptions start at column 19\n",
                "line 8: starts at column 20; the descriptions start at column 19\n",
            )
            .into()
        )
    );
}
#[test]
fn reference_layout_names_a_row_whose_description_is_one_column_off() {
    let text = sample(&[
        ref_row("read", "a file"),
        format!("  {:<17}{}\n", "grep <text>", "search"),
        ref_line(18, "watched files"),
        ref_row("find <text>", "search code"),
    ]);
    // Its continuation is held to the description column, so it adds no fault of its own.
    assert_eq!(
        reference_layout(&text),
        (false, "line 6: its description starts at column 20; the descriptions start at column 19\n".into())
    );
}
#[test]
fn reference_layout_lets_rows_next_to_each_other_share_their_own_column() {
    let table = |third: &str| {
        sample(&[
            ref_row("read", "a file"),
            ref_row("grep", "search"),
            "Files: by tier\n".into(),
            "  graph     the store\n".into(),
            ref_line(12, "refused"),
            format!("  {third} solace.toml\n"),
            ref_line(12, "the human's"),
            "  internal  .solace/\n".into(),
            ref_row("files [dir]", "tree"),
        ])
    };
    assert_eq!(reference_layout(&table("protected")), (true, String::new()));
    // One space after a term that doesn't reach the table's column: the row has no description of its own.
    assert_eq!(
        reference_layout(&table("protectedx")),
        (false, "line 11: starts at column 13; the descriptions start at column 19\n".into())
    );
}
#[test]
fn reference_layout_refuses_a_line_after_prose_and_a_reference_without_the_section() {
    let text = sample(&[ref_row("read", "a file"), "Prose here\n".into(), ref_line(18, "stray")]);
    assert_eq!(
        reference_layout(&text),
        (false, "line 7: starts at column 19; after a heading or prose, a line starts at column 1 or 3\n".into())
    );
    let text = sample(&[ref_row("read", "a file"), "\n".into(), ref_line(18, "stray")]);
    assert_eq!(
        reference_layout(&text),
        (false, "line 7: starts at column 19; after a blank line, a line starts at column 1 or 3\n".into())
    );
    assert_eq!(reference_layout("Workflow only\n"), (false, "no \"Commands in full\" line\n".into()));
    assert_eq!(
        reference_layout("Commands in full\n  grep <text>\n"),
        (false, "no command row has its description on the same line\n".into())
    );
}
#[test]
fn broken_symbols_counted_once() {
    let d = |sym: &str| Diag { error: true, module: None, symbol: Some(sym.into()), text: format!("error: {sym}") };
    let out = broken_report(&HashMap::new(), &[d("a.f"), d("a.g"), d("a.f")]);
    assert!(out.contains("Broken symbols (2): a.f, a.g"), "{out}");
}
/// A batch runner that records each command and fails the one named `fail`.
fn fake<'a>(fail: &'static str, ran: &'a mut Vec<String>) -> impl FnMut(&[String], usize) -> Out + 'a {
    move |w: &[String], _| {
        ran.push(w.join(" "));
        if w[0] == fail { Err(Failure(format!("E6002 {fail} failed"), 1)) } else { Ok("fine".into()) }
    }
}
#[test]
fn a_failed_change_stops_the_call() {
    // D-119: a failed write skips the read and the run after it.
    let mut ran = vec![];
    let (out, error) = batch(&["write a.md", "read b.md", "run rust-unit"], 10_000, &mut fake("write", &mut ran));
    assert!(error);
    assert_eq!(ran, ["write a.md"]);
    assert!(!out.contains("==== solace read") && !out.contains("==== solace run"), "{out}");
    assert!(out.ends_with("E6002 write failed\n1 of 3 commands failed: write a.md; skipped: read b.md; run rust-unit\n"), "{out}");
    // A failed read does not stop the call; with nothing skipped, the line is D-109's.
    let mut ran = vec![];
    let (out, error) = batch(&["read b.md", "write a.md"], 10_000, &mut fake("read", &mut ran));
    assert!(error);
    assert_eq!(ran, ["read b.md", "write a.md"]);
    assert!(out.ends_with("1 of 2 commands failed: read b.md\n"), "{out}");
    // A failed change as the last command skips nothing; a failed run stops nothing.
    let mut ran = vec![];
    let (out, _) = batch(&["read b.md", "solace write a.md"], 10_000, &mut fake("write", &mut ran));
    assert!(out.ends_with("1 of 2 commands failed: write a.md\n"), "{out}");
    let mut ran = vec![];
    let (out, _) = batch(&["run rust-unit", "read b.md"], 10_000, &mut fake("run", &mut ran));
    assert_eq!(ran, ["run rust-unit", "read b.md"]);
    assert!(out.ends_with("1 of 2 commands failed: run rust-unit\n"), "{out}");
    // Nothing failed: no last line, and no error.
    let (out, error) = batch(&["read b.md", "write a.md"], 10_000, &mut fake("none", &mut vec![]));
    assert!(!error && !out.contains("commands failed"), "{out}");
}
/// task:da797576d0cfaaac: a line that is only `--project <dir>` fails
/// alone, and the rest of the call runs.
#[test]
fn a_project_with_no_command_is_refused_and_the_call_goes_on() {
    let mut ran = vec![];
    let (out, error) = batch(&["--project solace-cli/board", "read b.md"], 10_000, &mut fake("none", &mut ran));
    assert!(error);
    assert_eq!(ran, ["read b.md"]);
    assert!(out.contains(PROJECT_ONLY), "{out}");
    assert!(out.ends_with("1 of 2 commands failed: --project solace-cli/board\n"), "{out}");
    // A line with no `--project` at all is no such line: it runs as before.
    let mut lens = vec![];
    let (_, error) = batch(&["solace"], 10_000, &mut |w: &[String], _| -> Out {
        lens.push(w.len());
        Ok(String::new())
    });
    assert!(!error && lens == [0]);
}
/// task:da797576d0cfaaac: a panic is that line's error and stops the call;
/// the tool outlives it, so the next call runs.
#[test]
fn a_panicking_command_is_an_error_and_stops_the_call() {
    let mut ran = vec![];
    let (out, error) = batch(&["read a.md", "view x", "read b.md"], 10_000, &mut |w: &[String], _| -> Out {
        ran.push(w.join(" "));
        if w[0] == "view" {
            panic!("boom");
        }
        Ok("fine".to_string())
    });
    assert!(error);
    assert_eq!(ran, ["read a.md", "view x"]);
    assert!(out.contains("fine"), "{out}");
    assert!(
        out.ends_with(concat!(
            "Solace hit an internal error running `view x`: boom; nothing after it ran\n",
            "1 of 3 commands failed: view x; skipped: read b.md\n",
        )),
        "{out}"
    );
    let mut ran = vec![];
    let (out, error) = batch(&["read a.md"], 10_000, &mut fake("none", &mut ran));
    assert!(!error && ran == ["read a.md"], "{out}");
}
/// task:da797576d0cfaaac: a panic drops the call's thread-local state,
/// rolling back an outer transaction it left open.
#[test]
fn a_panic_resets_the_calls_state() {
    let store = project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    store.db.execute_batch("BEGIN IMMEDIATE").unwrap();
    COMMAND_DB.set(vec![(store.root.clone(), store.db.clone())]);
    UNTESTED.with(|u| *u.borrow_mut() = Some("app.main".into()));
    reset_after_panic();
    assert!(store.db.is_autocommit(), "the open transaction is rolled back");
    assert!(COMMAND_DB.with(|c| c.borrow().is_empty()));
    assert!(UNTESTED.with(|u| u.borrow().is_none()));
}
/// A batch runner whose `test` passes or fails, leaving `untested` named as
/// the task's changes no test reaches.
fn fake_test<'a>(pass: bool, untested: Option<&'static str>, ran: &'a mut Vec<String>) -> impl FnMut(&[String], usize) -> Out + 'a {
    move |w: &[String], _| {
        ran.push(w.join(" "));
        if !w.iter().any(|x| x == "test") {
            return Ok("fine".into());
        }
        UNTESTED.with(|u| *u.borrow_mut() = untested.map(String::from));
        if pass { Ok("passed".into()) } else { Err(Failure("1 failure".into(), 1)) }
    }
}
#[test]
fn a_batched_task_done_waits_for_a_passing_test() {
    // D-152: a failed test holds back the task done after it.
    let mut ran = vec![];
    let (out, error) = batch(&["submit", "test", "task done fixed"], 10_000, &mut fake_test(false, None, &mut ran));
    assert!(error);
    assert_eq!(ran, ["submit", "test"]);
    assert!(!out.contains("==== solace task done"), "{out}");
    assert!(out.ends_with(&format!("1 of 3 commands failed: test; skipped: task done fixed (the test failed)\n{HELD}")), "{out}");
    // A pass that leaves a change no test reaches holds it back too, and is no error.
    let mut ran = vec![];
    let (out, error) = batch(&["submit", "test", "task done fixed"], 10_000, &mut fake_test(true, Some("app.three"), &mut ran));
    assert!(!error);
    assert_eq!(ran, ["submit", "test"]);
    assert!(out.ends_with(&format!("Skipped: task done fixed (no test reaches app.three)\n{HELD}")), "{out}");
    // A clean pass finishes, a test in another project too.
    let mut ran = vec![];
    let (out, error) = batch(&["submit", "--project lib test", "task done fixed"], 10_000, &mut fake_test(true, None, &mut ran));
    assert!(!error);
    assert_eq!(ran, ["submit", "--project lib test", "task done fixed"]);
    assert!(!out.contains("Skipped") && !out.contains(HELD), "{out}");
    // A task done before the test, or in a call with no test, runs.
    let mut ran = vec![];
    batch(&["task done fixed", "test"], 10_000, &mut fake_test(false, None, &mut ran));
    assert_eq!(ran, ["task done fixed", "test"]);
    let mut ran = vec![];
    let (out, _) = batch(&["submit", "task done fixed"], 10_000, &mut fake_test(false, Some("app.three"), &mut ran));
    assert_eq!(ran, ["submit", "task done fixed"]);
    assert!(!out.contains("Skipped"), "{out}");
}
/// D-145: from the second successful single read in a row, the reply hints to batch.
#[test]
fn a_second_single_read_in_a_row_gets_the_hint() {
    let mut streak = 0;
    assert_eq!(read_hint(&mut streak, &["read a.md"], false), "");
    assert_eq!(read_hint(&mut streak, &["solace grep x"], false), READ_HINT);
    assert_eq!(read_hint(&mut streak, &["view a.f --project lib"], false), READ_HINT);
    // A batch, a failed read and a change each start the count again.
    assert_eq!(read_hint(&mut streak, &["read a.md", "read b.md"], false), "");
    assert_eq!(read_hint(&mut streak, &["read a.md"], false), "");
    assert_eq!(read_hint(&mut streak, &["read a.md"], true), "");
    assert_eq!(read_hint(&mut streak, &["read a.md"], false), "");
    assert_eq!(read_hint(&mut streak, &["write a.md"], false), "");
    assert_eq!(read_hint(&mut streak, &["--project lib inspect a.f"], false), "");
    assert_eq!(read_hint(&mut streak, &["--project lib inspect a.f"], false), READ_HINT);
    assert_eq!(read_hint(&mut streak, &[], false), "");
    assert_eq!(streak, 0);
}
/// A read command given an option it doesn't take is refused by name.
#[test]
fn an_option_a_command_doesnt_take_is_refused_by_name() {
    let e = unknown_option(&["view", "strategy.update", "--lines", "50..80"]).unwrap().unwrap_err().0;
    assert!(e.starts_with("view has no --lines; read takes --lines. view takes: --bodies"), "{e}");
    assert!(unknown_option(&["read", "a.md", "--lines", "1..2"]).is_none());
    assert!(unknown_option(&["inspect", "a.b", "--context"]).is_none());
    assert!(unknown_option(&["view", "a", "--bodies"]).is_none());
    assert!(unknown_option(&["view", "--draft", "x"]).is_none(), "view --draft is in the reference");
    assert!(unknown_option(&["grep", "--x"]).is_none());
}
/// The unknown-option refusal comes before other refusals, at both entry
/// points: `run_inner`, and `--project` through `read_project`.
#[test]
fn an_unknown_option_is_refused_before_other_refusals() {
    let args = vec!["view".to_string(), "app.main".to_string(), "--lines".to_string(), "1..2".to_string()];
    let e = run_inner(&args, &mut || Ok(String::new()), None, 10_000).unwrap_err().0;
    assert!(e.starts_with("view has no --lines; read takes --lines"), "{e}");
    let e = read_project("no-such-project", &args, 10_000).unwrap_err().0;
    assert!(e.starts_with("view has no --lines; read takes --lines"), "{e}");
}
#[test]
fn the_tool_asks_for_batched_reads_short_text_and_a_clean_stop() {
    let d = mcp::tool_schema()["description"].as_str().unwrap().to_string();
    assert!(d.contains("put every independent read in the same call") && d.contains("never one read per call"), "{d}");
    assert!(d.contains("Keep text between calls to a line or two") && d.contains("Once the work is done, stop"), "{d}");
}
#[test]
fn agent_reference_leaves_out_host_and_task_commands() {
    // Tests run outside a board run; `help` keeps everything.
    let r = agent_reference(false);
    assert!(agent_reference(true).contains("task done") && !agent_reference(true).contains("Host commands"));
    assert!(r.contains("Several commands go in one call") && r.contains("work records"));
    assert!(!r.contains("task done") && !r.contains("Host commands"));
    assert!(REFERENCE.contains("task done") && REFERENCE.contains("agent-config"));
    // D-46: session start gets the workflow, with the signature-change and
    // test examples (D-48: a test at the level reported, one per shifted
    // behavior); D-102: the file tiers. D-123: no byte cap; it is kept as
    // brief as understanding it allows, and `help` keeps every option.
    assert!(r.contains("Files by tier") && r.contains("ignored or not"));
    assert!(r.contains("every part\n   of the change is made") && !r.contains("of the task is changed"));
    // D-158 point 6: the reference names both languages and the Gleam-only commands.
    assert!(r.contains("Files by tier: a Gleam project's src/ and test/ *.gleam, and a Rust project's"), "both languages: {r}");
    assert!(REFERENCE.contains("Same-project Rust (D-185)") && REFERENCE.contains("Gleam only: Rust merge is deferred"), "{REFERENCE}");
    assert!(
        r.contains("Changing a signature or adding a field")
            && r.contains("one at the level the task reports")
            && r.contains("`help <command>`")
    );
    assert!(!r.contains("probe --from <fn>") && REFERENCE.contains("probe --from <fn>") && REFERENCE.contains("--replace [--all]"));
    assert!(r.contains("Imports are added") && !r.contains("task ask"));
    // Scope, the code's own conventions, comments that say why, and no secrets.
    assert!(r.contains("stop exploring") && r.contains("Write code the way the code around it is written"));
    assert!(r.contains("A comment says why code exists") && r.contains("Never put secrets or keys"));
    // Craft rules: the smallest complete change, and what `test` can't reach.
    assert!(r.contains("Make the smallest complete change") && r.contains("Fix a reported bug once, at its cause"));
    assert!(r.contains("`test` doesn't reach docs, config, JavaScript or golden files"));
    // D-119 point 5: plain-file projects batch too, every edit in one patch.
    assert!(r.contains("then `run <name>`") && r.contains("go in one patch") && r.contains("a patch of many edits"));
    // D-123: where a line runs, and that options belong to one command.
    assert!(
        r.contains("Each line runs in one project") && r.contains("`--lines` is `read`'s") && r.contains("`help <command>`\nlists them")
    );
    // `help` for agents: every option, but no host or task commands here.
    let h = agent_help(None);
    assert!(h.contains("probe --from <fn>") && !h.contains("task diff") && !h.contains("agent-config"));
    let s = agent_help(Some("submit"));
    assert!(s.contains("--replace [--all]") && s.contains("draft") && !s.contains("probe --from"), "{s}");
}

/// D-144: `help <command>` answers the one paragraph holding the command's
/// entry; every command in the summary table has its own entry, so its
/// section is much shorter than the whole reference.
#[test]
fn help_answers_one_command_its_own_section() {
    let full = agent_help(None);
    let table = REFERENCE.split("Commands (").nth(1).unwrap();
    let rows: Vec<&str> = table.lines().skip_while(|l| !l.starts_with("  ")).take_while(|l| l.starts_with("  ")).collect();
    assert!(rows.len() >= 19, "{rows:?}");
    for row in rows {
        let cmd = row.split_whitespace().next().unwrap();
        let h = agent_help(Some(cmd));
        assert!(h.contains(&format!("  {cmd} ")), "{cmd}: {h}");
        assert!(h.len() * 2 < full.len(), "{cmd}: {} of {} bytes", h.len(), full.len());
    }
    let w = agent_help(Some("work"));
    assert!(w.contains("work done <id>") && !w.contains("inspect"), "{w}");
    let r = agent_help(Some("run"));
    assert!(r.contains("run <name>") && !r.contains("work add"), "{r}");
    let f = agent_help(Some("format"));
    assert!(f.contains("format") && f.contains("Rust") && f.contains("module"), "{f}");
    let n = agent_help(Some("nosuch"));
    assert!(
        n.starts_with("No command `nosuch`")
            && n.contains("inspect")
            && n.contains("work")
            && !n.contains("--context")
            && !n.contains("watched")
            && n.len() * 2 < full.len(),
        "{n}"
    );
}

/// D-123: an unknown name another project has says where it is, and the
/// session start says where the agent is and how `--project` and paths work.
#[test]
fn a_name_in_another_project_says_where_it_is() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"a\", \"b\"]\n"),
        ("a/gleam.toml", "name = \"alpha\"\nversion = \"1.0.0\"\n"),
        ("a/src/app.gleam", "pub fn one() -> Int {\n  1\n}\n"),
        ("b/gleam.toml", "name = \"beta\"\nversion = \"1.0.0\"\n"),
        ("b/src/plan.gleam", "pub fn answer() -> Int {\n  42\n}\n"),
    ]);
    for d in ["a", "b"] {
        import(&root.join(d)).unwrap();
    }
    let a = Store::at(&root.join("a")).unwrap();
    let e = hinted(a.view(&["plan.answer"], false, BUDGET), &a.root).unwrap_err().0;
    assert!(e.contains("E1001 unknown name: plan.answer\nfound in b: add --project b"), "{e}");
    let e = hinted(a.view(&["plan.nope"], false, BUDGET), &a.root).unwrap_err().0;
    assert!(e.contains("E1001") && !e.contains("found in"), "{e}");
    let s = a.other_projects(None);
    for part in [
        "You are in a (alpha).\n",
        "Other projects under this Solace root (read only): b (beta).",
        "(`--project b`), for that line only",
        "grep",
        "relative to the project the line runs in",
    ] {
        assert!(s.contains(part), "{part} missing from:\n{s}");
    }
}

#[test]
fn probe_args_parse() {
    assert_eq!(probe_args(&["--from", "t", "--trace", "a.b", "c"]).unwrap(), (Some("t"), vec!["a.b", "c"]));
    assert_eq!(probe_args(&[]).unwrap(), (None, vec![]));
    assert!(probe_args(&["--trace"]).is_err() && probe_args(&["--from"]).is_err() && probe_args(&["x"]).is_err());
}

#[test]
fn dependencies_show_their_public_api() {
    let store = project(&[
        ("src/app.gleam", "pub fn main() { 1 }\n"),
        (
            "build/packages/dep/src/dep/api.gleam",
            "//// Talks to things.\n////\n//// A long guide.\n\nimport gleam/list\n\n\
         /// Makes a widget.\n/// Second doc line.\npub fn make(size: Int) -> Widget {\n  secret(size)\n}\n\n\
         // SECTION ---\n\nfn secret(n: Int) -> Widget {\n  Widget(n)\n}\n\n\
         /// A widget.\npub opaque type Widget {\n  Widget(size: Int)\n}\n",
        ),
    ]);
    // A module: its first doc paragraph and public declarations, no bodies.
    let out = store.view(&["dep/api"], false, usize::MAX).ok().unwrap();
    assert!(out.starts_with("// ── module dep/api (dep, a dependency; read only)") && out.contains("//// Talks to things.\n\n"), "{out}");
    assert!(out.contains("/// Makes a widget.\npub fn make(size: Int) -> Widget\n") && out.contains("pub opaque type Widget\n"), "{out}");
    for hidden in ["secret", "A long guide", "SECTION", "Widget(size", "Second doc line"] {
        assert!(!out.contains(hidden), "{hidden}: {out}");
    }
    // A symbol: all of its doc; `inspect` falls back to the same view; store names still win.
    let one = store.inspect(&["dep/api.make", "app.main"], true, usize::MAX).ok().unwrap();
    assert!(one.contains("/// Second doc line.\npub fn make(size: Int) -> Widget") && !one.contains("secret(size)"), "{one}");
    assert!(one.contains("app.main"), "{one}");
    assert!(store.view(&["dep/api.secret"], false, usize::MAX).unwrap_err().0.contains("E1001"));
}

/// D-14: a multi-name inspect prints each signature once, and trims, then
/// defers names, to stay within the budget.
#[test]
fn inspect_dedups_and_fits_budget() {
    // A real project of some size: the bench pilot's code.
    let dir = Scratch::new();
    let pilot = Path::new(env!("CARGO_MANIFEST_DIR")).join("../bench/pilot-large");
    for top in ["src", "test"] {
        migrate::copy_tree(&pilot.join(top), &dir.join(top), &|_| false).unwrap();
    }
    fs::copy(pilot.join("gleam.toml"), dir.join("gleam.toml")).unwrap();
    let store = dir.import();
    let names = ["store.put_shipment", "shipments.dispatch", "shipments.deliver", "fulfillment.ship"];
    let ok = |r: Out| r.ok().unwrap();

    let out = ok(store.inspect(&names, true, usize::MAX));
    assert_eq!(out.matches("-- pilot/shipments.Shipment\n").count(), 1);
    assert!(out.contains("-- pilot/shipments.Shipment (above)"));
    assert!(!out.contains("Trimmed") && !out.contains("Not shown"));

    let small = ok(store.inspect(&names, true, 3_000));
    assert!(small.contains("Trimmed to fit the size limit"));
    assert!(small.contains("Not shown, to fit the size limit: `solace inspect "));
    assert!(small.len() < out.len());
    for n in names {
        assert!(small.contains(&format!("// ── pilot/{n} ")) || small.lines().last().unwrap().contains(&format!("pilot/{n}")), "{n} lost");
    }

    // an unknown name is reported after the others, not instead of them
    let mixed = ok(store.inspect(&["shipments.deliver", "shipping.standard_cost_cents"], false, usize::MAX));
    assert!(mixed.contains("// ── pilot/shipments.deliver ") && mixed.contains("E1001 unknown name: shipping.standard_cost_cents"));
    assert!(store.inspect(&["shipping.nope"], false, usize::MAX).is_err());
    // D-46: a type's users split into the sites that list every field and
    // the rest by name; a function's callers show the call line.
    let ty = ok(store.inspect(&["customer.Customer"], true, usize::MAX));
    assert!(ty.contains("== Constructed by (") && ty.contains("-- pilot/seed.customers, line 3: Customer("), "{ty}");
    assert!(ty.contains("-- pilot/accounts.register, line 11: customer.Customer(\n"), "{ty}");
    assert!(ty.contains("== Other users (") && !ty.contains("== Used by"), "{ty}");
    // commands in one MCP call share what was printed: `refs` says (above)
    SHOWN.with(|s| *s.borrow_mut() = Some(HashSet::new()));
    let f = ok(store.inspect(&["pricing.quote"], true, usize::MAX));
    assert!(f.contains("-- pilot/checkout.quote\n") && f.contains("  > line "), "{f}");
    let r = ok(store.refs("pricing.quote"));
    assert!(r.contains("-- pilot/checkout.quote (above)\n  > line "), "{r}");
    SHOWN.with(|s| *s.borrow_mut() = None);
    assert!(ok(store.refs("pricing.quote")).contains("-- pilot/checkout.quote\n"));
    // D-102: `refs <name>...` gives each symbol's users, in the order named
    let many = ok(store.refs_all(&["pricing.quote", "checkout.quote"], usize::MAX));
    assert!(many.starts_with("pilot/pricing.quote is used by") && many.contains("\npilot/checkout.quote is used by"), "{many}");
}

#[test]
fn code_is_told_from_commands() {
    assert!(looks_like_code("let assert Ok(q) = checkout.quote(db, o.id, Standard)") && looks_like_code("echo q"));
    assert!(!looks_like_code("probe --from checkout_test.quote_test --trace pricing.quote"));
}

#[test]
fn import_creates_settings_without_overwriting() {
    for existing in [None, Some("# custom\nresources = [\"docs/**\"]\n")] {
        let root =
            Scratch::new().with(&[("gleam.toml", "name = \"app\"\nversion = \"1.0.0\"\n"), ("src/app.gleam", "pub fn main() { 1 }\n")]);
        if let Some(text) = existing {
            fs::write(root.join("solace.toml"), text).unwrap();
        }
        let out = import(&root).unwrap();
        let text = fs::read_to_string(root.join("solace.toml")).unwrap();
        if let Some(existing) = existing {
            assert_eq!(text, existing);
            assert!(!out.contains("Created solace.toml"));
        } else {
            assert!(out.contains("Created solace.toml"));
            assert_eq!(lockdown::settings(&root).unwrap()["watch_limit"].as_integer(), Some(262144));
            assert!(lockdown::settings(&root).unwrap()["commands"].as_table().unwrap().is_empty());
        }
        let store = fs::read(root.join(".solace/store.db")).unwrap();
        assert!(import(&root).unwrap().contains("existing symbols were preserved"));
        assert_eq!(fs::read_to_string(root.join("solace.toml")).unwrap(), text);
        fs::remove_file(root.join("solace.toml")).unwrap();
        fs::write(root.join("src/app.gleam"), "pub fn changed_on_disk() { 2 }\n").unwrap();
        assert!(import(&root).unwrap().contains("Created solace.toml"));
        assert!(lockdown::settings(&root).is_ok());
        assert_eq!(fs::read(root.join(".solace/store.db")).unwrap(), store);
    }
}

#[test]
fn a_refused_rust_package_creates_no_settings_or_store() {
    let root =
        Scratch::new().with(&[("Cargo.toml", CARGO), ("src/main.rs", "fn main() {}\n\nlazy_static! {\n    static ref X: u8 = 1;\n}\n")]);
    let err = import_with(&root, false).unwrap_err();
    assert!(err.0.contains("src/main.rs: line 3") && err.0.contains("Not saved"), "{}", err.0);
    assert!(!root.join("solace.toml").exists());
    assert!(!root.join(".solace").exists());
}

#[test]
fn reimporting_a_stored_rust_package_keeps_it_even_if_a_file_is_now_refused() {
    let root = Scratch::new().with(&[("Cargo.toml", CARGO), ("src/main.rs", "fn main() {}\n")]);
    import_with(&root, false).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n\nlazy_static! {\n    static ref X: u8 = 1;\n}\n").unwrap();
    let out = import_with(&root, false).unwrap();
    assert!(out.contains("existing symbols were preserved"), "{out}");
}

#[test]
fn submit_new_creates_a_module() {
    let store = project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    // Code with errors creates nothing.
    assert!(store.submit_symbols(None, Some("calc"), "pub fn add(a: Int) -> Int { a <> 1 }").is_err());
    assert!(store.resolve_module(&store.index().unwrap(), "calc").is_none());
    let out = store.submit_symbols(None, Some("calc"), "pub fn add(a: Int, b: Int) -> Int { a + b }").unwrap();
    assert!(out.contains("created module calc (src/calc.gleam)"), "{out}");
    assert!(store.resolve_module(&store.index().unwrap(), "calc").is_some());
    let out = store.submit_symbols(None, Some("calc_test"), "pub fn add_test() { assert calc.add(1, 2) == 3 }").unwrap();
    assert!(out.contains("(test/calc_test.gleam)"), "{out}");
    assert!(store.submit_symbols(None, Some("Bad"), "pub fn x() { 1 }").unwrap_err().0.starts_with("E1003"));
    // A whole new module in one submit: module comment, imports and several symbols.
    let module = concat!(
        "//// Geometry helpers.\nimport calc\n\npub const unit = 1\n\n",
        "pub type Shape {\n  Square(Int)\n}\n\n",
        "pub fn area(s: Shape) -> Int {\n  let Square(n) = s\n  calc.add(n * n, unit)\n}\n",
    );
    let out = store.submit_symbols(None, Some("geo"), module).unwrap();
    assert!(out.contains("created module geo") && out.contains("geo.unit, geo.Shape, geo.area"), "{out}");
    let file = fs::read_to_string(store.root.join("src/geo.gleam")).unwrap();
    assert!(file.starts_with("//// Geometry helpers.\n") && file.contains("import calc\n"), "{file}");
    assert!(store.submit_symbols(None, Some("geo2"), "pub fn a() { 1 }\npub fn a() { 2 }").unwrap_err().0.starts_with("E0103"));
    // Blocks may name a module that doesn't exist yet; a named block still holds one symbol.
    let out = store
        .submit_symbols(None, None, "// ── shapes.side ──\npub fn side() { 2 }\n// ── shapes.twice ──\npub fn twice() { side() * 2 }\n")
        .unwrap();
    assert!(out.contains("created module shapes"), "{out}");
    assert!(
        store
            .submit_symbols(Some("calc.add"), None, "pub fn add(a: Int, b: Int) -> Int { a + b }\npub fn sub() { 1 }")
            .unwrap_err()
            .0
            .starts_with("E0101")
    );
}

/// `calc.add` and `scale`, a test helper, and a test with a local.
fn probe_project() -> Project {
    project(&[
        (
            "src/calc.gleam",
            "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n\npub fn scale(x: Int, by factor: Int) -> Int {\n  factor * x\n}\n",
        ),
        ("test/helper.gleam", "pub fn two() {\n  2\n}\n"),
        ("test/calc_test.gleam", "import calc\n\npub fn sum_test() {\n  let x = calc.add(1, 2)\n  assert x == 3\n}\n"),
    ])
}

#[test]
fn probe_runs_and_leaves_nothing() {
    let store = probe_project();
    let probe_file = store.root.join("test/solace_probe.gleam");
    let out = store.probe("echo calc.add(2, 3)", None, &[]).unwrap();
    assert!(out.contains("5"), "{out}");
    assert!(!probe_file.exists());
    let err = store.probe("echo calc.nope()", None, &[]).unwrap_err().0;
    assert!(err.contains("The probe ran as") && err.contains("import calc"), "{err}");
    // D-145: an unknown module value says how to print a value; another failure doesn't.
    assert!(err.contains("Unknown module value") && err.contains("Hint: print a value with `echo value`.\n\nThe probe ran as"), "{err}");
    let err = store.probe("echo nope", None, &[]).unwrap_err().0;
    assert!(err.contains("The probe ran as") && !err.contains("Hint:"), "{err}");
    assert!(!probe_file.exists());
    // A probe is test-side code: it may use test helpers.
    assert!(store.probe("echo helper.two()", None, &[]).unwrap().contains("2"));
    // `--from` starts from a function's body; a single expression is printed (D-43).
    let out = store.probe("echo x + 10", Some("calc_test.sum_test"), &[]).unwrap();
    assert!(out.contains("13"), "{out}");
    assert!(store.probe("", Some("calc.add"), &[]).unwrap_err().0.starts_with("E1005"));
    let out = store.probe("calc.add(20, 3)", None, &[]).unwrap();
    assert!(out.contains("23") && !out.contains("warning"), "{out}");
}

#[test]
fn probe_trace_shows_calls_and_puts_the_module_back() {
    let store = probe_project();
    let before = fs::read_to_string(store.root.join("src/calc.gleam")).unwrap();
    let out = store.probe("calc.add(2, 3)", None, &["calc.add"]).unwrap();
    assert!(out.contains("→ calc.add(2, 3)") && out.contains("← calc.add = 5"), "{out}");
    assert_eq!(fs::read_to_string(store.root.join("src/calc.gleam")).unwrap(), before);
    let out = store.probe("calc.scale(3, by: 2)", None, &["calc.scale"]).unwrap();
    assert!(out.contains("← calc.scale = 6"), "{out}");
    assert!(store.probe("1", None, &["calc_test"]).is_err());
}

#[test]
fn test_helpers_are_imported_only_by_test_code() {
    let store = probe_project();
    let out = store.submit_symbols(None, Some("calc_test"), "pub fn two_test() { assert helper.two() == 2 }").unwrap();
    assert!(out.contains("import helper"), "{out}");
    assert!(store.submit_symbols(None, Some("calc"), "pub fn four() { helper.two() * 2 }").is_err());
}

/// D-43: an assert whose expected side reads the value it checks gets a note.
#[test]
fn weak_asserts_get_a_note() {
    let store = probe_project();
    let out = store
        .submit_symbols(None, Some("calc_test"), "pub fn weak_test() {\n  let q = #(1, 2)\n  assert q.0 == q.1 - 1\n  assert q.1 == 2\n}")
        .unwrap();
    assert!(out.contains("`assert q.0 == q.1 - 1` reads `q` on both sides") && !out.contains("assert q.1 == 2`"), "{out}");
}

/// A small project: `calc.add` and three callers.
fn calc_project() -> Project {
    let callers =
        "pub fn one() -> Int {\n  add(0, 1)\n}\n\npub fn two() -> Int {\n  add(1, 1)\n}\n\npub fn three() -> Int {\n  add(1, 2)\n}\n";
    project(&[("src/calc.gleam", &format!("pub fn add(a: Int, b: Int) -> Int {{\n  a + b\n}}\n\n{callers}"))])
}

/// D-46: a submit reports what changed in the broken set; an error seen
/// before is one line, and errors with one message are printed in full once.
#[test]
fn signature_change_reports_changes_once() {
    let store = calc_project();
    let out = store.submit_symbols(Some("calc.add"), None, "pub fn add(a: Int, b: Int, c: Int) -> Int { a + b + c }").unwrap();
    assert!(out.contains("Broken symbols (3)") && out.contains("same error: 2 more"), "{out}");
    assert_eq!(out.matches("Expected 3 arguments, got 2").count(), 1, "{out}");
    assert!(out.contains("  at calc.three, line 2: add(1, 2)"), "{out}");
    let out = store.submit_symbols(Some("calc.one"), None, "pub fn one() -> Int { add(0, 1, 0) }").unwrap();
    assert!(out.contains("Fixed: calc.one.") && out.contains("Still broken, with the errors shown before"), "{out}");
    assert!(out.contains("  calc.two, line 2: Incorrect arity: Expected 3 arguments, got 2\n"), "{out}");
    assert!(!out.contains("│"), "{out}");
    // `check` stays complete.
    let full = store.check().unwrap_err().0;
    assert_eq!(full.matches("Expected 3 arguments, got 2").count(), 2, "{full}");
    let out = store
        .submit_symbols(
            None,
            None,
            "// ── calc.two ──\npub fn two() -> Int { add(1, 1, 0) }\n// ── calc.three ──\npub fn three() -> Int { add(1, 2, 0) }\n",
        )
        .unwrap();
    assert!(out.contains("Fixed: calc.three, calc.two.\nThe project type-checks."), "{out}");
}

/// D-46: text equal to what is stored is reported unchanged, and when
/// nothing changed the type check is skipped.
#[test]
fn unchanged_submit_skips_check() {
    let store = calc_project();
    let blocks = "// ── calc.one ──\npub fn one() -> Int { add(0, 2) }\n// ── calc.two ──\npub fn two() -> Int { add(1, 1) }\n";
    let out = store.submit_symbols(Some("calc.one"), None, blocks).unwrap();
    assert!(
        out.contains("plain `submit` does the same") && out.contains("Saved calc.one.\nUnchanged (the same as stored): calc.two."),
        "{out}"
    );
    // A source change on disk would show if the project were checked again.
    fs::write(store.root.join("src/calc.gleam"), "not gleam").unwrap();
    let out = store.submit_symbols(Some("calc.two"), None, blocks).unwrap();
    assert!(out.contains("Unchanged: calc.one, calc.two.") && out.contains("nothing was saved or checked"), "{out}");
    assert_eq!(fs::read_to_string(store.root.join("src/calc.gleam")).unwrap(), "not gleam");
}

/// D-44: `test` runs the tests whose reach changed since they passed, those
/// that failed, and those reaching foreign code; the rest are skipped.
#[test]
fn test_runs_what_reaches_a_change() {
    let store = Scratch::new()
        .with(&[
            (
                "src/calc.gleam",
                concat!(
                    "pub fn add(a: Int, b: Int) -> Int {\n  a + b\n}\n\n",
                    "pub fn double(a: Int) -> Int {\n  add(a, a)\n}\n\n",
                    "pub fn sub(a: Int, b: Int) -> Int {\n  a - b\n}\n\n",
                    "@external(erlang, \"erlang\", \"abs\")\npub fn abs(x: Int) -> Int\n",
                ),
            ),
            (
                "test/calc_test.gleam",
                concat!(
                    "import calc\n\npub fn add_test() {\n  assert calc.add(1, 2) == 3\n}\n\n",
                    "pub fn double_test() {\n  assert calc.double(2) == 4\n}\n\n",
                    "pub fn sub_test() {\n  assert calc.sub(3, 1) == 3\n}\n\n",
                    "pub fn abs_test() {\n  assert calc.abs(-2) == 2\n}\n",
                ),
            ),
        ])
        .with_packages()
        .import();
    let first = |out: Out| out.unwrap_or_else(|e| e.0).lines().next().unwrap_or("").to_string();
    // First run: everything; sub_test fails.
    let err = store.test(&[]).unwrap_err().0;
    assert!(err.starts_with("Ran all 4 tests.") && err.contains("sub_test"), "{err}");
    // The failing test runs again, as does the one reaching foreign code.
    store.submit_symbols(Some("calc_test.sub_test"), None, "pub fn sub_test() {\n  assert calc.sub(3, 1) == 2\n}").unwrap();
    assert_eq!(
        first(store.test(&[])),
        "Ran 2 of 4 tests; the other 2 reach nothing changed since they passed (`test --all` runs every test)."
    );
    // A change reaches only the tests that can see it, and the files match the store afterwards.
    store.submit_symbols(Some("calc.add"), None, "pub fn add(a: Int, b: Int) -> Int {\n  b + a\n}").unwrap();
    let out = store.test(&[]).unwrap();
    assert!(out.starts_with("Ran 3 of 4 tests;") && out.contains("3 passed"), "{out}");
    let file = fs::read_to_string(store.root.join("test/calc_test.gleam")).unwrap();
    assert_eq!(file.matches("pub fn ").count(), 4, "{file}");
    assert_eq!(first(store.test(&["--all"])), "Ran all 4 tests.");
    assert_eq!(first(store.test(&["calc.double"])), "Ran 1 of 4 tests: those that reach calc.double.");
    assert!(store.test(&["calc.nope"]).unwrap_err().0.starts_with("E1001"));
    // With nothing changed, only the test reaching foreign code runs.
    assert!(first(store.test(&[])).starts_with("Ran 1 of 4 tests;"));
}

/// D-44: the packet quotes code that names the task's terms, and names a
/// test that reaches the top symbols to reproduce the case with.
#[test]
fn packet_quotes_evidence_and_reproduction() {
    let store = project(&[
        (
            "src/shop.gleam",
            concat!(
                "pub type Customer {\n  Customer(name: String, tier: Int)\n}\n\n",
                "pub const customers = [Customer(\"Grace\", 2), Customer(\"Ada\", 1)]\n\n",
                "/// The price after the tier discount.\n",
                "pub fn price(c: Customer, cents: Int) -> Int {\n  cents - c.tier * 100\n}\n",
            ),
        ),
        (
            "test/shop_test.gleam",
            "import shop\n\npub fn price_test() {\n  let assert [grace, ..] = shop.customers\n  assert shop.price(grace, 1000) == 800\n}\n",
        ),
    ]);
    let root = &store.root;
    let out = store.session_task("Grace (tier 2) was charged the wrong price.").unwrap();
    assert!(
        out.contains(
            "-- shop.customers\npub const customers = [Customer(\"Grace\", 2), Customer(\"Ada\", 1)]\n  > Customer(\"Grace\", 2)\n"
        ),
        "{out}"
    );
    assert!(out.contains("To reproduce the task's case: `probe --from shop_test.price_test --trace shop.price`"), "{out}");
    assert!(!out.contains("Project instructions"));
    // The project's AGENTS.md comes first in both orientations, cut at a whole line (D-45).
    fs::write(root.join("AGENTS.md"), "Prices are in cents.\n").unwrap();
    for out in [store.session_task("Grace").unwrap(), store.session(&[]).unwrap()] {
        assert!(out.starts_with("Project instructions (AGENTS.md") && out.contains("):\nPrices are in cents.\n\n"), "{out}");
    }
    fs::write(root.join("AGENTS.md"), "a line of forty bytes, give or take ok.\n".repeat(250)).unwrap();
    let out = store.session(&[]).unwrap();
    assert!(out.contains("ok.\n… `read AGENTS.md --lines 201..` for the rest\n"), "{out}");
}
/// D-145: every session start ends with a first call to send.
#[test]
fn session_start_ends_with_a_first_call() {
    let store = calc_project();
    let next = "\nNext: read what the change needs in one call, one command per line: ";
    let out = store.session(&[]).unwrap();
    assert!(out.ends_with(&format!("{next}for example `inspect checkout.pay payments.capture --context`.\n")), "{out}");
    // With --task, the call reads the top-ranked symbols.
    let out = store.session_task("Make calc.add handle overflow").unwrap();
    let last = out.rsplit_once(next).map(|(_, l)| l).unwrap_or_default();
    assert!(last.starts_with("`inspect calc.add ") && last.ends_with(" --context`.\n") && last.lines().count() == 1, "{out}");
    // A project of plain files reads with grep and read, with or without --task.
    fs::remove_file(store.root.join("gleam.toml")).unwrap();
    // the language is chosen when a command opens the store
    let store = Store::at(&store.root).unwrap();
    let plain = format!("{next}for example `grep <text> --in <path>` and `read <path> --lines a..b`.\n");
    for out in [store.session(&[]).unwrap(), store.session_task("calc.add").unwrap()] {
        assert!(out.contains("plain files") && out.ends_with(&plain), "{out}");
    }
}
/// D-151: session start shows whole open work records within a cap and
/// says how to read the rest; `work list` shows them all.
#[test]
fn session_start_caps_the_open_work_records() {
    let store = calc_project();
    store.work_add("Short record about calc.add").unwrap();
    let long = "x".repeat(WORK_CAP);
    store.work_add(&long).unwrap();
    let out = store.session(&[]).unwrap();
    assert!(out.contains("#1\nShort record about calc.add\n") && !out.contains(&long), "{out}");
    assert!(out.contains("1 of 2 open work records left out to fit the reply's size limit; `work list` shows them all.\n"), "{out}");
    assert!(out.contains("== Context of the symbols the work record names =="), "the shown record's symbols: {out}");
    let all = store.work_list().unwrap();
    assert!(all.contains(&long) && !all.contains("left out"), "{all}");
}
/// D-151: the board's call names the task, so a task scoped to this
/// project and another lists both, outside any run's environment.
#[test]
fn a_task_session_names_the_projects_of_its_task() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"a\", \"b\"]\n"),
        ("a/gleam.toml", "name = \"alpha\"\nversion = \"1.0.0\"\n"),
        ("a/src/app.gleam", "pub fn one() -> Int {\n  1\n}\n"),
        ("b/gleam.toml", "name = \"beta\"\nversion = \"1.0.0\"\n"),
        ("b/src/plan.gleam", "pub fn answer() -> Int {\n  42\n}\n"),
    ]);
    for d in ["a", "b"] {
        import(&root.join(d)).unwrap();
    }
    let a = Store::at(&root.join("a")).unwrap();
    board::open(&a.root).unwrap();
    let t = board::api(&a.root, "POST", "/tasks", json!({"title":"Answer"})).unwrap();
    tasks::update(&a.root, t["id"].as_str().unwrap(), |t| {
        t["scope"] = json!(["a", "b"]);
        Ok(())
    })
    .unwrap();
    let out = a.task_session("Change plan.answer", t["id"].as_str(), "").unwrap();
    assert!(out.contains("This task can change: a (alpha), b (beta).") && out.contains("-- plan.answer (b)"), "{out}");
    assert!(out.contains("task done"), "a board run's reference: {out}");
    let out = a.task_session("Change plan.answer", None, "").unwrap();
    assert!(out.contains("Other projects under this Solace root (read only): b (beta).") && !out.contains("task done"), "{out}");
}
/// D-145: a strategy run gets no workflow, so no first call.
#[test]
fn a_strategy_run_gets_no_first_call() {
    assert_eq!(next_call("`read a`", true), "");
    assert_eq!(next_call("`read a`", false), "\nNext: read what the change needs in one call, one command per line: `read a`.\n");
}

/// A whole module replaces every symbol it holds, and may not drop one;
/// `--imports` replaces only the header.
#[test]
fn whole_module_and_imports_submit() {
    let store = calc_project();
    let file = store.root.join("src/calc.gleam");
    let module = fs::read_to_string(&file).unwrap().replace("a + b", "b + a");
    let out = store.submit(Target::Module("calc"), &module).unwrap();
    assert_eq!(out, "Note: calc: changed add; added none\nSaved calc.\nThe project type-checks.\n");
    assert!(fs::read_to_string(&file).unwrap().contains("b + a"));
    let without_three = module.split("pub fn three").next().unwrap();
    assert!(store.submit(Target::Module("calc"), without_three).unwrap_err().0.contains("E0102 the module text leaves out: three"));
    assert!(store.submit(Target::Module("calc"), &format!("{module}\npub fn one() {{ 1 }}")).unwrap_err().0.starts_with("E0103"));
    // A rejected module leaves the stored code, and its file, as they were.
    let before = fs::read_to_string(&file).unwrap();
    let err = store.submit(Target::Module("calc"), &module.replace("b + a", "b +. a")).unwrap_err().0;
    assert!(err.starts_with("Not saved: the submitted text has errors."), "{err}");
    assert_eq!(fs::read_to_string(&file).unwrap(), before);
    store.submit_symbols(None, Some("other"), "pub fn x() { 1 }").unwrap();
    let out = store.submit(Target::Imports("calc"), "//// Arithmetic.\nimport other").unwrap();
    assert!(out.starts_with("Saved calc imports."), "{out}");
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("//// Arithmetic.\n\nimport other\n"), "{text}");
    assert!(store.submit(Target::Imports("calc"), "pub fn x() { 1 }").unwrap_err().0.starts_with("E0101"));
    assert!(store.submit(Target::Imports("nope"), "").unwrap_err().0.starts_with("E1003"));
}

/// D-44: `impact` follows a symbol's data to what uses it.
#[test]
fn impact_names_what_a_symbol_reaches() {
    let store = calc_project();
    let out = store.impact(&["calc.add"], BUDGET).unwrap();
    assert!(out.starts_with("calc.add reaches 3 uses, nearest first:"), "{out}");
    assert!(out.contains("calc.one") && out.contains("calc.two") && out.contains("calc.three"), "{out}");
    assert!(store.impact(&[], BUDGET).unwrap().starts_with("Tests haven't passed in full yet"));
    assert!(store.impact(&["calc.nope"], BUDGET).unwrap_err().0.starts_with("E1001"));
}

/// D-12: work records last between sessions; a session start shows the
/// open ones with the symbols they name.
#[test]
fn work_records_open_and_close() {
    let store = calc_project();
    assert_eq!(store.work_add("Make calc.add handle overflow").unwrap(), "Work record #1 added.\n");
    let list = store.work_list().unwrap();
    assert!(list.contains("Open work records:\n#1\nMake calc.add handle overflow\n"), "{list}");
    let session = store.session(&[]).unwrap();
    assert!(session.contains("== Context of the symbols the work record names ==") && session.contains("pub fn add("), "{session}");
    assert_eq!(store.work_done("#1").unwrap(), "Work record #1 closed.\n");
    assert!(!store.work_list().unwrap().contains("#1"));
    assert_eq!(store.work_done("9").unwrap_err().0, "No work record #9.");
    assert_eq!(store.work_done("x").unwrap_err().1, 2);
}

#[test]
fn delete_refuses_a_used_symbol_and_names_are_unambiguous() {
    let store = calc_project();
    let err = store.delete("calc.add").unwrap_err().0;
    assert!(err.starts_with("E5001 calc.add is still used by:") && err.contains("calc.three"), "{err}");
    assert!(store.delete("calc.one").unwrap().starts_with("Deleted calc.one."));
    assert!(store.resolve(&store.index().unwrap(), "calc.one").is_err());
    store.submit_symbols(None, Some("other"), "pub fn two() -> Int { 2 }").unwrap();
    let err = store.resolve(&store.index().unwrap(), "two").unwrap_err().0;
    assert!(err.starts_with("E1002 ambiguous name: two") && err.contains("calc.two") && err.contains("other.two"), "{err}");
    // D-102: `delete <name>...` deletes each named symbol in order; a name
    // used outside the list stops the whole list, so nothing is deleted.
    let store = calc_project();
    let err = store.delete_all(&["calc.add", "calc.one"], usize::MAX).unwrap_err().0;
    assert!(err.starts_with("E5001 calc.add is still used by:") && err.contains("calc.three") && !err.contains("calc.one"), "{err}");
    assert!(store.resolve(&store.index().unwrap(), "calc.one").is_ok());
    let out = store.delete_all(&["calc.one", "calc.two"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted calc.one.\nDeleted calc.two."), "{out}");
    // A helper goes with its last caller in one list.
    let out = store.delete_all(&["calc.add", "calc.three"], usize::MAX).unwrap();
    assert!(out.starts_with("Deleted calc.add.\nDeleted calc.three"), "{out}");
}

fn module_paths(s: &Store) -> Vec<String> {
    s.index().unwrap().modules.into_iter().map(|m| m.path).collect()
}

/// Deleting a module's last symbol retires the module, and its file and
/// emptied folder go with it; a module another one imports stays.
#[test]
fn deleting_the_last_symbol_retires_the_module_and_its_file() {
    let store = calc_project();
    store.submit_symbols(None, Some("other/deep"), "pub fn four() -> Int { 4 }").unwrap();
    assert!(store.root.join("src/other/deep.gleam").exists());
    let out = store.delete("other/deep.four").unwrap();
    assert!(out.starts_with("Deleted other/deep.four; module other/deep retired."), "{out}");
    assert!(!module_paths(&store).contains(&"other/deep".to_string()));
    assert!(!store.root.join("src/other").exists());
    let written = fs::read_to_string(store.root.join(".solace/written")).unwrap();
    assert!(!written.contains("src/other/deep.gleam"), "{written}");
    store.submit_symbols(None, Some("lib"), "pub fn five() -> Int { 5 }").unwrap();
    store.set_header("calc", "import lib\n").unwrap();
    assert!(store.delete("lib.five").unwrap().starts_with("Deleted lib.five.\n"));
    assert!(module_paths(&store).contains(&"lib".to_string()) && store.root.join("src/lib.gleam").exists());
}

/// Prune keeps a file edited since Solace wrote it, for `absorb` to read
/// back in and report; a rolled-back change leaves no file behind.
#[test]
fn prune_keeps_an_edited_file_and_a_rollback_leaves_none() {
    let store = calc_project();
    store.submit_symbols(None, Some("other"), "pub fn four() -> Int { 4 }").unwrap();
    let file = store.root.join("src/other.gleam");
    fs::write(&file, "pub fn four() -> Int {\n  44\n}\n").unwrap();
    let ix = store.index().unwrap();
    store.retire(&ix.syms[store.resolve(&ix, "other.four").unwrap()].id, "").unwrap();
    store.db.execute("DELETE FROM modules WHERE path = 'other'", []).unwrap();
    store.materialize(&store.index().unwrap()).unwrap();
    assert!(file.exists());
    let note = outside::absorb(&store, outside::Mode::Notice).unwrap();
    assert!(note.contains("other.four"), "{note}");
    assert!(module_paths(&store).contains(&"other".to_string()));
    let out = store.transaction::<String>(&[], || {
        store.submit_symbols(None, Some("temp"), "pub fn t() -> Int { 1 }")?;
        fail("stop")
    });
    assert!(out.is_err() && !store.root.join("src/temp.gleam").exists());
}

/// `gleam build` keeps a deleted module's `.beam` and artefacts; prune
/// removes them with its file.
#[test]
fn prune_drops_the_build_output_of_a_retired_module() {
    let store = calc_project();
    store.submit_symbols(None, Some("other"), "pub fn four() -> Int { 4 }").unwrap();
    assert!(store.tool(&["build"]).unwrap().0);
    let left = |s: &Store| {
        let mut out = vec![];
        let mut todo = vec![s.root.join("build/dev")];
        while let Some(d) = todo.pop() {
            for e in fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    todo.push(p);
                } else if p.file_name().unwrap().to_string_lossy().starts_with("other.") {
                    out.push(p.strip_prefix(&s.root).unwrap().to_string_lossy().to_string());
                }
            }
        }
        out
    };
    assert!(!left(&store).is_empty());
    store.delete("other.four").unwrap();
    assert!(store.tool(&["build"]).unwrap().0);
    assert!(left(&store).is_empty(), "{:?}", left(&store));
}

/// An unqualified type or constructor the module doesn't import is imported
/// for it: from the standard library, or the one project module that has it.
#[test]
fn missing_type_imports_are_added() {
    let store = Scratch::new()
        .with(&[
            ("src/shape.gleam", "pub type Shape {\n  Square(Int)\n}\n"),
            ("src/app.gleam", "import shape\n\npub fn main() {\n  1\n}\n"),
        ])
        .with_packages()
        .import();
    let out = store.submit_symbols(None, Some("app"), "pub fn unit() -> Shape { Square(1) }").unwrap();
    assert!(
        out.contains("Note: imported `type Shape` from shape in app") && out.contains("Note: imported `Square` from shape in app"),
        "{out}"
    );
    let file = fs::read_to_string(store.root.join("src/app.gleam")).unwrap();
    assert!(file.starts_with("import shape.{type Shape, Square}\n"), "{file}");
    let out = store.submit_symbols(None, Some("app"), "pub fn none() -> Option(Int) { None }").unwrap();
    assert!(out.contains("from gleam/option in app") && out.contains("type-checks"), "{out}");
}

#[test]
fn task_words() {
    assert_eq!(
        words("Add the TENOFF coupon to refundsByMonth in order_totals, 10 tests"),
        ["tenoff", "coupon", "refund", "month", "order", "total"]
    );
}

/// D-151: a task may start in a plain root and still hold symbols in a Rust
/// project, whose modules, symbols and brief the session lists.
#[test]
fn a_plain_root_names_the_rust_project_of_its_task() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"r\"]\n"),
        ("r/Cargo.toml", testing::CARGO),
        ("r/src/lib.rs", "//! A helper crate.\n\n/// Helps.\npub fn helper() -> i32 {\n    1\n}\n"),
    ]);
    import(&root).unwrap();
    import(&root.join("r")).unwrap();
    let store = Store::at(&root).unwrap();
    board::open(&root).unwrap();
    let t = board::api(&root, "POST", "/tasks", json!({"title":"Change helper"})).unwrap();
    tasks::update(&root, t["id"].as_str().unwrap(), |t| {
        t["scope"] = json!([".", "r"]);
        Ok(())
    })
    .unwrap();
    let out = store.task_session("Change helper", t["id"].as_str(), "").unwrap();
    assert!(out.contains("-- crate.helper (r)"), "{out}");
    assert!(out.contains("r:\n  crate — A helper crate."), "{out}");
    assert!(out.contains("Rust in Solace: every item has its own name"), "{out}");
    assert!(out.contains("`inspect crate.helper --project r --context`"), "{out}");
    assert!(!out.contains("This project has plain files"), "{out}");
}

/// A task whose projects are all plain keeps the plain session.
#[test]
fn a_task_of_only_plain_projects_stays_plain() {
    let root = testing::Scratch::new().with(&[("solace.toml", "projects = [\"notes\"]\n"), ("notes/README.md", "Order notes.\n")]);
    import(&root).unwrap();
    import(&root.join("notes")).unwrap();
    let store = Store::at(&root).unwrap();
    board::open(&root).unwrap();
    let t = board::api(&root, "POST", "/tasks", json!({"title":"Change notes"})).unwrap();
    tasks::update(&root, t["id"].as_str().unwrap(), |t| {
        t["scope"] = json!([".", "notes"]);
        Ok(())
    })
    .unwrap();
    let out = store.task_session("Change notes", t["id"].as_str(), "").unwrap();
    assert!(out.contains("This project has plain files"), "{out}");
    assert!(!out.contains("no Gleam code"), "{out}");
    assert!(!out.contains("Rust in Solace"), "{out}");
}

/// A Gleam project's task that also covers a Rust sibling lists both
/// projects' modules and carries the Rust brief.
#[test]
fn a_gleam_task_with_a_rust_sibling_lists_both() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"g\", \"r\"]\n"),
        ("g/gleam.toml", "name = \"app\"\nversion = \"1.0.0\"\n"),
        ("g/src/app.gleam", "/// Totals an order.\npub fn total() -> Int {\n  1\n}\n"),
        ("r/Cargo.toml", testing::CARGO),
        (
            "r/src/lib.rs",
            concat!(
                "//! A helper crate.\n\n/// Helps.\npub fn helper() -> i32 {\n    1\n}\n",
                "pub struct Store;\n\nimpl Store {\n    pub fn open() -> Self {\n        Store\n    }\n}\n",
            ),
        ),
    ]);
    import(&root.join("g")).unwrap();
    import(&root.join("r")).unwrap();
    let g = Store::at(&root.join("g")).unwrap();
    board::open(&g.root).unwrap();
    let t = board::api(&g.root, "POST", "/tasks", json!({"title":"Change total"})).unwrap();
    tasks::update(&g.root, t["id"].as_str().unwrap(), |t| {
        t["scope"] = json!(["g", "r"]);
        Ok(())
    })
    .unwrap();
    let out = g.task_session("Change total", t["id"].as_str(), "").unwrap();
    assert!(out.contains("g:\n  app — "), "{out}");
    assert!(out.contains("r:\n  crate — A helper crate."), "{out}");
    assert!(out.contains("Rust in Solace: every item has its own name"), "{out}");
    assert!(out.contains("crate.Store::open"), "{out}");
}

/// D-158: `--project <rust>` reads Rust items with inspect, refs and find.
#[test]
fn read_project_reads_a_rust_project() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"r\"]\n"),
        ("r/Cargo.toml", testing::CARGO),
        ("r/src/lib.rs", "pub fn helper() -> i32 {\n    1\n}\n\npub fn caller() -> i32 {\n    helper()\n}\n"),
    ]);
    import(&root).unwrap();
    import(&root.join("r")).unwrap();
    for (args, want) in [
        (&["inspect", "crate.helper"][..], "crate.helper (fn)"),
        (&["refs", "crate.helper"][..], "crate.caller"),
        (&["find", "helper"][..], "crate.helper"),
    ] {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let got = crate::with_root(&root, || crate::read_project("r", &args, 10_000)).unwrap();
        assert!(got.contains(want), "{args:?}: {got}");
    }
}

/// A task in a plain root names its Rust project and how to run there.
#[test]
fn a_plain_task_refusal_names_the_project_to_run_in() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"r\"]\n"),
        ("r/Cargo.toml", testing::CARGO),
        ("r/src/lib.rs", "pub fn helper() -> i32 {\n    1\n}\n"),
    ]);
    import(&root).unwrap();
    import(&root.join("r")).unwrap();
    let store = Store::at(&root).unwrap();
    board::open(&root).unwrap();
    let t = board::api(&root, "POST", "/tasks", json!({"title":"Change helper"})).unwrap();
    tasks::update(&root, t["id"].as_str().unwrap(), |t| {
        t["scope"] = json!([".", "r"]);
        Ok(())
    })
    .unwrap();
    for command in ["submit", "check", "test", "delete"] {
        let e = lockdown::plain::<()>(&store, Some(t["id"].as_str().unwrap()), command).unwrap_err().0;
        assert!(e.contains("files, grep, read, write and run. This task can change: . (the root), r."), "{command}: {e}");
        assert!(e.contains("`--project r`"), "{command}: {e}");
        assert!(!e.contains("Gleam"), "{command}: {e}");
    }
}

#[test]
fn a_rust_refusal_says_gleam_only() {
    let (_root, store) = testing::rust_project("pub fn helper() -> i32 {\n    1\n}\n");
    for command in ["extract", "merge", "probe"] {
        let e = lockdown::plain::<()>(&store, None, command).unwrap_err().0;
        assert!(e.contains("Gleam only") && e.contains("deferred (D-158 point 6)"), "{command}: {e}");
        assert!(!e.contains("no Gleam code"), "{command}: {e}");
    }
}

#[test]
fn a_rust_destination_says_gleam_only() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"web\", \"r\"]\n"),
        ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n"),
        ("web/src/app.gleam", "pub fn main() {\n  1\n}\n"),
        ("r/Cargo.toml", testing::CARGO),
        ("r/src/lib.rs", "pub fn helper() -> i32 {\n    1\n}\n"),
    ]);
    import(&root.join("web")).unwrap();
    import(&root.join("r")).unwrap();
    let store = Store::at(&root.join("web")).unwrap();
    for (command, args) in
        [("extract", vec!["app.main", "--to", "x", "--to-project", "r"]), ("merge", vec!["app.main", "app.main", "--to-project", "r"])]
    {
        let e = if command == "extract" {
            store.extract(&args, usize::MAX).unwrap_err().0
        } else {
            store.merge(&args, usize::MAX).unwrap_err().0
        };
        if command == "extract" {
            assert!(e.contains("Rust project") && e.contains("--to-project") && e.contains("D-185"), "extract: {e}");
        } else {
            assert!(e.contains("Gleam only") && e.contains("deferred (D-158 point 6)"), "merge: {e}");
        }
        assert!(!e.contains("no Gleam code"), "{command}: {e}");
    }
}

#[test]
fn a_plain_refusal_omits_a_project_without_code() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"r\"]\n"),
        ("r/Cargo.toml", testing::CARGO),
        ("r/src/lib.rs", "pub fn helper() -> i32 {\n    1\n}\n"),
    ]);
    import(&root).unwrap();
    import(&root.join("r")).unwrap();
    let store = Store::at(&root.join("r")).unwrap();
    board::open(&root).unwrap();
    let t = board::api(&root, "POST", "/tasks", json!({"title":"Change helper"})).unwrap();
    tasks::update(&root, t["id"].as_str().unwrap(), |t| {
        t["scope"] = json!([".", "r"]);
        Ok(())
    })
    .unwrap();
    let e = lockdown::plain::<()>(&store, Some(t["id"].as_str().unwrap()), "write").unwrap_err().0;
    assert!(
        e.contains(concat!(
            "this Rust project has no code that `write` handles; use files, grep, read, write and run. ",
            "This task can change: . (the root), r.",
        )),
        "{e}"
    );
    assert!(!e.contains("`--project"), "{e}");
}

/// The Rust reference gives the names to use and says what Rust lacks.
#[test]
fn rust_reference_lists_the_names_and_what_is_missing() {
    for part in [
        "<module>.<item>",
        "<module>.<Type>::<method>",
        "<module>.tests::<name>",
        "`crate`",
        "the crate root",
        "testing.rust_project",
        "crate.Store::resolve_module",
        "drafts.tests::rejected_text_survives_and_retries_atomically",
        "submit --new <module>",
        "submit --new <module>.<Type>",
        "submit --new <module>.tests",
        "submit --imports <module>",
        "submit <module>",
        "submit <container>",
        "submit --new <parent>::<new>",
        "submit --new store::notes",
        "integration::x",
        "*** Add: a::b::c",
        "find <text>",
        "grep <text>",
        "find resolve_module",
        "grep rust-unit",
        "view <module>",
        "E1001",
        "Similar names",
        "format <module>",
        "lockdown.dispatch",
        "checkout.pay",
        "`lockdown::dispatch` is",
        "accepted for `lockdown.dispatch`",
        "graph source is read by symbol",
        "a .py or plain-file .rs file's outline",
        "`view <file>` are refused",
        "[graph]",
        "Not available for Rust: `probe`, `merge`.",
    ] {
        assert!(REFERENCE.contains(part), "{part} missing:\n{REFERENCE}");
    }
    for wrong in [
        "<module>::tests.<name>",
        "drafts::tests.",
        "submit --new <module>::tests.<test>",
        "submit --new <module>.tests::<test>",
        "board.rs::within",
        "board.rs::Store::view",
    ] {
        assert!(!REFERENCE.contains(wrong), "{wrong} still in:\n{REFERENCE}");
    }
}

/// D-180: Rust `format` refuses without a module and says why.
#[test]
fn a_rust_format_needs_a_module() {
    let (_root, store) = testing::rust_project("pub fn helper() -> i32 {\n    1\n}\n");
    let v = lockdown::settings(&store.root).unwrap();
    let e = lockdown::dispatch(&store, &v, &["format"], &mut || Ok(String::new()), None, 1000).unwrap().unwrap_err().0;
    assert!(e.contains("needs a module") && e.contains("rustfmt-clean"), "{e}");
}

/// D-180: `format` takes one module the store holds: extra operands are a
/// usage error, and a file such as `build.rs` is not a module.
#[test]
fn a_rust_format_takes_one_module_the_store_holds() {
    let (root, store) = testing::rust_project("pub fn helper() -> i32 {\n    1\n}\n");
    let v = lockdown::settings(&root).unwrap();
    let extra =
        lockdown::dispatch(&store, &v, &["format", "crate", "extra"], &mut || Ok(String::new()), None, 1000).unwrap().unwrap_err().0;
    assert!(extra.contains("format <module>") && extra.contains("name one module"), "{extra}");
    let build = lockdown::dispatch(&store, &v, &["format", "build"], &mut || Ok(String::new()), None, 1000).unwrap().unwrap_err().0;
    assert!(build.contains("E1003") && build.contains("build"), "{build}");
}

/// D-180: Rust `format` saves through the store, so it shares the command's
/// outer transaction; a Gleam `format` or a build stays a process outside it.
#[test]
fn a_rust_format_is_not_a_process() {
    let (root, _) = testing::rust_project("pub fn helper() -> i32 {\n    1\n}\n");
    assert!(!is_process(&["format", "crate"], &root));
    assert!(!is_process(&["format"], &root));
    assert!(is_process(&["build"], &root));
    let plain = testing::Scratch::new();
    assert!(is_process(&["format", "crate"], &plain));
}

/// D-180: `format` names the tool a machine lacks and saves nothing.
#[test]
fn a_rust_format_names_the_missing_tool() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (_root, store) = testing::rust_project("pub fn helper() -> i32 {\n    1\n}\n");
    let v = lockdown::settings(&store.root).unwrap();
    let format = || lockdown::dispatch(&store, &v, &["format", "crate"], &mut || Ok(String::new()), None, 1000).unwrap().unwrap_err().0;
    let no_cargo = crate::language::cargo::with_path(&[], &format);
    assert!(no_cargo.contains("E6013") && no_cargo.contains(crate::language::cargo::MISSING), "{no_cargo}");
    let dir = testing::Scratch::new().with(&[("cargo", "")]);
    let no_rustfmt = crate::language::cargo::with_path(&[dir.0.clone()], &format);
    assert!(no_rustfmt.contains("E6013") && no_rustfmt.contains(crate::language::cargo::RUSTFMT_MISSING), "{no_rustfmt}");
}

/// D-57: without an owning store, a read finds the root from the working folder.
#[test]
fn read_project_finds_its_root_from_the_working_folder() {
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"r\"]\n"),
        ("r/Cargo.toml", testing::CARGO),
        ("r/src/lib.rs", "pub fn helper() -> i32 {\n    1\n}\n"),
    ]);
    import(&root).unwrap();
    import(&root.join("r")).unwrap();
    let got = crate::read_project_in(&["inspect", "crate.helper"], &root.join("r"), None, "r", 10_000).unwrap();
    assert!(got.contains("crate.helper (fn)"), "{got}");
    let outside = testing::Scratch::new();
    let e = crate::read_project_in(&["inspect", "crate.helper"], &outside, None, "r", 10_000).unwrap_err();
    assert!(e.0.contains("No Solace root here"), "{}", e.0);
}

/// D-184: a pass in a work run names the next step, with a test first for a change no test reaches.
#[test]
fn a_pass_names_the_next_step() {
    assert_eq!(next_step(None, true), "Next: if every part of the task is changed, finish with task done.\n");
    assert_eq!(
        next_step(Some("app.two"), true),
        "Next: add a test for app.two; then, if every part of the task is changed, finish with task done.\n"
    );
    assert_eq!(next_step(None, false), "");
    assert_eq!(next_step(Some("app.two"), false), "");
}

#[test]
fn grouped_privacy_errors_name_the_fix_once() {
    let mut diags: Vec<Diag> = (0..6)
        .map(|n| Diag {
            error: true,
            module: None,
            symbol: Some(format!("crate.site{n}")),
            text: format!("error[E0603]: function `helper` is private\n  at crate.site{n}, line 1"),
        })
        .collect();
    diags.extend((0..6).map(|n| Diag {
        error: true,
        module: None,
        symbol: Some(format!("crate.call{n}")),
        text: format!("error[E0599]: no method named `open` found for struct `Store` in the current scope\n  at crate.call{n}, line 1"),
    }));
    diags.push(Diag { error: true, module: None, symbol: Some("crate.other".into()), text: "error: something else".into() });
    let out = broken_report(&HashMap::new(), &diags);
    assert!(out.contains("`helper` is private (6 places): make it `pub(crate)`"), "{out}");
    assert!(out.contains("`open` is missing on `Store` (6 places): add the method or fix the call"), "{out}");
    assert_eq!(out.matches("error: something else").count(), 1, "{out}");
    for n in 0..6 {
        assert!(out.contains(&format!("at crate.site{n}, line 1")), "{out}");
    }
}

/// D-180: `format` reads the store's text, so a broken file on disk is named
/// and repaired rather than formatted, and the store's items stay saved.
#[test]
fn a_rust_format_repairs_a_broken_file_from_the_store() {
    if !crate::language::cargo::installed() || !crate::language::cargo::rustfmt_installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n";
    let (root, store) = testing::rust_project(lib);
    let broken = "pub fn one() -> i32 {\n    1\n}\n\npub fn two( {} \n";
    fs::write(root.join("src/lib.rs"), broken).unwrap();
    let v = lockdown::settings(&store.root).unwrap();
    let out = lockdown::dispatch(&store, &v, &["format", "crate"], &mut || Ok(String::new()), None, 1000).unwrap().unwrap();
    assert!(out.contains("Repaired src/lib.rs") && out.contains("crate.two"), "{out}");
    assert_eq!(fs::read_to_string(root.join("src/lib.rs")).unwrap(), lib);
    let kept = testing::kept_copy(&root, &out);
    assert_eq!(fs::read_to_string(kept).unwrap(), broken);
    let two = store.index().unwrap().syms.into_iter().find(|s| s.name == "two").unwrap();
    assert_eq!(two.text, "pub fn two() -> i32 {\n    2\n}");
}

/// A command line with only `--project <dir>` is refused, not a panic.
#[test]
fn a_project_with_no_command_is_refused_by_the_cli() {
    let p = testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let t = board::api(&p.root, "POST", "/tasks", json!({"title":"Project"})).unwrap();
    let tid = t["id"].as_str().unwrap();
    let e = crate::with_root(&p.root, || {
        board::with_context(tid, "r", || {
            crate::run_options(&["--project".to_string(), ".".to_string()], &mut || Ok(String::new()), None, 10_000, &[])
        })
    })
    .unwrap_err();
    assert!(e.0.starts_with(PROJECT_ONLY.trim_end()));
    assert_eq!(e.1, 2);
}

/// A strategy run's help shows a worker command it can't run, and row names
/// ignore the `|` inside placeholders and option lists.
#[test]
fn strategy_help_shows_worker_commands_and_clean_command_names() {
    let missing = agent_help_for(Some("nosuch"), false, true);
    assert!(missing.starts_with("No command `nosuch`"), "{missing}");
    for bad in ["milestone>", "all]", "--tail"] {
        assert!(!missing.contains(bad), "{bad}: {missing}");
    }
    let submit = agent_help_for(Some("submit"), false, true);
    assert!(submit.starts_with("Worker command: a strategy run can't run it."), "{submit}");
    assert!(submit.contains("submit <name> --replace [--all]") && submit.contains("draft"), "{submit}");
    assert!(!submit.contains("probe --from"), "{submit}");
    let feature = agent_help_for(Some("feature"), false, true);
    assert!(feature.contains("strategy task|feature|milestone"), "{feature}");
    let strategy = agent_help_for(Some("strategy"), false, true);
    assert!(strategy.contains("strategy show run <task> <run> --failed"), "{strategy}");
    let host = agent_help_for(Some("create"), false, true);
    assert!(host.starts_with("No command `create`"), "{host}");
}

#[test]
fn the_reference_and_worker_schema_show_the_submit_call_fields() {
    let step = reference::REFERENCE
        .split_once("2. Change and test in one call")
        .and_then(|(_, rest)| rest.split_once("\n3. Fix only what the reply reports"))
        .map(|(step, _)| step)
        .expect("workflow step 2");
    for part in ["command: submit --project <folder>", "test --project <folder>", "text:    *** Begin Patch"] {
        assert!(step.contains(part), "workflow step 2: missing {part}");
    }
    let text = mcp::tool_schema()["inputSchema"]["properties"]["text"]["description"].as_str().unwrap().to_string();
    assert!(text.contains("`submit --project <folder>` then `test --project <folder>`"), "{text}");
}
