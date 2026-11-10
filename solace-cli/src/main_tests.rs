use super::testing::*;
use super::*;

use serde_json::json;
use crate::cli::{hinted, is_process, reset_after_panic, run_inner, run_options, with_root};
use std::env;
use crate::tasks::store::put;
use crate::board::task_model::task;
use crate::board::reviews::changes_of;
use crate::board::server::api;
use crate::board::task_model::open;
use crate::board::runs::with_context;
use crate::board::setup::with_config;

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
    assert!(REFERENCE.contains("project's folder exactly as the task's scope lists it"));
    assert!(agent_reference(true).contains("first add it in the task's Scope tab"));
    assert!(!agent_reference(false).contains("Scope tab"));
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
    assert_eq!(reference_layout("Workflow only\n"), (true, String::new()));
    assert_eq!(
        reference_layout("Commands in full\n  grep <text>\n"),
        (false, "no command row has its description on the same line\n".into())
    );
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
    // D-158 point 6 and D-207: the reference names both languages and their limits.
    assert!(r.contains("Files by tier: a Gleam project's src/ and test/ *.gleam, and a Rust project's"), "both languages: {r}");
    assert!(REFERENCE.contains("Same-project Rust (D-185, D-205)") && REFERENCE.contains("Same-project Rust (D-207)"), "{REFERENCE}");
    assert!(
        r.contains("Changing a signature or adding a field")
            && r.contains("one at the level the task reports")
            && r.contains("`help <command>`")
    );
    assert!(!r.contains("probe --from <fn>") && REFERENCE.contains("probe --from <fn>") && REFERENCE.contains("--replace [--all]"));
    assert!(r.contains("In Gleam, a needed import is") && r.contains("submit --imports <module>") && !r.contains("task ask"));
    // Scope, the code's own conventions, comments that say why, and no secrets.
    assert!(r.contains("stop exploring") && r.contains("Write code the way the code around it is written"));
    assert!(r.contains("A comment says why code exists") && r.contains("Never put secrets or keys"));
    // Craft rules: the smallest complete change, and what `test` can't reach.
    assert!(r.contains("Make the smallest complete change") && r.contains("Fix a reported bug once, at its cause"));
    assert!(r.contains("`test` doesn't reach docs, config, JavaScript or golden files"));
    // D-119 point 5: plain-file projects batch too, in their own patch.
    for part in [
        "code changes",
        "(symbols and modules) go in one `submit` patch, then `test`",
        "other files go in one `write` patch (no path), then `run <name>`",
        "a change to both is two calls",
        "a patch of many edits",
    ] {
        assert!(r.contains(part), "workflow step 2: missing {part}");
    }
    assert!(!r.contains("*** Update File: <path>"), "code and file headers in one patch: {r}");
    // D-123: where a line runs, and that options belong to one command.
    assert!(
        r.contains("Each line runs in one project")
            && r.contains("`--lines` is `read`'s and `inspect`'s")
            && r.contains("`help <command>`\nlists them")
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
    let e = agent_help(Some("extract"));
    assert!(e.contains("Moving\n") && e.contains("Moved crate.Store::find"), "{e}");
    let s = agent_help(Some("submit"));
    assert!(!s.contains("Moving\n"), "{s}");
    let g = agent_help(Some("grep"));
    assert!(g.contains("over the watch limit"), "{g}");
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
    let e = hinted(a.view(&["plan.answer"], false, crate::cli::BUDGET), &a.root).unwrap_err().0;
    assert!(e.contains("E1001 unknown name: plan.answer\nfound in b: add --project b"), "{e}");
    let e = hinted(a.view(&["plan.nope"], false, crate::cli::BUDGET), &a.root).unwrap_err().0;
    assert!(e.contains("E1001") && !e.contains("found in"), "{e}");
    let patch = concat!(
        "*** Begin Patch\n*** Update: plan.answer\n@@\n",
        "-pub fn answer() -> Int {\n-  42\n-}\n+pub fn answer() -> Int {\n+  43\n+}\n*** End Patch\n",
    );
    let e = hinted(Err(crate::drafts::patch_blocks(&a, patch).unwrap_err()), &a.root).unwrap_err().0;
    assert!(e.contains("E1001 unknown name: plan.answer\nfound in b: add --project b"), "{e}");
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
    let dir = crate::board::store_dir(&root);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t1.json"), r#"{"id":"t1","column":"backlog","home":"a","scope":["a"],"runs":[]}"#).unwrap();
    let s = a.other_projects(Some("t1"));
    assert!(s.contains("This task can change: a (alpha)."), "{s}");
    assert!(s.contains("Read only: b (beta)."), "{s}");
    assert_eq!(s.matches("add it").count(), 1, "{s}");
    assert!(!s.contains("ask for it"), "{s}");
    assert!(s.contains("Then make the change yourself"), "{s}");
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
    let one = store.inspect(&["dep/api.make", "app.main"], true, None, usize::MAX).ok().unwrap();
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

    let out = ok(store.inspect(&names, true, None, usize::MAX));
    assert_eq!(out.matches("-- pilot/shipments.Shipment\n").count(), 1);
    assert!(out.contains("-- pilot/shipments.Shipment (above)"));
    assert!(!out.contains("Trimmed") && !out.contains("Not shown"));

    let small = ok(store.inspect(&names, true, None, 3_000));
    assert!(small.contains("Trimmed to fit the size limit"));
    assert!(small.contains("Not shown, to fit the size limit: `solace inspect "));
    assert!(small.len() < out.len());
    for n in names {
        assert!(small.contains(&format!("// ── pilot/{n} ")) || small.lines().last().unwrap().contains(&format!("pilot/{n}")), "{n} lost");
    }

    // an unknown name is reported after the others, not instead of them
    let mixed = ok(store.inspect(&["shipments.deliver", "shipping.standard_cost_cents"], false, None, usize::MAX));
    assert!(mixed.contains("// ── pilot/shipments.deliver ") && mixed.contains("E1001 unknown name: shipping.standard_cost_cents"));
    assert!(store.inspect(&["shipping.nope"], false, None, usize::MAX).is_err());
    // D-46: a type's users split into the sites that list every field and
    // the rest by name; a function's callers show the call line.
    let ty = ok(store.inspect(&["customer.Customer"], true, None, usize::MAX));
    assert!(ty.contains("== Constructed by (") && ty.contains("-- pilot/seed.customers, line 3: Customer("), "{ty}");
    assert!(ty.contains("-- pilot/accounts.register, line 11: customer.Customer(\n"), "{ty}");
    assert!(ty.contains("== Other users (") && !ty.contains("== Used by"), "{ty}");
    // commands in one MCP call share what was printed: `refs` says (above)
    SHOWN.with(|s| *s.borrow_mut() = Some(HashSet::new()));
    let f = ok(store.inspect(&["pricing.quote"], true, None, usize::MAX));
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
    let out = store.probe("echo calc.add(2, 3)", None, &[], None).unwrap();
    assert!(out.contains("5"), "{out}");
    assert!(!probe_file.exists());
    let err = store.probe("echo calc.nope()", None, &[], None).unwrap_err().0;
    assert!(err.contains("The probe ran as") && err.contains("import calc"), "{err}");
    // D-145: an unknown module value says how to print a value; another failure doesn't.
    assert!(err.contains("Unknown module value") && err.contains("Hint: print a value with `echo value`.\n\nThe probe ran as"), "{err}");
    let err = store.probe("echo nope", None, &[], None).unwrap_err().0;
    assert!(err.contains("The probe ran as") && !err.contains("Hint:"), "{err}");
    assert!(!probe_file.exists());
    // A probe is test-side code: it may use test helpers.
    assert!(store.probe("echo helper.two()", None, &[], None).unwrap().contains("2"));
    // `--from` starts from a function's body; a single expression is printed (D-43).
    let out = store.probe("echo x + 10", Some("calc_test.sum_test"), &[], None).unwrap();
    assert!(out.contains("13"), "{out}");
    assert!(store.probe("", Some("calc.add"), &[], None).unwrap_err().0.starts_with("E1005"));
    let out = store.probe("calc.add(20, 3)", None, &[], None).unwrap();
    assert!(out.contains("23") && !out.contains("warning"), "{out}");
}

#[test]
fn probe_trace_shows_calls_and_puts_the_module_back() {
    let store = probe_project();
    let before = fs::read_to_string(store.root.join("src/calc.gleam")).unwrap();
    let out = store.probe("calc.add(2, 3)", None, &["calc.add"], None).unwrap();
    assert!(out.contains("→ calc.add(2, 3)") && out.contains("← calc.add = 5"), "{out}");
    assert_eq!(fs::read_to_string(store.root.join("src/calc.gleam")).unwrap(), before);
    let out = store.probe("calc.scale(3, by: 2)", None, &["calc.scale"], None).unwrap();
    assert!(out.contains("← calc.scale = 6"), "{out}");
    assert!(store.probe("1", None, &["calc_test"], None).is_err());
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

/// D-44: `test` runs the tests whose reach changed since they passed and those
/// that failed or never ran; the rest are skipped.
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
    // The failing test runs again; the one reaching foreign code does not.
    store.submit_symbols(Some("calc_test.sub_test"), None, "pub fn sub_test() {\n  assert calc.sub(3, 1) == 2\n}").unwrap();
    assert_eq!(
        first(store.test(&[])),
        "Ran 1 of 4 tests; the other 3 reach nothing changed since they passed (`test --all` runs every test)."
    );
    // A change reaches only the tests that can see it, and the files match the store afterwards.
    store.submit_symbols(Some("calc.add"), None, "pub fn add(a: Int, b: Int) -> Int {\n  b + a\n}").unwrap();
    let out = store.test(&[]).unwrap();
    assert!(out.starts_with("Ran 2 of 4 tests;") && out.contains("2 passed"), "{out}");
    let file = fs::read_to_string(store.root.join("test/calc_test.gleam")).unwrap();
    assert_eq!(file.matches("pub fn ").count(), 4, "{file}");
    assert_eq!(first(store.test(&["--all"])), "Ran all 4 tests.");
    assert_eq!(first(store.test(&["calc.double"])), "Ran 1 of 4 tests: those that reach calc.double.");
    assert!(store.test(&["calc.nope"]).unwrap_err().0.starts_with("E1001"));
    // With nothing changed, no test runs.
    assert!(first(store.test(&[])).starts_with("Ran 0 of 4 tests;"));
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
    let long = "x".repeat(crate::cli::WORK_CAP);
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
    let out = store.submit(crate::store::Target::Module("calc"), &module).unwrap();
    assert_eq!(out, "Note: calc: changed add; added none\nSaved calc.\nThe project type-checks.\n");
    assert!(fs::read_to_string(&file).unwrap().contains("b + a"));
    let without_three = module.split("pub fn three").next().unwrap();
    assert!(store.submit(crate::store::Target::Module("calc"), without_three).unwrap_err().0.contains("E0102 the module text leaves out: three"));
    assert!(store.submit(crate::store::Target::Module("calc"), &format!("{module}\npub fn one() {{ 1 }}")).unwrap_err().0.starts_with("E0103"));
    // A rejected module leaves the stored code, and its file, as they were.
    let before = fs::read_to_string(&file).unwrap();
    let err = store.submit(crate::store::Target::Module("calc"), &module.replace("b + a", "b +. a")).unwrap_err().0;
    assert!(err.starts_with("Not saved: the submitted text has errors."), "{err}");
    assert_eq!(fs::read_to_string(&file).unwrap(), before);
    store.submit_symbols(None, Some("other"), "pub fn x() { 1 }").unwrap();
    let out = store.submit(crate::store::Target::Imports("calc"), "//// Arithmetic.\nimport other").unwrap();
    assert!(out.starts_with("Saved calc imports."), "{out}");
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("//// Arithmetic.\n\nimport other\n"), "{text}");
    assert!(store.submit(crate::store::Target::Imports("calc"), "pub fn x() { 1 }").unwrap_err().0.starts_with("E0101"));
    assert!(store.submit(crate::store::Target::Imports("nope"), "").unwrap_err().0.starts_with("E1003"));
}

/// D-44: `impact` follows a symbol's data to what uses it.
#[test]
fn impact_names_what_a_symbol_reaches() {
    let store = calc_project();
    let out = store.impact(&["calc.add"], crate::cli::BUDGET).unwrap();
    assert!(out.starts_with("calc.add reaches 3 uses, nearest first:"), "{out}");
    assert!(out.contains("calc.one") && out.contains("calc.two") && out.contains("calc.three"), "{out}");
    assert!(store.impact(&[], crate::cli::BUDGET).unwrap().starts_with("Tests haven't passed in full yet"));
    assert!(store.impact(&["calc.nope"], crate::cli::BUDGET).unwrap_err().0.starts_with("E1001"));
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
        let e = lockdown::plain::<()>(&store, Some(t["id"].as_str().unwrap()), command, None).unwrap_err().0;
        assert!(e.contains("files, grep, read, write and run. This task can change: . (the root), r."), "{command}: {e}");
        assert!(e.contains("`--project r`"), "{command}: {e}");
        assert!(!e.contains("Gleam"), "{command}: {e}");
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
    let e = lockdown::plain::<()>(&store, Some(t["id"].as_str().unwrap()), "write", None).unwrap_err().0;
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
        "<module>_tests.rs",
        "submit --imports <module>",
        "submit <module>",
        "submit <container>",
        "submit --new <parent>::<new>",
        "submit --new store::notes",
        "integration::x",
        "*** Add: a::b::c",
        "*** Add: m.f",
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
        "probe --in <module>",
        "Gleam only",
        "Same-project Rust (D-207)",
        "functions, constants and statics",
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
        "Not available for Rust: `probe`",
        "Not available for Rust: `merge`.",
        "Rust probe is deferred",
        "Rust merge is deferred",
        "a test in an inline `mod tests`",
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

/// A strategy run's help shows a worker command it can't run, and row names
/// ignore the `|` inside placeholders and option lists.
#[test]
fn strategy_help_shows_worker_commands_and_clean_command_names() {
    let missing = agent_help_for(Some("nosuch"), false, true, false);
    assert!(missing.starts_with("No command `nosuch`"), "{missing}");
    for bad in ["milestone>", "all]", "--tail"] {
        assert!(!missing.contains(bad), "{bad}: {missing}");
    }
    let submit = agent_help_for(Some("submit"), false, true, false);
    assert!(submit.starts_with("Worker command: a strategy run can't run it."), "{submit}");
    assert!(submit.contains("submit <name> --replace [--all]") && submit.contains("draft"), "{submit}");
    assert!(!submit.contains("probe --from"), "{submit}");
    let feature = agent_help_for(Some("feature"), false, true, false);
    assert!(feature.contains("strategy task|feature|milestone"), "{feature}");
    let strategy = agent_help_for(Some("strategy"), false, true, false);
    assert!(strategy.contains("strategy show run <task> <run> --failed"), "{strategy}");
    let host = agent_help_for(Some("create"), false, true, false);
    assert!(host.starts_with("No command `create`"), "{host}");
}

#[test]
fn the_reference_and_worker_schema_show_the_submit_call_fields() {
    let step = reference::REFERENCE
        .split_once("2. Change and test in one call")
        .and_then(|(_, rest)| rest.split_once("\n3. Fix only what the reply reports"))
        .map(|(step, _)| step)
        .expect("workflow step 2");
    for part in ["submit --project <folder>", "test --project <folder>", "argument text:", "argument old:", "submit --draft <id> --replace"]
    {
        assert!(step.contains(part), "workflow step 2: missing {part}");
    }
    let text = mcp::tool_schema()["inputSchema"]["properties"]["text"]["description"].as_str().unwrap().to_string();
    assert!(text.contains("`submit --project <folder>` then `test --project <folder>`"), "{text}");
}

#[test]
fn dependency_modules_are_imported_by_their_last_path_segment() {
    let store = project(&[
        (
            "gleam.toml",
            concat!(
                "name = \"app\"\nversion = \"1.0.0\"\n\n[dependencies]\n",
                "sqlight = { path = \"deps/sqlight\" }\n",
                "gleam_json = { path = \"deps/gleam_json\" }\n",
            ),
        ),
        ("deps/sqlight/gleam.toml", "name = \"sqlight\"\nversion = \"1.0.0\"\n"),
        ("deps/sqlight/src/sqlight.gleam", "pub fn query() -> Int {\n  1\n}\n"),
        ("deps/gleam_json/gleam.toml", "name = \"gleam_json\"\nversion = \"1.0.0\"\n"),
        ("deps/gleam_json/src/gleam/json.gleam", "pub fn decode() -> Int {\n  2\n}\n"),
        ("src/app.gleam", "pub fn main() -> Int {\n  1\n}\n"),
    ]);
    let out = store.submit_symbols(None, Some("app"), "pub fn run_sql() -> Int { sqlight.query() }").unwrap();
    assert!(out.contains("added `import sqlight` to app"), "{out}");
    let out = store.submit_symbols(None, Some("app"), "pub fn run_json() -> Int { json.decode() }").unwrap();
    assert!(out.contains("added `import gleam/json` to app"), "{out}");
    let file = fs::read_to_string(store.root.join("src/app.gleam")).unwrap();
    assert!(file.contains("import gleam/json") && file.contains("import sqlight"), "{file}");
}

#[test]
fn a_dependency_module_is_not_guessed_when_two_match() {
    let store = project(&[
        (
            "gleam.toml",
            concat!(
                "name = \"app\"\nversion = \"1.0.0\"\n\n[dependencies]\n",
                "first = { path = \"deps/first\" }\n",
                "second = { path = \"deps/second\" }\n",
            ),
        ),
        ("deps/first/gleam.toml", "name = \"first\"\nversion = \"1.0.0\"\n"),
        ("deps/first/src/first/json.gleam", "pub fn decode() -> Int {\n  1\n}\n"),
        ("deps/second/gleam.toml", "name = \"second\"\nversion = \"1.0.0\"\n"),
        ("deps/second/src/second/json.gleam", "pub fn decode() -> Int {\n  2\n}\n"),
        ("src/app.gleam", "pub fn main() -> Int {\n  1\n}\n"),
    ]);
    let err = store.submit_symbols(None, Some("app"), "pub fn run() -> Int { json.decode() }").unwrap_err().0;
    assert!(err.contains("matches more than one module"), "{err}");
    assert!(err.contains("import first/json") && err.contains("import second/json") && err.contains("submit --imports app"), "{err}");
    let file = fs::read_to_string(store.root.join("src/app.gleam")).unwrap();
    assert!(!file.contains("import first/json") && !file.contains("import second/json"), "{file}");
}

#[test]
fn an_alias_already_used_in_the_project_is_reused() {
    let store = project(&[
        ("src/helper.gleam", "pub fn value() -> Int {\n  1\n}\n"),
        ("src/caller.gleam", "import helper as values\n\npub fn call() -> Int {\n  values.value()\n}\n"),
        ("src/app.gleam", "pub fn main() -> Int {\n  1\n}\n"),
    ]);
    let out = store.submit_symbols(None, Some("app"), "pub fn call() -> Int { values.value() }").unwrap();
    assert!(out.contains("added `import helper as values` to app"), "{out}");
    let file = fs::read_to_string(store.root.join("src/app.gleam")).unwrap();
    assert!(file.contains("import helper as values"), "{file}");
}

#[test]
fn an_alias_with_two_targets_is_not_guessed() {
    let store = project(&[
        ("src/one.gleam", "pub fn value() -> Int {\n  1\n}\n"),
        ("src/two.gleam", "pub fn value() -> Int {\n  2\n}\n"),
        ("src/first.gleam", "import one as shared\n\npub fn a() -> Int {\n  shared.value()\n}\n"),
        ("src/second.gleam", "import two as shared\n\npub fn b() -> Int {\n  shared.value()\n}\n"),
        ("src/app.gleam", "pub fn main() -> Int {\n  1\n}\n"),
    ]);
    let err = store.submit_symbols(None, Some("app"), "pub fn c() -> Int { shared.value() }").unwrap_err().0;
    assert!(err.contains("matches more than one module"), "{err}");
    assert!(err.contains("import one as shared") && err.contains("import two as shared") && err.contains("submit --imports app"), "{err}");
}

#[test]
fn a_dev_dependency_is_imported_only_by_test_code() {
    let store = project(&[
        (
            "gleam.toml",
            concat!("name = \"app\"\nversion = \"1.0.0\"\n\n[dev-dependencies]\n", "dev_tool = { path = \"deps/dev_tool\" }\n",),
        ),
        ("deps/dev_tool/gleam.toml", "name = \"dev_tool\"\nversion = \"1.0.0\"\n"),
        ("deps/dev_tool/src/dev_tool.gleam", "pub fn value() -> Int {\n  1\n}\n"),
        ("src/app.gleam", "pub fn main() -> Int {\n  1\n}\n"),
        ("test/app_test.gleam", "pub fn existing_test() {\n  assert 1 == 1\n}\n"),
    ]);
    let err = store.submit_symbols(None, Some("app"), "pub fn run() -> Int { dev_tool.value() }").unwrap_err().0;
    assert!(err.contains("only available to test code") && err.contains("dev_tool"), "{err}");
    assert!(!fs::read_to_string(store.root.join("src/app.gleam")).unwrap().contains("import dev_tool"));
    let out = store.submit_symbols(None, Some("app_test"), "pub fn dev_test() { assert dev_tool.value() == 1 }").unwrap();
    assert!(out.contains("added `import dev_tool` to app_test"), "{out}");
}

#[test]
fn an_unknown_module_names_the_imports_call() {
    let store = project(&[
        ("src/helper.gleam", "pub fn value() -> Int {\n  1\n}\n"),
        ("src/app.gleam", "import helper\n\npub fn main() -> Int {\n  1\n}\n"),
    ]);
    let err = store.submit_symbols(None, Some("app"), "pub fn run() -> Int { sqlight.query() }").unwrap_err().0;
    assert!(err.contains("Unknown module") && err.contains("submit --imports app"), "{err}");
    assert!(err.contains("import sqlight") && err.contains("import helper"), "{err}");
}

#[test]
fn a_dependency_module_does_not_shadow_the_stdlib() {
    let store = project(&[
        (
            "gleam.toml",
            concat!(
                "name = \"app\"\nversion = \"1.0.0\"\n\n[dependencies]\n",
                "other = { path = \"deps/other\" }\n",
                "gleam_stdlib = { path = \"deps/gleam_stdlib\" }\n",
            ),
        ),
        ("deps/other/gleam.toml", "name = \"other\"\nversion = \"1.0.0\"\n"),
        ("deps/other/src/other/list.gleam", "pub fn length() -> Int {\n  1\n}\n"),
        ("deps/gleam_stdlib/gleam.toml", "name = \"gleam_stdlib\"\nversion = \"1.0.0\"\n"),
        ("deps/gleam_stdlib/src/gleam/list.gleam", "pub fn length() -> Int {\n  2\n}\n"),
        ("src/app.gleam", "pub fn main() -> Int {\n  1\n}\n"),
    ]);
    let out = store.submit_symbols(None, Some("app"), "pub fn run() -> Int { list.length() }").unwrap();
    assert!(out.contains("added `import gleam/list` to app"), "{out}");
}

#[test]
fn a_project_test_module_is_imported_only_by_test_code() {
    let store =
        project(&[("src/app.gleam", "pub fn main() -> Int {\n  1\n}\n"), ("test/support.gleam", "pub fn helper() -> Int {\n  1\n}\n")]);
    let err = store.submit_symbols(None, Some("app"), "pub fn run() -> Int { support.helper() }").unwrap_err().0;
    assert!(err.contains("only available to test code") && err.contains("support"), "{err}");
    let out = store.submit_symbols(None, Some("app_test"), "pub fn support_test() { assert support.helper() == 1 }").unwrap();
    assert!(out.contains("added `import support` to app_test"), "{out}");
}

#[test]
fn an_unknown_module_value_gets_no_import_hint() {
    let store = project(&[
        ("src/helper.gleam", "pub fn value() -> Int {\n  1\n}\n"),
        ("src/app.gleam", "import helper\n\npub fn main() -> Int {\n  1\n}\n"),
    ]);
    let err = store.submit_symbols(None, Some("app"), "pub fn run() -> Int { helper.missing() }").unwrap_err().0;
    assert!(err.contains("Unknown module value") && !err.contains("submit --imports"), "{err}");
}

#[test]
fn an_unknown_type_in_an_imported_module_gets_no_import_hint() {
    let store = project(&[
        ("src/helper.gleam", "pub fn value() -> Int {\n  1\n}\n"),
        ("src/app.gleam", "import helper\n\npub fn main() -> Int {\n  1\n}\n"),
    ]);
    let err = store.submit_symbols(None, Some("app"), "pub fn f() -> helper.Foo { todo }").unwrap_err().0;
    assert!(err.contains("Unknown module type") && !err.contains("submit --imports"), "{err}");
}

#[test]
fn dependency_modules_are_read_once_until_the_manifest_changes() {
    let store = project(&[
        ("gleam.toml", concat!("name = \"app\"\nversion = \"1.0.0\"\n\n[dependencies]\n", "first = { path = \"deps/first\" }\n",)),
        ("deps/first/gleam.toml", "name = \"first\"\nversion = \"1.0.0\"\n"),
        ("deps/first/src/first.gleam", "pub fn value() -> Int {\n  1\n}\n"),
        ("deps/second/gleam.toml", "name = \"second\"\nversion = \"1.0.0\"\n"),
        ("deps/second/src/second.gleam", "pub fn value() -> Int {\n  2\n}\n"),
        ("src/app.gleam", "pub fn main() -> Int {\n  1\n}\n"),
    ]);
    let first = store.dependency_modules();
    let again = store.dependency_modules();
    assert!(std::rc::Rc::ptr_eq(&first, &again), "the unchanged manifest reuses the scan");
    assert_eq!(first.iter().map(|d| d.path.as_str()).collect::<Vec<_>>(), vec!["first"]);
    fs::write(store.root.join("deps/first/src/extra.gleam"), "pub fn extra() -> Int {\n  3\n}\n").unwrap();
    let with_extra = store.dependency_modules();
    assert!(!std::rc::Rc::ptr_eq(&first, &with_extra), "a changed path dependency's src skips the cache");
    assert!(with_extra.iter().any(|d| d.path == "extra"), "{with_extra:?}");
    fs::create_dir(store.root.join("deps/first/src/sub")).unwrap();
    let with_sub = store.dependency_modules();
    assert!(std::rc::Rc::ptr_eq(&with_extra, &with_sub), "an empty nested folder changes nothing");
    fs::write(store.root.join("deps/first/src/sub/extra.gleam"), "pub fn extra() -> Int {\n  4\n}\n").unwrap();
    let with_nested = store.dependency_modules();
    assert!(!std::rc::Rc::ptr_eq(&with_sub, &with_nested), "a module added under an existing nested folder skips the cache");
    assert!(with_nested.iter().any(|d| d.path == "sub/extra"), "{with_nested:?}");
    fs::write(
        store.root.join("gleam.toml"),
        concat!(
            "name = \"app\"\nversion = \"1.0.0\"\n\n[dependencies]\n",
            "first = { path = \"deps/first\" }\n",
            "second = { path = \"deps/second\" }\n",
        ),
    )
    .unwrap();
    let modified = std::time::SystemTime::now() + std::time::Duration::from_secs(1);
    fs::File::options().write(true).open(store.root.join("gleam.toml")).unwrap().set_modified(modified).unwrap();
    let changed = store.dependency_modules();
    assert!(!std::rc::Rc::ptr_eq(&first, &changed), "a changed manifest skips the cache");
    assert!(changed.iter().any(|d| d.path == "second"), "{changed:?}");
}

#[test]
fn a_rust_project_can_drop_a_draft() {
    let (root, s) = testing::rust_project("pub fn helper() -> i32 {\n    1\n}\n");
    s.db.execute(
        "INSERT INTO drafts(id,owner,args,text,base) VALUES ('d1',?,?,?,?)",
        params!["cli", json!(["submit"]).to_string(), "text", ""],
    )
    .unwrap();
    let out = with_root(&root, || {
        run_options(&["drafts".to_string(), "drop".to_string(), "d1".to_string()], &mut || Ok(String::new()), None, 10_000, &[])
    })
    .unwrap();
    assert!(out.contains("Dropped draft d1"), "{out}");
}

/// A Rust probe runs in a scratch copy: it sees the crate, shows its output,
/// and leaves the project and the test records alone.
#[test]
fn probe_runs_rust_code_and_leaves_nothing() {
    if !crate::language::cargo::installed() {
        return;
    }
    let lib = "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
    let (root, store) = testing::rust_project(lib);
    let out = store.probe("println!(\"{}\", crate::add(2, 3));", None, &[], None).unwrap();
    assert!(out.contains('5'), "{out}");
    assert!(probe_seconds(&out).is_some(), "{out}");
    assert_eq!(fs::read_to_string(root.join("src/lib.rs")).unwrap(), lib);
    assert!(root.join("target/solace-probe/target/debug").is_dir(), "cargo builds outside the crate copy");
    let runs: i64 = store.db.query_row("SELECT COUNT(*) FROM test_runs", [], |r| r.get(0)).unwrap();
    let base: i64 = store.db.query_row("SELECT COUNT(*) FROM test_base", [], |r| r.get(0)).unwrap();
    assert_eq!((runs, base), (0, 0));
    let err = store.probe("crate::missing();", None, &[], None).unwrap_err().0;
    assert!(err.contains("at probe, line 1"), "{err}");
    assert!(probe_seconds(&err).is_some(), "{err}");
    assert!(err.contains("The probe's text:\n1 | crate::missing();"), "{err}");
    assert!(!err.contains("mod solace_probe") && !err.contains("src/lib.rs:"), "{err}");
    let dbg = store.probe("dbg!(1);", None, &[], None).unwrap();
    assert!(dbg.contains("[probe, line 1] 1 = 1"), "{dbg}");
    assert!(probe_seconds(&dbg).is_some(), "{dbg}");
    assert!(!dbg.contains("src/lib.rs:"), "{dbg}");
    let panic = store.probe("panic!(\"boom\");", None, &[], None).unwrap_err().0;
    assert!(panic.contains("panicked at probe, line 1:"), "{panic}");
    assert!(probe_seconds(&panic).is_some(), "{panic}");
    assert!(!panic.contains("src/lib.rs:"), "{panic}");
    assert!(!panic.contains("failures:") && !panic.contains("    solace_probe::probe\n"), "{panic}");
    assert!(!panic.contains("error: test failed") && !panic.contains("RUST_BACKTRACE"), "{panic}");
    let note = store.probe("crate::add(2);", None, &[], None).unwrap_err().0;
    assert!(note.contains("at crate.add, line 1"), "{note}");
    assert!(probe_seconds(&note).is_some(), "{note}");
    let mismatch = store.probe("let x: i32 = \"not a number\";", None, &[], None).unwrap_err().0;
    assert!(mismatch.contains("1 | let x: i32 = \"not a number\";"), "{mismatch}");
    assert!(!mismatch.contains("|         let x"), "{mismatch}");
    assert!(!mismatch.contains("|         ^"), "{mismatch}");
    assert!(probe_seconds(&mismatch).is_some(), "{mismatch}");
    let unclosed = store.probe("let x = (;", None, &[], None).unwrap_err().0;
    assert!(probe_seconds(&unclosed).is_some(), "{unclosed}");
    assert!(!unclosed.contains("imports") && !unclosed.contains("src/lib.rs:"), "{unclosed}");
    assert_eq!(fs::read_to_string(root.join("src/lib.rs")).unwrap(), lib);
    let copy = root.join("target/solace-probe/crate");
    assert_eq!(fs::read_to_string(copy.join("src/lib.rs")).unwrap(), lib);
    assert!(copy.is_dir(), "the crate copy is reused");
}

/// `--in` places the probe inside a module, so it calls that module's private
/// items; a root probe that hits one names the flag.
#[test]
fn a_rust_probe_in_a_module_reaches_its_private_items() {
    if !crate::language::cargo::installed() {
        return;
    }
    let parser = "fn helper() -> i32 {\n    7\n}\n\npub fn two() -> i32 {\n    helper() + 1\n}\n";
    let root = testing::Scratch::new().with(&[
        ("Cargo.toml", testing::CARGO),
        ("src/lib.rs", "mod parser;\n\npub fn one() -> i32 {\n    1\n}\n"),
        ("src/parser.rs", parser),
    ]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let err = store.probe("println!(\"{}\", parser::helper());", None, &[], None).unwrap_err().0;
    assert!(err.contains("E0603") && err.contains("--in parser"), "{err}");
    let out = store.probe("println!(\"{}\", helper());", None, &[], Some("parser")).unwrap();
    assert!(out.contains('7'), "{out}");
    assert_eq!(fs::read_to_string(root.join("src/parser.rs")).unwrap(), parser);
}

/// Rust has no `--trace`, and `--from` decides where the probe runs.
#[test]
fn a_rust_probe_refuses_trace_and_from_with_in() {
    let (_root, store) = testing::rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let e = store.probe("one()", Some("crate.one"), &[], Some("crate")).unwrap_err().0;
    assert!(e.starts_with("E1006") && e.contains("--from") && e.contains("--in"), "{e}");
    let e = store.probe("one()", None, &["crate.one"], None).unwrap_err().0;
    assert!(e.starts_with("E1006") && e.contains("--trace") && e.contains("Gleam only"), "{e}");
}

/// The command route reaches the Rust probe instead of the plain-files refusal.
#[test]
fn a_rust_project_runs_a_probe_without_saving_it() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (root, _store) = testing::rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let out = with_root(&root, || {
        run_inner(&root, &root, &["probe".to_string()], &mut || Ok("println!(\"{}\", crate::one());".to_string()), None, 10_000)
    })
    .unwrap();
    assert!(out.contains("exit code: 0") && out.contains('1'), "{out}");
    assert_eq!(fs::read_to_string(root.join("src/lib.rs")).unwrap(), "pub fn one() -> i32 {\n    1\n}\n");
}

/// A `[lib] path` is a target setting, not a path dependency: the probe runs.
#[test]
fn a_rust_probe_accepts_a_lib_path_setting() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = testing::Scratch::new().with(&[
        ("Cargo.toml", &format!("{}[lib]\npath = \"src/lib.rs\"\n", testing::CARGO)),
        ("src/lib.rs", "pub fn one() -> i32 {\n    1\n}\n"),
    ]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.probe("println!(\"{}\", crate::one());", None, &[], None).unwrap();
    assert!(out.contains('1'), "{out}");
}

/// A scratch copy can't follow a path dependency, so the probe refuses it.
#[test]
fn a_rust_probe_refuses_a_path_dependency() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = testing::Scratch::new().with(&[
        ("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nx = { path = \"../x\" }\n"),
        ("src/lib.rs", "pub fn one() -> i32 {\n    1\n}\n"),
    ]);
    import_with(&root, false).unwrap();
    let store = Store::at(&root).unwrap();
    let err = store.probe("println!(\"{}\", crate::one());", None, &[], None).unwrap_err().0;
    assert!(err.contains("path dependency (`x`)"), "{err}");
}

/// Two probes share one crate copy, so the second reuses its binary and lock.
#[test]
fn a_second_rust_probe_reuses_the_crates_build_files() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (root, store) = testing::rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let artifacts = || {
        fs::read_dir(root.join("target/solace-probe/target/debug/deps"))
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with("app-") && !name.ends_with(".d"))
            .collect::<std::collections::BTreeSet<_>>()
    };
    let lock = root.join("target/solace-probe/crate/Cargo.lock");
    let first_reply = store.probe("println!(\"{}\", crate::one());", None, &[], None).unwrap();
    let first = artifacts();
    let first_lock = fs::read(&lock).unwrap();
    let second_reply = store.probe("println!(\"{}\", crate::one() + 1);", None, &[], None).unwrap();
    assert_eq!(artifacts(), first);
    assert_eq!(fs::read(&lock).unwrap(), first_lock);
    assert!(probe_seconds(&first_reply).is_some(), "{first_reply}");
    assert!(probe_seconds(&second_reply).is_some(), "{second_reply}");
}

/// A crate inside a workspace above it probes without joining that workspace.
#[test]
fn a_rust_probe_runs_in_a_workspace_member() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = testing::Scratch::new().with(&[
        ("Cargo.toml", "[workspace]\nmembers = [\"app\"]\n"),
        ("app/Cargo.toml", testing::CARGO),
        ("app/src/lib.rs", "pub fn one() -> i32 {\n    1\n}\n"),
    ]);
    let app = root.join("app");
    import(&app).unwrap();
    let store = Store::at(&app).unwrap();
    let out = store.probe("println!(\"{}\", crate::one());", None, &[], None).unwrap();
    assert!(out.contains('1'), "{out}");
}

/// A member that inherits a package key gets E1006, not a raw cargo error.
#[test]
fn a_rust_probe_in_a_member_that_inherits_a_key_says_e1006() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = testing::Scratch::new().with(&[
        ("Cargo.toml", "[workspace]\nmembers = [\"app\"]\n[workspace.package]\nversion = \"0.1.0\"\n"),
        ("app/Cargo.toml", "[package]\nname = \"app\"\nversion.workspace = true\nedition = \"2021\"\n"),
        ("app/src/lib.rs", "pub fn one() -> i32 {\n    1\n}\n"),
    ]);
    let app = root.join("app");
    import(&app).unwrap();
    let store = Store::at(&app).unwrap();
    let e = store.probe("println!(\"{}\", crate::one());", None, &[], None).unwrap_err().0;
    assert!(e.starts_with("E1006") && e.contains("version"), "{e}");
    assert!(!app.join("target/solace-probe/crate").exists());
}

/// A second probe keeps the workspace lock cargo trimmed in the copy.
#[test]
fn a_workspace_members_existing_lock_is_not_copied_again() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = testing::Scratch::new().with(&[
        ("Cargo.toml", "[workspace]\nmembers = [\"app\", \"other\"]\nresolver = \"2\"\n"),
        (
            "Cargo.lock",
            concat!(
                "# This file is automatically @generated by Cargo.\n",
                "# It is not intended for manual editing.\n",
                "version = 3\n\n",
                "[[package]]\nname = \"app\"\nversion = \"0.1.0\"\n\n",
                "[[package]]\nname = \"other\"\nversion = \"0.1.0\"\n",
            ),
        ),
        ("app/Cargo.toml", testing::CARGO),
        ("app/src/lib.rs", "pub fn one() -> i32 {\n    1\n}\n"),
        ("other/Cargo.toml", "[package]\nname = \"other\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("other/src/lib.rs", "pub fn two() -> i32 {\n    2\n}\n"),
    ]);
    let app = root.join("app");
    import(&app).unwrap();
    let store = Store::at(&app).unwrap();
    let lock = app.join("target/solace-probe/crate/Cargo.lock");
    store.probe("println!(\"{}\", crate::one());", None, &[], None).unwrap();
    assert!(!fs::read_to_string(&lock).unwrap().contains("name = \"other\""));
    let source = fs::metadata(root.join("Cargo.lock")).unwrap().modified().unwrap();
    let newer = source + std::time::Duration::from_secs(1);
    std::fs::OpenOptions::new().write(true).open(&lock).unwrap().set_modified(newer).unwrap();
    let bytes = fs::read(&lock).unwrap();
    let modified = fs::metadata(&lock).unwrap().modified().unwrap();
    store.probe("println!(\"{}\", crate::one() + 1);", None, &[], None).unwrap();
    assert_eq!(fs::read(&lock).unwrap(), bytes);
    assert_eq!(fs::metadata(&lock).unwrap().modified().unwrap(), modified);
}

/// A build script and `include_bytes!` see files outside the crate's files, and
/// the probe leaves the crate's own build artifacts alone.
#[test]
fn probe_reads_files_beside_the_crate_and_in_nested_projects() {
    if !crate::language::cargo::installed() {
        return;
    }
    let lib = concat!(
        "pub fn icon() -> &'static [u8] {\n",
        "    include_bytes!(\"assets/icon.bin\")\n",
        "}\n\n",
        "pub fn app() -> &'static str {\n",
        "    include_str!(concat!(env!(\"OUT_DIR\"), \"/app.txt\"))\n",
        "}\n\n",
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    #[test]\n",
        "    fn app_is_current() {\n",
        "        assert_eq!(crate::app(), \"second\\n\");\n",
        "    }\n",
        "}\n",
    );
    let (root, store) = testing::rust_project(lib);
    std::fs::create_dir_all(root.join("frontend/.solace")).unwrap();
    std::fs::write(root.join("frontend/.solace/store.db"), "nested").unwrap();
    std::fs::create_dir_all(root.join("frontend/priv")).unwrap();
    std::fs::write(root.join("frontend/priv/app.js"), "first\n").unwrap();
    std::fs::create_dir_all(root.join("src/assets")).unwrap();
    std::fs::write(root.join("src/assets/icon.bin"), [0, 1, 2, 3]).unwrap();
    let build = concat!(
        "fn main() {\n",
        "    let app = std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\"))\n",
        "        .join(\"frontend/priv/app.js\");\n",
        "    let text = std::fs::read_to_string(&app).unwrap();\n",
        "    let out = std::path::Path::new(&std::env::var(\"OUT_DIR\").unwrap()).join(\"app.txt\");\n",
        "    std::fs::write(out, text).unwrap();\n",
        "    println!(\"cargo:rerun-if-changed={}\", app.display());\n",
        "}\n",
    );
    std::fs::write(root.join("build.rs"), build).unwrap();
    let out = store.probe("dbg!(crate::app(), crate::icon().len());", None, &[], None).unwrap();
    assert!(out.contains("first") && out.contains("crate::icon().len() = 4"), "{out}");
    assert!(root.join("target/solace-probe/target/debug").is_dir(), "the probe keeps its own profile");
    assert_eq!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap(), lib);
    assert_eq!(std::fs::read_to_string(root.join("frontend/priv/app.js")).unwrap(), "first\n");
    assert_eq!(std::fs::read_to_string(root.join("frontend/.solace/store.db")).unwrap(), "nested");
    let runs: i64 = store.db.query_row("SELECT COUNT(*) FROM test_runs", [], |r| r.get(0)).unwrap();
    let base: i64 = store.db.query_row("SELECT COUNT(*) FROM test_base", [], |r| r.get(0)).unwrap();
    assert_eq!((runs, base), (0, 0));
    std::fs::write(root.join("frontend/priv/app.js"), "second\n").unwrap();
    let cargo = std::process::Command::new("cargo")
        .args(["test", "--offline", "--target-dir"])
        .arg(root.join("target"))
        .arg("--")
        .arg("app_is_current")
        .current_dir(&*root)
        .output()
        .unwrap();
    assert!(cargo.status.success(), "{}\n{}", String::from_utf8_lossy(&cargo.stdout), String::from_utf8_lossy(&cargo.stderr));
}

/// A real inline `solace_probe` clashes with the module the probe adds.
#[test]
fn a_rust_probe_refuses_an_inline_solace_probe() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (_root, store) = testing::rust_project("mod solace_probe {\n    pub fn marker() {}\n}\n");
    let e = store.probe("1 + 1", None, &[], None).unwrap_err().0;
    assert!(e.starts_with("E0101 a project module is named solace_probe"), "{e}");
}

/// A file-backed `solace_probe` is a real module in the index and is refused.
#[test]
fn a_rust_probe_refuses_a_file_backed_solace_probe() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = testing::Scratch::new().with(&[
        ("Cargo.toml", testing::CARGO),
        ("src/lib.rs", "mod solace_probe;\n\npub fn one() -> i32 {\n    1\n}\n"),
        ("src/solace_probe.rs", "pub fn marker() -> i32 {\n    2\n}\n"),
    ]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let e = store.probe("1 + 1", None, &[], None).unwrap_err().0;
    assert!(e.starts_with("E0101 a project module is named solace_probe"), "{e}");
}

/// `mod solace_probe` in a string or comment is text, not a module.
#[test]
fn a_rust_probe_runs_beside_a_solace_probe_mention_in_text() {
    if !crate::language::cargo::installed() {
        return;
    }
    let parser = concat!(
        "// mod solace_probe in a comment is not a module.\n",
        "/// A doc comment may say mod solace_probe.\n",
        "pub fn note() -> &'static str {\n",
        "    \"mod solace_probe\"\n",
        "}\n",
    );
    let root = testing::Scratch::new().with(&[("Cargo.toml", testing::CARGO), ("src/lib.rs", "mod parser;\n"), ("src/parser.rs", parser)]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.probe("1 + 1", None, &[], Some("parser")).unwrap();
    assert!(out.contains("1 + 1 = 2"), "{out}");
    assert_eq!(fs::read_to_string(root.join("src/parser.rs")).unwrap(), parser);
}

/// A probe reply's elapsed time in seconds, from its last line.
fn probe_seconds(reply: &str) -> Option<f64> {
    let (_, time) = reply.trim_end().rsplit_once('(')?;
    time.strip_suffix("s)")?.parse().ok()
}

#[test]
fn a_rust_destination_says_same_project_only() {
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
            assert!(e.contains("same-project only") && e.contains("D-207"), "merge: {e}");
        }
        assert!(!e.contains("no Gleam code"), "{command}: {e}");
    }
}

/// D-185, D-207: the command route reaches Rust `extract` and `merge`, not the
/// plain-files refusal.
#[test]
fn a_rust_project_extracts_and_merges_through_the_cli() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (extract_root, _) = testing::rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let extract = ["extract", "crate.one", "--to", "helpers"].map(String::from);
    let out = with_root(&extract_root, || run_options(&extract, &mut || Ok(String::new()), None, 10_000, &[])).unwrap();
    assert!(out.contains("Created module helpers (src/helpers.rs)"), "{out}");

    let (merge_root, _) =
        testing::rust_project("pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n\npub fn go() -> i32 {\n    two()\n}\n");
    let merge = ["merge", "crate.one", "crate.two"].map(String::from);
    let out = with_root(&merge_root, || run_options(&merge, &mut || Ok(String::new()), None, 10_000, &[])).unwrap();
    assert!(out.contains("Merged crate.two into crate.one"), "{out}");
    assert!(out.contains("Redirected 1 reference in 1 module"), "{out}");
}

/// A Rust project's `extract` and `merge` need cargo, like its other tools.
#[test]
fn a_rust_project_needs_cargo_to_extract_and_merge() {
    let (root, _) = testing::rust_project("pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n");
    let run = |args: &[&str]| {
        let args: Vec<String> = args.iter().copied().map(String::from).collect();
        with_root(&root, || crate::language::cargo::with_path(&[], || run_options(&args, &mut || Ok(String::new()), None, 10_000, &[])))
    };
    for args in [&["extract", "crate.one", "--to", "helpers"][..], &["merge", "crate.one", "crate.two"][..]] {
        let err = run(args).unwrap_err().0;
        assert!(err.contains("E6013") && err.contains(crate::language::cargo::MISSING), "{args:?}: {err}");
    }
}

#[test]
fn a_language_reason_refusal_covers_extract_and_merge() {
    let (root, _) = testing::rust_project("pub fn one() -> i32 {\n    1\n}\n\npub fn two() -> i32 {\n    2\n}\n");
    let off = testing::Scratch::new().with(&[("system.toml", "[languages]\nrust = false\n")]);
    let reason = crate::language::off(crate::language::RUST);
    for args in [&["extract", "crate.one", "--to", "helpers"][..], &["merge", "crate.one", "crate.two"][..]] {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let err = with_root(&root, || {
            with_config(&off.join("system.toml"), || run_options(&args, &mut || Ok(String::new()), None, 10_000, &[]))
        })
        .unwrap_err();
        assert!(err.0.contains("E6013") && err.0.contains(reason), "{args:?}: {}", err.0);
    }
}

#[test]
fn a_rust_extract_to_project_in_a_task_leaves_no_open_change() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"web\", \"r\"]\n"),
        (".solace/store.db", ""),
        ("web/gleam.toml", "name = \"web\"\nversion = \"1.0.0\"\n"),
        ("web/src/app.gleam", "pub fn main() {\n  1\n}\n"),
        ("r/Cargo.toml", testing::CARGO),
        ("r/src/lib.rs", "pub fn helper() -> i32 {\n    1\n}\n"),
    ]);
    let web = root.join("web");
    let rust = root.join("r");
    import(&web).unwrap();
    import(&rust).unwrap();
    let s = open(&root).unwrap();
    let mut t = api(&root, "POST", "/tasks", json!({"title": "Extract"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["scope"] = json!(["r", "web"]);
    t["runs"] = json!([{"id": "run", "kind": "work", "state": "running"}]);
    put(&s, &t).unwrap();
    let args = ["extract", "crate.helper", "--to", "helpers", "--to-project", "web"].map(String::from);
    let e = with_root(&rust, || {
        with_context(&tid, "run", || run_options(&args, &mut || Ok(String::new()), None, 10_000, &[]))
    })
    .unwrap_err();
    assert!(e.0.contains("E6013") && e.0.contains("not available for Rust") && e.0.contains("D-185"), "{}", e.0);
    let t = task(&root, &tid).unwrap();
    assert!(changes_of(&s, &t).unwrap().is_empty(), "no change recorded");
    for project in [&rust, &web] {
        assert!(!crate::extract::ahead::waiting(project), "{} left an ahead record", project.display());
        let store = open(project).unwrap();
        store
            .db
            .execute_batch("BEGIN IMMEDIATE")
            .and_then(|_| store.db.execute_batch("ROLLBACK"))
            .unwrap_or_else(|e| panic!("{} left an open transaction: {e}", project.display()));
    }
}

#[test]
fn reference_hash() {
    assert_eq!(sha256(REFERENCE.as_bytes()), "25084f455b58d1963e2a72aa0c78efd003291a41543c931470d5b46555a6d5c2");
}

#[test]
fn reference_section_layout_is_clean() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/reference");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "txt"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no section files");
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/reference.rs");
    let source = fs::read_to_string(source_path).unwrap();
    let (_, reference) = source.split_once("const REFERENCE: &str = concat!(").expect("REFERENCE is not a concat");
    let (reference, _) = reference.split_once(");").expect("REFERENCE has no end");
    let mut listed: Vec<_> = reference
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .filter_map(|line| line.trim().strip_prefix("include_str!(\"reference/")?.strip_suffix("\"),"))
        .collect();
    listed.sort_unstable();
    let mut present: Vec<_> = files.iter().map(|file| file.file_name().unwrap().to_str().unwrap()).collect();
    present.sort_unstable();
    assert_eq!(listed, present, "section files and REFERENCE differ");
    for file in &files {
        let section = fs::read_to_string(file).unwrap();
        let (clean, out) = reference_layout(&section);
        assert!(clean, "{}: {out}", file.display());
    }
    let misaligned = fs::read_to_string(dir.join("project.txt")).unwrap().replacen("\n  build ", "\n   build ", 1);
    assert!(!reference_layout(&misaligned).0, "a misaligned section file passes");
}

#[test]
fn reference_layout_checks_a_section_file_without_the_heading() {
    let section = |spaces: usize| format!("Reading\n  read <path>    a file\n{}or more\n", " ".repeat(spaces));
    assert_eq!(reference_layout(&section(17)), (true, String::new()));
    assert_eq!(reference_layout(&section(18)), (false, "line 3: starts at column 19; the descriptions start at column 18\n".into()));
}

#[test]
fn reference_layout_checks_a_section_file_whose_rows_wrap() {
    let section = |second: usize| {
        format!("Host commands\n  create <name>\n{}create a project\n  serve [--port N]\n{}serve it\n", " ".repeat(30), " ".repeat(second))
    };
    assert_eq!(reference_layout(&section(30)), (true, String::new()));
    assert_eq!(reference_layout(&section(31)), (false, "line 5: starts at column 32; the descriptions start at column 31\n".into()));
}

/// `--from` runs an inline test's body in the test's own module, so its
/// imports, helpers and locals are in scope; nothing is saved.
#[test]
fn a_rust_probe_from_an_inline_test_runs_its_body() {
    if !crate::language::cargo::installed() {
        return;
    }
    let lib = concat!(
        "mod tests {\n",
        "    use std::fs;\n\n",
        "    fn helper() -> u64 {\n",
        "        3\n",
        "    }\n\n",
        "    #[test]\n",
        "    fn probe_values() {\n",
        "        let value = helper() + 4;\n",
        "        assert!(fs::metadata(file!()).unwrap().len() > 0);\n",
        "    }\n",
        "}\n",
    );
    let (root, store) = testing::rust_project(lib);
    let out = store.probe("dbg!(value);", Some("crate.tests::probe_values"), &[], None).unwrap();
    assert!(out.contains("[probe, line 1] value = 7"), "{out}");
    assert!(probe_seconds(&out).is_some(), "{out}");
    assert_eq!(fs::read_to_string(root.join("src/lib.rs")).unwrap(), lib);
    let runs: i64 = store.db.query_row("SELECT COUNT(*) FROM test_runs", [], |r| r.get(0)).unwrap();
    let base: i64 = store.db.query_row("SELECT COUNT(*) FROM test_base", [], |r| r.get(0)).unwrap();
    assert_eq!((runs, base), (0, 0));
}

/// A file-backed test module appends the probe to that module's file.
#[test]
fn a_rust_probe_from_a_file_backed_test_uses_its_module() {
    if !crate::language::cargo::installed() {
        return;
    }
    let lib = "#[path = \"lib_tests.rs\"]\nmod tests;\n\npub fn one() -> i32 {\n    1\n}\n";
    let tests =
        "fn helper() -> i32 {\n    5\n}\n\n#[test]\nfn probe_values() {\n    let value = helper() + 1;\n    assert_eq!(value, 6);\n}\n";
    let root = testing::Scratch::new().with(&[("Cargo.toml", testing::CARGO), ("src/lib.rs", lib), ("src/lib_tests.rs", tests)]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let out = store.probe("dbg!(value);", Some("tests.probe_values"), &[], None).unwrap();
    assert!(out.contains("value = 6"), "{out}");
    assert_eq!(fs::read_to_string(root.join("src/lib.rs")).unwrap(), lib);
    assert_eq!(fs::read_to_string(root.join("src/lib_tests.rs")).unwrap(), tests);
}

/// An integration test is its own crate, so `--from` refuses it like `--in`.
#[test]
fn a_rust_probe_refuses_from_an_integration_test() {
    if !crate::language::cargo::installed() {
        return;
    }
    let root = testing::Scratch::new().with(&[
        ("Cargo.toml", testing::CARGO),
        ("src/lib.rs", "pub fn one() -> i32 {\n    1\n}\n"),
        ("tests/acceptance.rs", "#[test]\nfn probe_values() {\n    let value = 1;\n}\n"),
    ]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    let e = store.probe("dbg!(value);", Some("integration::acceptance.probe_values"), &[], None).unwrap_err().0;
    assert!(e.starts_with("E1006") && e.contains("test crate") && e.contains("--from"), "{e}");
}

/// A generic header hides the body from `--from`; the probe says so instead
/// of running without the test's locals.
#[test]
fn a_rust_probe_from_a_generic_function_says_it_cant_read_the_body() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (_root, store) = testing::rust_project("fn helper<T>() -> i32 {\n    1\n}\n");
    let e = store.probe("dbg!(1);", Some("crate.helper"), &[], None).unwrap_err().0;
    assert_eq!(e, "E1006 can't read the body of crate.helper; --from takes a non-generic function without arguments.");
}

/// `--from` places the probe in the test's own nested inline module, not a
/// same-named `tests` module at the top level.
#[test]
fn a_rust_probe_from_a_nested_inline_test_uses_its_own_module() {
    if !crate::language::cargo::installed() {
        return;
    }
    let lib = concat!(
        "mod tests {\n",
        "    fn helper() -> i32 {\n",
        "        100\n",
        "    }\n\n",
        "    #[test]\n",
        "    fn top() {\n",
        "        assert_eq!(helper(), 100);\n",
        "    }\n",
        "}\n\n",
        "mod inner {\n",
        "    mod tests {\n",
        "        fn helper() -> i32 {\n",
        "            7\n",
        "        }\n\n",
        "        #[test]\n",
        "        fn probe_values() {\n",
        "            let value = helper() + 1;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let (_root, store) = testing::rust_project(lib);
    let out = store.probe("dbg!(value);", Some("crate.inner::tests::probe_values"), &[], None).unwrap();
    assert!(out.contains("[probe, line 1] value = 8"), "{out}");
}

/// A panic in the `--from` test's own body means the probe's statements never
/// ran; the error says so instead of leaving only the panic.
#[test]
fn a_rust_probe_from_a_failing_test_says_the_probe_text_did_not_run() {
    if !crate::language::cargo::installed() {
        return;
    }
    let lib = "#[test]\nfn fails_before_probe() {\n    assert_eq!(1, 2);\n}\n";
    let (_root, store) = testing::rust_project(lib);
    let e = store.probe("dbg!(1);", Some("crate.fails_before_probe"), &[], None).unwrap_err().0;
    assert!(e.contains("crate.fails_before_probe panicked before the probe's statements ran"), "{e}");
}

/// A generic function with arguments is refused for its arguments, before its body.
#[test]
fn a_rust_probe_from_a_generic_function_with_arguments_says_it_takes_arguments() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (_root, store) = testing::rust_project("fn helper<T>(x: T) -> T {\n    x\n}\n");
    let e = store.probe("dbg!(1);", Some("crate.helper"), &[], None).unwrap_err().0;
    assert_eq!(e, "E1005 crate.helper takes arguments; --from takes a test or another function without arguments.");
}

/// `components`' `solace_task.Column`, and a `frontend` that constructs it.
fn column_workspace() -> Scratch {
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"components\", \"frontend\"]\n"),
        ("components/gleam.toml", "name = \"components\"\nversion = \"1.0.0\"\n"),
        (
            "components/src/solace_task.gleam",
            "pub type Column {\n  Column(name: String)\n}\n\npub fn columns() -> List(Column) {\n  [Column(\"a\")]\n}\n",
        ),
        ("frontend/gleam.toml", "name = \"frontend\"\nversion = \"1.0.0\"\n\n[dependencies]\ncomponents = { path = \"../components\" }\n"),
        (
            "frontend/src/knowledgebase.gleam",
            "import solace_task\n\npub fn known_columns() -> List(solace_task.Column) {\n  [solace_task.Column(\"a\")]\n}\n",
        ),
    ]);
    for d in ["components", "frontend"] {
        import(&root.join(d)).unwrap();
    }
    root
}

fn column_task(root: &Scratch, scope: &[&str]) -> (std::path::PathBuf, String) {
    task_at(root, "components", scope)
}

/// `Column` gains a field, and `components` updates its own constructor.
fn column_patch() -> String {
    concat!(
        "// ── solace_task.Column ──\n",
        "pub type Column {\n  Column(name: String, role: String)\n}\n",
        "// ── solace_task.columns ──\n",
        "pub fn columns() -> List(Column) {\n  [Column(\"a\", \"b\")]\n}\n",
    )
    .to_string()
}

/// A save in a shared project refuses when a project outside its task breaks,
/// and names that project and its first error.
#[test]
fn an_uncovered_dependent_refuses_a_save() {
    let root = column_workspace();
    let (components, tid) = column_task(&root, &["components"]);
    let text = column_patch();
    let err = with_root(&components, || {
        with_context(&tid, "run", || run_options(&["submit".to_string()], &mut || Ok(text.clone()), None, 100_000, &[]))
    })
    .unwrap_err()
    .0;
    assert!(err.contains("E5003") && err.contains("frontend") && err.contains("knowledgebase"), "{err}");
    let written = std::fs::read_to_string(components.join("src/solace_task.gleam")).unwrap();
    assert!(!written.contains("role"), "{written}");
}

/// A save whose task covers the broken project is kept, named in the reply, and
/// `task done` waits until the dependent is fixed.
#[test]
fn a_covered_dependent_saves_and_waits_for_done() {
    let root = column_workspace();
    let (components, tid) = column_task(&root, &["components", "frontend"]);
    let text = column_patch();
    let out = with_root(&components, || {
        with_context(&tid, "run", || run_options(&["submit".to_string()], &mut || Ok(text.clone()), None, 100_000, &[]))
    })
    .unwrap();
    assert!(out.contains("Saved") && out.contains("frontend") && out.contains("knowledgebase"), "{out}");
    Store::at(&root.join("frontend")).unwrap().check().unwrap_err();
    let done = ["task".to_string(), "done".to_string(), "Done.".to_string()];
    let err = with_root(&components, || with_context(&tid, "run", || run_options(&done, &mut || Ok(String::new()), None, 10_000, &[])))
        .unwrap_err()
        .0;
    assert!(err.contains("task done is refused") && err.contains("frontend") && err.contains("knowledgebase"), "{err}");
    let fixed = fix_frontend_columns(&components, &tid);
    assert!(fixed.contains("Saved knowledgebase.known_columns"), "{fixed}");
    let out =
        with_root(&components, || with_context(&tid, "run", || run_options(&done, &mut || Ok(String::new()), None, 10_000, &[]))).unwrap();
    assert!(out.contains("Recorded"), "{out}");
}

/// A change that leaves its dependents clean saves as before.
#[test]
fn a_clean_dependent_still_saves() {
    let root = column_workspace();
    let (components, tid) = column_task(&root, &["components"]);
    let text = "// ── solace_task.columns ──\npub fn columns() -> List(Column) {\n  [Column(\"b\")]\n}\n";
    let out = with_root(&components, || {
        with_context(&tid, "run", || run_options(&["submit".to_string()], &mut || Ok(text.to_string()), None, 100_000, &[]))
    })
    .unwrap();
    assert!(out.contains("Saved solace_task.columns") && !out.contains("frontend"), "{out}");
}

/// `refs` lists uses in the workspace projects that depend on this one.
#[test]
fn refs_name_dependent_projects() {
    let root = column_workspace();
    let s = Store::at(&root.join("components")).unwrap();
    let out = s.refs("solace_task.Column").unwrap();
    assert!(out.contains("In frontend") && out.contains("knowledgebase.known_columns"), "{out}");
}

fn column_workspace_with(extra: &[(&str, &str)]) -> Scratch {
    let mut files = vec![
        ("solace.toml", "projects = [\"components\", \"frontend\"]\n"),
        ("components/gleam.toml", "name = \"components\"\nversion = \"1.0.0\"\n"),
        (
            "components/src/solace_task.gleam",
            "pub type Column {\n  Column(name: String)\n}\n\npub fn columns() -> List(Column) {\n  [Column(\"a\")]\n}\n",
        ),
        ("frontend/gleam.toml", "name = \"frontend\"\nversion = \"1.0.0\"\n\n[dependencies]\ncomponents = { path = \"../components\" }\n"),
        (
            "frontend/src/knowledgebase.gleam",
            "import solace_task\n\npub fn known_columns() -> List(solace_task.Column) {\n  [solace_task.Column(\"a\")]\n}\n",
        ),
    ];
    files.extend_from_slice(extra);
    let root = Scratch::new().with(&files);
    for d in ["components", "frontend"] {
        import(&root.join(d)).unwrap();
    }
    root
}

fn column_workspace_with_a_broken_frontend() -> Scratch {
    column_workspace_with(&[("frontend/src/legacy.gleam", "pub fn legacy() -> Int {\n  \"old\"\n}\n")])
}

fn fix_frontend_columns(components: &std::path::Path, tid: &str) -> String {
    let text = "import solace_task\n\npub fn known_columns() -> List(solace_task.Column) {\n  [solace_task.Column(\"a\", \"b\")]\n}\n";
    let args = ["submit", "knowledgebase.known_columns", "--project", "frontend"].map(String::from);
    with_root(components, || with_context(tid, "run", || run_options(&args, &mut || Ok(text.to_string()), None, 100_000, &[]))).unwrap()
}

#[test]
fn a_covered_dependent_keeps_an_earlier_error() {
    let root = column_workspace_with_a_broken_frontend();
    let (components, tid) = column_task(&root, &["components", "frontend"]);
    Store::at(&root.join("frontend")).unwrap().check().unwrap_err();
    let text = column_patch();
    let out = with_root(&components, || {
        with_context(&tid, "run", || run_options(&["submit".to_string()], &mut || Ok(text.clone()), None, 100_000, &[]))
    })
    .unwrap();
    assert!(out.contains("Saved") && out.contains("frontend") && out.contains("knowledgebase"), "{out}");
    let done = ["task".to_string(), "done".to_string(), "Done.".to_string()];
    let err = with_root(&components, || with_context(&tid, "run", || run_options(&done, &mut || Ok(String::new()), None, 10_000, &[])))
        .unwrap_err()
        .0;
    assert!(err.contains("frontend") && err.contains("knowledgebase"), "{err}");
    let fixed = fix_frontend_columns(&components, &tid);
    assert!(fixed.contains("Saved knowledgebase.known_columns"), "{fixed}");
    let out =
        with_root(&components, || with_context(&tid, "run", || run_options(&done, &mut || Ok(String::new()), None, 10_000, &[]))).unwrap();
    assert!(out.contains("Recorded"), "{out}");
    let frontend = Store::at(&root.join("frontend")).unwrap();
    assert!(frontend.check().unwrap_err().0.contains("legacy"), "the earlier error remains");
}

#[test]
fn a_dependent_that_cannot_be_checked_names_itself() {
    let root = column_workspace();
    std::fs::remove_dir_all(root.join("frontend/.solace")).unwrap();
    let (components, tid) = column_task(&root, &["components"]);
    let text = column_patch();
    let err = with_root(&components, || {
        with_context(&tid, "run", || run_options(&["submit".to_string()], &mut || Ok(text.clone()), None, 100_000, &[]))
    })
    .unwrap_err()
    .0;
    assert!(err.contains("frontend:"), "{err}");
    let written = std::fs::read_to_string(components.join("src/solace_task.gleam")).unwrap();
    assert!(!written.contains("role"), "{written}");
}

/// An earlier error in the dependent's same module stays old when the fix moves
/// its line, so finishing is not refused for it.
#[test]
fn a_covered_dependent_keeps_a_moved_earlier_error() {
    let root = column_workspace_with(&[(
        "frontend/src/knowledgebase.gleam",
        concat!(
            "import solace_task\n\n",
            "pub fn known_columns() -> List(solace_task.Column) {\n",
            "  [solace_task.Column(\"a\")]\n}\n\n",
            "pub fn legacy() -> Int {\n  \"old\"\n}\n",
        ),
    )]);
    let (components, tid) = column_task(&root, &["components", "frontend"]);
    Store::at(&root.join("frontend")).unwrap().check().unwrap_err();
    let text = column_patch();
    let out = with_root(&components, || {
        with_context(&tid, "run", || run_options(&["submit".to_string()], &mut || Ok(text.clone()), None, 100_000, &[]))
    })
    .unwrap();
    assert!(out.contains("Saved") && out.contains("knowledgebase"), "{out}");
    let fix = concat!(
        "import solace_task\n\n",
        "pub fn known_columns() -> List(solace_task.Column) {\n",
        "  let role = \"b\"\n",
        "  [solace_task.Column(\"a\", role)]\n}\n",
    );
    let args = ["submit", "knowledgebase.known_columns", "--project", "frontend"].map(String::from);
    let fixed =
        with_root(&components, || with_context(&tid, "run", || run_options(&args, &mut || Ok(fix.to_string()), None, 100_000, &[])))
            .unwrap();
    assert!(fixed.contains("Saved knowledgebase.known_columns"), "{fixed}");
    let done = ["task".to_string(), "done".to_string(), "Done.".to_string()];
    let out =
        with_root(&components, || with_context(&tid, "run", || run_options(&done, &mut || Ok(String::new()), None, 10_000, &[]))).unwrap();
    assert!(out.contains("Recorded"), "{out}");
    let frontend = Store::at(&root.join("frontend")).unwrap();
    assert!(frontend.check().unwrap_err().0.contains("legacy"), "the earlier error remains");
}

#[test]
fn test_runs_a_dependent_projects_tests_that_reach_a_change() {
    if !cfg!(unix) {
        return;
    }
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"components\", \"board\", \"frontend\"]\n"),
        ("components/gleam.toml", "name = \"components\"\nversion = \"1.0.0\"\n"),
        (
            "components/src/solace_task.gleam",
            concat!(
                "pub type Column {\n  Column(name: String)\n}\n\n",
                "pub fn columns() -> List(Column) {\n  [Column(\"a\")]\n}\n\n",
                "pub fn unrelated() -> Int {\n  1\n}\n",
            ),
        ),
        ("board/gleam.toml", "name = \"board\"\nversion = \"1.0.0\"\n\n[dependencies]\ncomponents = { path = \"../components\" }\n"),
        (
            "board/src/board.gleam",
            concat!(
                "import solace_task\n\n",
                "pub fn count() -> Int {\n",
                "  case solace_task.columns() {\n",
                "    [] -> 0\n",
                "    [_, ..] -> 1\n",
                "  }\n",
                "}\n",
            ),
        ),
        (
            "frontend/gleam.toml",
            concat!(
                "name = \"frontend\"\nversion = \"1.0.0\"\n\n[dependencies]\n",
                "board = { path = \"../board\" }\ncomponents = { path = \"../components\" }\n",
            ),
        ),
        ("frontend/src/view.gleam", "import board\n\npub fn count() -> Int {\n  board.count()\n}\n"),
        (
            "frontend/test/frontend_test.gleam",
            concat!(
                "import board\n\n",
                "pub fn counts_columns_test() {\n  assert board.count() > 0\n}\n\n",
                "pub fn arithmetic_test() {\n  assert 1 + 1 == 2\n}\n",
            ),
        ),
    ]);
    fake_gleam(&root);
    for d in ["components", "board", "frontend"] {
        import(&root.join(d)).unwrap();
    }
    let components = Store::at(&root.join("components")).unwrap();
    let frontend = Store::at(&root.join("frontend")).unwrap();
    crate::language::with_tool_path(&[root.join("bin")], || {
        assert!(frontend.test(&["--all"]).unwrap().starts_with("Ran all 2 tests."));
        components.submit_symbols(Some("solace_task.columns"), None, "pub fn columns() -> List(Column) {\n  [Column(\"b\")]\n}").unwrap();
        let out = components.test(&[]).unwrap();
        assert!(out.contains("== In frontend ==") && out.contains("Ran 1 of 2 tests") && out.contains("counts_columns_test"), "{out}");
        assert!(!out.contains("arithmetic_test"), "{out}");
        components.submit_symbols(Some("solace_task.unrelated"), None, "pub fn unrelated() -> Int {\n  2\n}").unwrap();
        let out = components.test(&[]).unwrap();
        assert!(out.contains("== In frontend ==") && out.contains("Ran 0 of 2 tests"), "{out}");
        assert!(!out.contains("counts_columns_test"), "{out}");
    });
}

#[test]
fn a_failing_dependent_test_fails_the_test_reply_and_names_its_project() {
    if !cfg!(unix) {
        return;
    }
    let root = column_workspace_with(&[(
        "frontend/test/frontend_test.gleam",
        concat!("import solace_task\n\n", "pub fn columns_test() {\n  let _ = solace_task.columns()\n  assert [1] == []\n}\n",),
    )]);
    fake_gleam(&root);
    fs::write(root.join("frontend/.fail_columns"), "").unwrap();
    let components = Store::at(&root.join("components")).unwrap();
    let frontend = Store::at(&root.join("frontend")).unwrap();
    crate::language::with_tool_path(&[root.join("bin")], || {
        let err = frontend.test(&["--all"]).unwrap_err().0;
        assert!(err.contains("columns_test"), "{err}");
        let err = components.test(&[]).unwrap_err().0;
        assert!(err.contains("== In frontend ==") && err.contains("columns_test"), "{err}");
    });
}

fn fake_gleam(root: &Scratch) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = root.join("bin/gleam");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"#!/bin/sh
if [ "$1" != "test" ]; then
  exit 0
fi
file=$(find test -name '*_test.gleam' | head -n 1)
if [ -z "$file" ]; then
  exit 0
fi
module=$(basename "$file" .gleam)
names=$(grep -E '^pub fn [a-z_0-9]+_test' "$file" | sed 's/^pub fn //' | sed 's/(.*//')
passed=0
failed=0
for name in $names; do
  if [ "$name" = "columns_test" ] && [ -f .fail_columns ]; then
    echo "assert $file:1"
    echo " test: $module.$name"
    echo " info: Assertion failed."
    echo
    failed=$((failed+1))
  else
    echo "test $name ... ok"
    passed=$((passed+1))
  fi
done
if [ "$failed" -eq 0 ]; then
  echo "$passed passed, 0 failures"
else
  echo "$passed passed, $failed failure"
  exit 1
fi
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
    }
}

/// A running work task at `home`, covering `scope`.
fn task_at(root: &Scratch, home: &str, scope: &[&str]) -> (std::path::PathBuf, String) {
    let home = root.join(home);
    let s = open(&home).unwrap();
    let mut t = api(&home, "POST", "/tasks", json!({"title":"Task"})).unwrap();
    let tid = t["id"].as_str().unwrap().to_string();
    t["scope"] = json!(scope);
    t["runs"] = json!([{"id":"run","kind":"work","state":"running"}]);
    put(&s, &t).unwrap();
    (home, tid)
}

/// A dependent's run must not replace the changed project's "No test reaches"
/// warning or add a second next step to the reply.
#[test]
fn a_dependent_keeps_the_primary_projects_untested_warning() {
    if !cfg!(unix) {
        return;
    }
    let root = column_workspace_with(&[
        ("components/src/misc.gleam", "pub fn unrelated() -> Int {\n  1\n}\n"),
        ("frontend/test/frontend_test.gleam", "import solace_task\n\npub fn columns_test() {\n  let _ = solace_task.columns()\n}\n"),
    ]);
    fake_gleam(&root);
    let (components, tid) = column_task(&root, &["components", "frontend"]);
    let store = Store::at(&components).unwrap();
    crate::language::with_tool_path(&[root.join("bin")], || {
        with_root(&components, || {
            with_context(&tid, "run", || {
                let text = "pub fn unrelated() -> Int {\n  2\n}\n";
                let args = ["submit".to_string(), "misc.unrelated".to_string()];
                run_options(&args, &mut || Ok(text.to_string()), None, 100_000, &[]).unwrap();
                let out = store.test(&[]).unwrap();
                assert!(out.contains("No test reaches misc.unrelated."), "{out}");
                assert_eq!(out.matches("Next: ").count(), 1, "{out}");
                assert_eq!(crate::UNTESTED.with(|u| u.borrow().clone()).as_deref(), Some("misc.unrelated"));
            })
        })
    });
}

/// A board run already holds its home project; testing a project it depends on
/// must not wait on that lock again.
#[test]
fn a_dependent_locked_by_the_session_is_not_locked_again() {
    if !cfg!(unix) {
        return;
    }
    let root = column_workspace_with(&[(
        "frontend/test/frontend_test.gleam",
        "import solace_task\n\npub fn columns_test() {\n  let _ = solace_task.columns()\n}\n",
    )]);
    fake_gleam(&root);
    let (frontend, tid) = task_at(&root, "frontend", &["components", "frontend"]);
    let components = Store::at(&root.join("components")).unwrap();
    let _held = crate::board::lock(&frontend).unwrap();
    crate::language::with_tool_path(&[root.join("bin")], || {
        let out = with_context(&tid, "run", || components.test(&[])).unwrap();
        assert!(out.contains("== In frontend =="), "{out}");
        assert!(out.contains("columns_test"), "{out}");
    });
}

/// A save that fails after its dependent check leaves no baseline on the task.
#[test]
fn a_rolled_back_save_keeps_no_dependent_baseline() {
    let root = column_workspace_with_a_broken_frontend();
    let (components, tid) = column_task(&root, &["components", "frontend"]);
    std::fs::write(components.join("solace.toml"), "not = [valid").unwrap();
    let text = column_patch();
    let err = with_context(&tid, "run", || Store::at(&components).unwrap().submit_symbols_blocks(None, None, &text, false)).unwrap_err().0;
    assert!(err.contains("E6020"), "{err}");
    let t = task(&components, &tid).unwrap();
    assert!(t.get("dependent_errors").is_none(), "{t}");
}

/// A new error on a dependent symbol already broken another way still blocks
/// `task done`, even when an earlier save recorded that symbol as old.
#[test]
fn a_new_error_on_an_already_broken_dependent_symbol_blocks_done() {
    let root = column_workspace_with(&[(
        "frontend/src/legacy.gleam",
        "import solace_task\n\npub fn legacy() -> Int {\n  let _ = solace_task.columns()\n  \"old\"\n}\n",
    )]);
    let (components, tid) = column_task(&root, &["components", "frontend"]);
    Store::at(&root.join("frontend")).unwrap().check().unwrap_err();
    let text = column_patch();
    let out = with_root(&components, || {
        with_context(&tid, "run", || run_options(&["submit".to_string()], &mut || Ok(text.clone()), None, 100_000, &[]))
    })
    .unwrap();
    assert!(out.contains("Saved") && out.contains("frontend") && out.contains("knowledgebase"), "{out}");
    let fixed = fix_frontend_columns(&components, &tid);
    assert!(fixed.contains("Saved knowledgebase.known_columns"), "{fixed}");
    let takes_a_name =
        concat!("// ── solace_task.columns ──\n", "pub fn columns(extra: Int) -> List(Column) {\n", "  [Column(\"a\", \"b\")]\n", "}\n",);
    let out = with_root(&components, || {
        with_context(&tid, "run", || run_options(&["submit".to_string()], &mut || Ok(takes_a_name.to_string()), None, 100_000, &[]))
    })
    .unwrap();
    assert!(out.contains("Saved") && out.contains("legacy") && out.contains("Incorrect arity"), "{out}");
    let done = ["task".to_string(), "done".to_string(), "Done.".to_string()];
    let err = with_root(&components, || with_context(&tid, "run", || run_options(&done, &mut || Ok(String::new()), None, 10_000, &[])))
        .unwrap_err()
        .0;
    assert!(err.contains("task done is refused") && err.contains("legacy"), "{err}");
}

#[test]
fn a_guarded_local_name_is_not_imported() {
    let store = project(&[
        (
            "src/solace_task.gleam",
            "pub type Task {\n  Task(inner: Inner)\n  Unknown\n}\n\npub type Inner {\n  Inner(id: Int, total: Int)\n}\n",
        ),
        ("src/caller.gleam", "import solace_task as task\n\npub fn call() -> Int {\n  task.Inner(1, 2).total\n}\n"),
        ("src/app.gleam", "pub fn main() -> Int {\n  1\n}\n"),
    ]);
    let out = store
        .submit_symbols(
            None,
            Some("solace_task"),
            "pub fn value(x: Task) -> Int {\n  case x {\n    Task(task) if task.id != 0 -> task.total\n    _ -> 0\n  }\n}",
        )
        .unwrap();
    assert!(!out.contains("import solace_task as task"), "{out}");
    let file = fs::read_to_string(store.root.join("src/solace_task.gleam")).unwrap();
    assert!(!file.contains("import solace_task"), "{file}");
}

#[test]
fn a_gleam_probe_compile_error_is_still_reported() {
    let store = probe_project();
    let err = store.probe_with_limit("pub fn main() {\n  nope()\n}\n", None, &[], None, 2).unwrap_err().0;
    assert!(err.contains("The probe ran as test/solace_probe.gleam") && err.contains("nope"), "{err}");
    assert!(!store.root.join("test/solace_probe.gleam").exists());
}

#[test]
fn a_rust_probe_stops_at_its_run_limit_and_keeps_its_output() {
    if !crate::language::cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = testing::rust_project(lib);
    let code = "println!(\"started\"); loop { std::thread::sleep(std::time::Duration::from_millis(20)); }";
    let err = store.probe_with_limit(code, None, &[], None, 3).unwrap_err().0;
    assert!(err.contains("ran out of run time") && err.contains("3s") && err.contains("started"), "{err}");
    assert_eq!(fs::read_to_string(root.join("src/lib.rs")).unwrap(), lib);
}

#[test]
fn a_rust_probe_compile_error_is_still_reported() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (_root, store) = testing::rust_project("pub fn one() -> i32 {\n    1\n}\n");
    let err = store.probe_with_limit("let x: i32 = \"no\";", None, &[], None, 2).unwrap_err().0;
    assert!(err.contains("Compile error:") && err.contains("The probe's text:"), "{err}");
}

#[test]
fn a_rust_probe_stops_the_programs_it_detached() {
    if !crate::language::cargo::installed() || !crate::migrate::sandbox_works() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = testing::rust_project(lib);
    let code = concat!(
        "let beat = std::env::current_dir().unwrap().join(\"beat\");\n",
        "let script = format!(\"while :; do echo x >> '{}'; sleep 0.2; done\", beat.display());\n",
        "std::process::Command::new(\"setsid\").args([\"sh\", \"-c\", script.as_str()]).spawn().unwrap();\n",
        "println!(\"started\");\n",
        "loop { std::thread::sleep(std::time::Duration::from_millis(20)); }\n",
    );
    let err = store.probe_with_limit(code, None, &[], None, 3).unwrap_err().0;
    assert!(err.contains("ran out of run time") && err.contains("started"), "{err}");
    let beat = root.join("target/solace-probe/crate/beat");
    let first = fs::read(&beat).unwrap().len();
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(fs::read(&beat).unwrap().len(), first);
}

#[test]
fn a_gleam_probe_stops_the_programs_it_detached() {
    if !crate::migrate::sandbox_works() {
        return;
    }
    let store = probe_project();
    let code = concat!(
        "@external(erlang, \"erlang\", \"binary_to_list\")\n",
        "fn binary_to_list(binary: String) -> List(Int)\n\n",
        "@external(erlang, \"os\", \"cmd\")\n",
        "fn os_cmd(command: List(Int)) -> List(Int)\n\n",
        "pub fn main() {\n",
        "  echo \"started\"\n",
        "  let _ = os_cmd(binary_to_list(\"setsid sh -c 'while :; do echo x >> beat; sleep 0.2; done' >/dev/null 2>&1 &\"))\n",
        "  spin()\n",
        "}\n\n",
        "fn spin() {\n  spin()\n}\n",
    );
    let err = store.probe_with_limit(code, None, &[], None, 3).unwrap_err().0;
    assert!(err.contains("ran out of run time") && err.contains("started"), "{err}");
    let beat = store.root.join("beat");
    let first = fs::read(&beat).unwrap().len();
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(fs::read(&beat).unwrap().len(), first);
}

#[test]
fn a_rust_probe_stops_endless_output() {
    if !crate::language::cargo::installed() {
        return;
    }
    let lib = "pub fn one() -> i32 {\n    1\n}\n";
    let (root, store) = testing::rust_project(lib);
    let code = "println!(\"started\");\nloop { println!(\"x\"); }";
    let err = store.probe_with_limit(code, None, &[], None, 30).unwrap_err().0;
    assert!(
        err.contains("out of output room") && err.contains("started") && err.len() < 400_000,
        "{} bytes: {}",
        err.len(),
        &err[..err.len().min(200)]
    );
    let logs: Vec<String> = fs::read_dir(root.join(".solace"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("command-") && n.ends_with(".log"))
        .collect();
    assert!(logs.is_empty(), "{logs:?}");
}

#[test]
fn a_gleam_probe_stops_endless_output() {
    let store = probe_project();
    let code = "pub fn main() {\n  echo \"started\"\n  spin()\n}\n\nfn spin() {\n  echo \"x\"\n  spin()\n}\n";
    let err = store.probe_with_limit(code, None, &[], None, 30).unwrap_err().0;
    assert!(
        err.contains("out of output room") && err.contains("started") && err.len() < 400_000,
        "{} bytes: {}",
        err.len(),
        &err[..err.len().min(200)]
    );
    let logs: Vec<String> = fs::read_dir(store.root.join(".solace"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("command-") && n.ends_with(".log"))
        .collect();
    assert!(logs.is_empty(), "{logs:?}");
}

#[test]
fn a_probe_sandbox_protects_the_original() {
    if !crate::migrate::sandbox_works() {
        return;
    }
    let root = Scratch::new();
    let protect = root.join("original");
    fs::create_dir_all(&protect).unwrap();
    let cmd = crate::migrate::probe_sandboxed(&["true".into()], &root, Some(&protect));
    let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    assert!(args.iter().any(|a| a == "--unshare-pid"), "{args:?}");
    assert!(args.windows(2).any(|w| w[0] == "--ro-bind" && w[1] == protect.to_string_lossy()), "{args:?}");
}

#[test]
fn a_long_module_list_still_names_symbols() {
    let docs: Vec<String> = (0..120).map(|i| format!("//// module {i} {}\npub fn f{i}() -> Int {{\n  {i}\n}}\n", "x".repeat(60))).collect();
    let paths: Vec<String> = (0..120).map(|i| format!("src/m{i}.gleam")).collect();
    let mut files: Vec<(&str, &str)> = paths.iter().zip(&docs).map(|(path, doc)| (path.as_str(), doc.as_str())).collect();
    files.push(("src/app.gleam", "/// Totals.\npub fn total() -> Int {\n  1\n}\n"));
    let store = project(&files);
    let out = store.session_task("total").unwrap();
    let start = out.find("Modules (`view <module>` for an outline):\n").unwrap();
    let modules = &out[start..];
    let end = modules.find("\nTest modules: ").unwrap();
    assert!(modules[..end].len() > 6500, "module list is {} bytes", modules[..end].len());
    assert!(out.contains("-- app.total"), "{out}");
}

#[test]
fn a_gleam_probe_from_a_test_reaches_its_modules_private_items() {
    let store = project(&[
        ("src/order.gleam", "pub fn nothing() {\n  0\n}\n"),
        (
            "test/order_test.gleam",
            concat!(
                "const fee = 2\n\n",
                "type Box {\n  Box(Int)\n}\n\n",
                "fn add_fee(n: Int) -> Int {\n  n + fee\n}\n\n",
                "fn twice(n: Int) -> Int {\n  add_fee(add_fee(n))\n}\n\n",
                "pub fn total_test() {\n",
                "  let boxed = Box(3)\n",
                "  let Box(n) = boxed\n",
                "  let total = twice(n)\n",
                "  assert total == 7\n",
                "}\n",
            ),
        ),
    ]);
    let out = store.probe("echo total", Some("order_test.total_test"), &[], None).unwrap();
    assert!(out.contains("7"), "{out}");
    assert!(!store.root.join("test/solace_probe.gleam").exists());
}

#[test]
fn a_gleam_probe_from_a_test_traces_its_private_helper() {
    let store = project(&[
        ("src/order.gleam", "pub fn nothing() {\n  0\n}\n"),
        (
            "test/order_test.gleam",
            concat!(
                "const fee = 2\n\n",
                "fn add_fee(n: Int) -> Int {\n  n + fee\n}\n\n",
                "pub fn total_test() {\n",
                "  let total = add_fee(3)\n",
                "  assert total == 5\n",
                "}\n",
            ),
        ),
    ]);
    let out = store.probe("echo total", Some("order_test.total_test"), &["order_test.add_fee"], None).unwrap();
    assert!(out.contains("→ order_test.add_fee("), "{out}");
    assert!(!out.contains("no traced function was called"), "{out}");
    assert!(!store.root.join("test/solace_probe.gleam").exists());
}

#[test]
fn a_gleam_probe_from_refuses_to_trace_its_starting_function() {
    let store = project(&[
        ("src/order.gleam", "pub fn nothing() {\n  0\n}\n"),
        ("test/order_test.gleam", "pub fn total_test() {\n  assert 1 == 1\n}\n"),
    ]);
    let e = store.probe("echo 1", Some("order_test.total_test"), &["order_test.total_test"], None).unwrap_err().0;
    assert_eq!(e, "E1005 order_test.total_test runs inline in the probe and can't be traced; trace a helper it calls instead.");
}

#[test]
fn a_gleam_probe_from_refuses_a_test_that_reaches_main() {
    let store = project(&[
        ("src/order.gleam", "pub fn nothing() {\n  0\n}\n"),
        ("test/order_test.gleam", "pub fn main() {\n  Nil\n}\n\npub fn total_test() {\n  main()\n}\n"),
    ]);
    let e = store.probe("echo 1", Some("order_test.total_test"), &[], None).unwrap_err().0;
    assert_eq!(e, "E1005 order_test.total_test can't use --from: it reaches order_test.main, which clashes with the probe's own main.");
}

#[test]
fn a_gleam_probe_from_a_recursive_function_runs() {
    let store = project(&[
        ("src/order.gleam", "pub fn nothing() {\n  0\n}\n"),
        ("test/order_test.gleam", "pub fn total() {\n  let n = 0\n  case n {\n    0 -> Nil\n    _ -> total()\n  }\n}\n"),
    ]);
    let out = store.probe("", Some("order_test.total"), &[], None).unwrap();
    assert!(!out.contains("not in scope"), "{out}");
}

#[test]
fn a_gleam_probe_from_refuses_a_main_that_is_called_back() {
    let store = project(&[
        ("src/order.gleam", "pub fn nothing() {\n  0\n}\n"),
        ("test/order_test.gleam", "pub fn main() {\n  helper()\n}\n\nfn helper() {\n  main()\n}\n"),
    ]);
    let e = store.probe("echo 1", Some("order_test.main"), &[], None).unwrap_err().0;
    assert_eq!(e, "E1005 order_test.main is a main that is called back, so it can't be a starting point.");
}

#[test]
fn a_gleam_probe_from_refuses_a_directly_recursive_main() {
    let store =
        project(&[("src/order.gleam", "pub fn nothing() {\n  0\n}\n"), ("test/order_test.gleam", "pub fn main() {\n  main()\n}\n")]);
    let e = store.probe("echo 1", Some("order_test.main"), &[], None).unwrap_err().0;
    assert_eq!(e, "E1005 order_test.main is a main that is called back, so it can't be a starting point.");
}

/// The first read follows the best-ranked symbol even when another of the
/// task's projects holds it.
#[test]
fn a_task_session_first_call_reads_the_best_matching_project() {
    let root = Scratch::new().with(&[
        ("solace.toml", "projects = [\"a\", \"b\"]\n"),
        ("a/gleam.toml", "name = \"alpha\"\nversion = \"1.0.0\"\n"),
        ("a/src/app.gleam", "//// A widget helper.\npub fn helper() -> Int {\n  1\n}\n"),
        ("b/gleam.toml", "name = \"beta\"\nversion = \"1.0.0\"\n"),
        ("b/src/widget.gleam", "pub fn widget() -> Int {\n  2\n}\n"),
    ]);
    for d in ["a", "b"] {
        import(&root.join(d)).unwrap();
    }
    let a = Store::at(&root.join("a")).unwrap();
    board::open(&a.root).unwrap();
    let t = board::api(&a.root, "POST", "/tasks", json!({"title": "Widget"})).unwrap();
    let tid = t["id"].as_str().unwrap();
    tasks::update(&a.root, tid, |t| {
        t["scope"] = json!(["a", "b"]);
        Ok(())
    })
    .unwrap();
    let out = a.task_session("widget", Some(tid), "").unwrap();
    let next = out.lines().find(|l| l.starts_with("Next:")).expect(&out);
    assert!(next.contains("`inspect widget.widget --project b --context`"), "{next}");
    assert!(!next.contains("app.helper"), "{next}");
}

#[test]
fn a_gleam_probe_stops_at_its_run_limit() {
    let store = probe_project();
    let code = "pub fn main() {\n  echo \"started\"\n  spin()\n}\n\nfn spin() {\n  spin()\n}\n";
    let err = store.probe_with_limit(code, None, &[], None, 2).unwrap_err().0;
    assert!(err.contains("ran out of run time") && err.contains("2s"), "{err}");
    assert!(!store.root.join("test/solace_probe.gleam").exists());
}

#[test]
fn design_help_shows_only_design_commands() {
    let full = agent_help_for(None, false, false, true);
    assert!(full.contains("Design runs") && full.contains("design note") && full.contains("strategy list"), "{full}");
    for word in ["task ask", "submit", "`test`", "work add", "strategy note", "strategy task", "strategy vision"] {
        assert!(!full.contains(word), "{word}: {full}");
    }
    let submit = agent_help_for(Some("submit"), false, false, true);
    assert!(submit.starts_with("A design run can't run it."), "{submit}");
    let note = agent_help_for(Some("design"), false, false, true);
    assert!(note.contains("design note") && !note.contains("strategy note"), "{note}");
}

/// A submit reaches its task when a scope update briefly replaces the task file.
#[test]
fn a_submit_waits_out_a_task_file_rewrite() {
    let root = column_workspace();
    let (_home, tid) = column_task(&root, &["components", "frontend"]);
    let path = crate::board::store_dir(&root).join(format!("{tid}.json"));
    let saved = std::fs::read_to_string(&path).unwrap();
    let (gone, removed) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        std::fs::remove_file(&path).unwrap();
        gone.send(()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        std::fs::write(path, saved).unwrap();
    });
    removed.recv().unwrap();
    let frontend = root.join("frontend");
    let patch = concat!(
        "*** Begin Patch\n",
        "*** Update: knowledgebase.known_columns\n",
        "@@\n",
        "-  [solace_task.Column(\"a\")]\n",
        "+  [solace_task.Column(\"b\")]\n",
        "*** End Patch"
    );
    let out = with_root(&frontend, || {
        with_context(&tid, "run", || run_options(&["submit".to_string()], &mut || Ok(patch.to_string()), None, 100_000, &[]))
    });
    writer.join().unwrap();
    let out = out.unwrap();
    assert!(out.contains("Saved knowledgebase.known_columns"), "{out}");
    let store = Store::at(&frontend).unwrap();
    let shown = store.view(&["knowledgebase.known_columns"], false, 10_000).unwrap();
    assert!(shown.contains("Column(\"b\")"), "{shown}");
}

/// A test run says which other unfinished tasks' changes it included, on a
/// pass and a failure, and only when a test that ran can reach them (D-110).
#[test]
fn a_test_run_names_the_unfinished_changes_it_included() {
    if !cfg!(unix) {
        return;
    }
    let root = Scratch::new().with(&[
        ("src/calc.gleam", "pub fn columns() -> Int {\n  1\n}\n\npub fn sub(a: Int, b: Int) -> Int {\n  a - b\n}\n"),
        (
            "test/calc_test.gleam",
            concat!(
                "import calc\n\n",
                "pub fn columns_test() {\n  assert calc.columns() == 1\n}\n\n",
                "pub fn sub_test() {\n  assert calc.sub(3, 1) == 2\n}\n",
            ),
        ),
    ]);
    fake_gleam(&root);
    let store = root.import();
    let dir = crate::board::task_model::store_dir(&crate::scope::top(&store.root));
    fs::create_dir_all(&dir).unwrap();
    let log = store.root.join(".solace/run.log");
    fs::write(&log, "").unwrap();
    let put = |id: &str, title: &str, column: &str, running: bool| {
        let runs =
            if running { json!([{"id":"run","kind":"work","state":"running","log":log.to_string_lossy().to_string()}]) } else { json!([]) };
        let t = json!({
            "id":id,"title":title,"body":"","home":".","column":column,"rank":1.,
            "scope":["."],"dependsOn":[],"runs":runs,"notes":[],"questions":[],"links":[],
            "closedTime":null,"archivedTime":null,
        });
        fs::write(dir.join(format!("{id}.json")), t.to_string()).unwrap();
    };
    put("t", "Running task", "assigned", true);
    put("u", "Reached change", "assigned", false);
    put("v", "Unreached change", "assigned", false);
    put("w", "Finished change", "complete", false);
    put("x", "Canceled change", "cancelled", false);
    let ix = store.index().unwrap();
    let id = |name: &str| ix.syms.iter().find(|s| s.name == name).unwrap().id.clone();
    let touch = |task: &str, entity: String| {
        store
            .db
            .execute("INSERT INTO board_touches(task,entity,run,name) VALUES (?1,?2,?3,?4)", rusqlite::params![task, entity, "run", entity])
            .unwrap();
    };
    touch("t", format!("symbol:{}", id("columns")));
    touch("u", format!("symbol:{}", id("columns")));
    touch("v", format!("symbol:{}", id("sub")));
    touch("w", format!("symbol:{}", id("columns")));
    touch("x", format!("symbol:{}", id("columns")));
    let line = "included unfinished changes from task \"Reached change\" in .";
    crate::language::with_tool_path(&[store.root.join("bin")], || {
        let out = with_root(&store.root, || with_context("t", "run", || store.test(&["calc.columns"]))).unwrap();
        assert!(out.contains(line), "{out}");
        for absent in ["Unreached change", "Finished change", "Canceled change", "Running task"] {
            assert!(!out.contains(absent), "{absent} in {out}");
        }
        assert_eq!(crate::board::runs::transcript(&log).last().unwrap(), &json!({"kind":"usage","text":line}));
        fs::write(store.root.join(".fail_columns"), "").unwrap();
        let err = with_root(&store.root, || with_context("t", "run", || store.test(&["calc.columns"]).unwrap_err().0));
        assert!(err.contains(line), "{err}");
        assert_eq!(crate::board::runs::transcript(&log).last().unwrap(), &json!({"kind":"usage","text":line}));
    });
}

/// A test that runs in a dependent project names that project's unfinished
/// changes too, and records the line on the running task's log.
#[test]
fn a_dependent_test_run_names_the_unfinished_changes_it_included() {
    if !cfg!(unix) {
        return;
    }
    let root = column_workspace_with(&[(
        "frontend/test/frontend_test.gleam",
        "import knowledgebase\n\npub fn known_test() {\n  let _ = knowledgebase.known_columns()\n}\n",
    )]);
    fake_gleam(&root);
    let components = Store::at(&root.join("components")).unwrap();
    let frontend = Store::at(&root.join("frontend")).unwrap();
    let dir = crate::board::task_model::store_dir(&crate::scope::top(&root));
    fs::create_dir_all(&dir).unwrap();
    let log = root.join(".solace/run.log");
    fs::write(&log, "").unwrap();
    let put = |id: &str, title: &str, home: &str, running: bool| {
        let runs =
            if running { json!([{"id":"run","kind":"work","state":"running","log":log.to_string_lossy().to_string()}]) } else { json!([]) };
        let t = json!({
            "id":id,"title":title,"body":"","home":home,"column":"assigned","rank":1.,
            "scope":[home],"dependsOn":[],"runs":runs,"notes":[],"questions":[],"links":[],
            "closedTime":null,"archivedTime":null,
        });
        fs::write(dir.join(format!("{id}.json")), t.to_string()).unwrap();
    };
    put("t", "Running task", "components", true);
    put("u", "Frontend change", "frontend", false);
    put("v", "Components change", "components", false);
    let ix = frontend.index().unwrap();
    let id = ix.syms.iter().find(|s| s.name == "known_columns").unwrap().id.clone();
    frontend
        .db
        .execute(
            "INSERT INTO board_touches(task,entity,run,name) VALUES (?1,?2,?3,?4)",
            rusqlite::params!["u", format!("symbol:{id}"), "run", format!("symbol:{id}")],
        )
        .unwrap();
    let ix = components.index().unwrap();
    let column = ix.syms.iter().find(|s| s.name == "Column").unwrap().id.clone();
    components
        .db
        .execute(
            "INSERT INTO board_touches(task,entity,run,name) VALUES (?1,?2,?3,?4)",
            rusqlite::params!["v", format!("symbol:{column}"), "run", format!("symbol:{column}")],
        )
        .unwrap();
    let line = "included unfinished changes from task \"Frontend change\" in frontend";
    let dependency = "included unfinished changes from task \"Components change\" in components";
    crate::language::with_tool_path(&[root.join("bin")], || {
        let out = with_root(&root, || with_context("t", "run", || components.test(&[]))).unwrap();
        assert!(out.contains("== In frontend =="), "{out}");
        assert!(out.contains(line), "{out}");
        assert!(out.contains(dependency), "{out}");
        let entries = crate::board::runs::transcript(&log);
        assert!(entries.iter().any(|e| e == &json!({"kind":"usage","text":line})), "{entries:?}");
        assert!(entries.iter().any(|e| e == &json!({"kind":"usage","text":dependency})), "{entries:?}");
    });
}

/// A compile failure may come from anywhere in the crate, so the reply names
/// every unfinished task with a crate change, not only one a test reaches.
#[test]
fn a_compile_failure_names_the_unfinished_crate_changes() {
    if !crate::language::cargo::installed() {
        return;
    }
    let (root, store) = rust_project(
        "use foo_not_a_dep::Bar;\n\npub fn one() -> i32 {\n    1\n}\n\n#[test]\nfn it_works() {\n    assert_eq!(one(), 1);\n}\n",
    );
    let dir = crate::board::task_model::store_dir(&crate::scope::top(&root));
    fs::create_dir_all(&dir).unwrap();
    let log = root.join(".solace/run.log");
    fs::write(&log, "").unwrap();
    let put = |id: &str, title: &str, column: &str, running: bool| {
        let runs =
            if running { json!([{"id":"run","kind":"work","state":"running","log":log.to_string_lossy().to_string()}]) } else { json!([]) };
        let t = json!({
            "id":id,"title":title,"body":"","home":".","column":column,"rank":1.,
            "scope":["."],"dependsOn":[],"runs":runs,"notes":[],"questions":[],"links":[],
            "closedTime":null,"archivedTime":null,
        });
        fs::write(dir.join(format!("{id}.json")), t.to_string()).unwrap();
    };
    put("t", "Running task", "assigned", true);
    put("u", "Crate change", "assigned", false);
    put("v", "Doc change", "assigned", false);
    for (task, file) in [("t", "Cargo.toml"), ("u", "Cargo.toml"), ("v", "README.md")] {
        store
            .db
            .execute(
                "INSERT INTO board_touches(task,entity,run,name) VALUES (?1,?2,?3,?4)",
                rusqlite::params![task, format!("file:{file}"), "run", file],
            )
            .unwrap();
    }
    let line = "included unfinished changes from task \"Crate change\" in .";
    let err = with_root(&root, || with_context("t", "run", || store.test(&["--all"]).unwrap_err().0));
    assert!(err.contains(line), "{err}");
    assert!(!err.contains("Doc change") && !err.contains("Running task"), "{err}");
    assert!(crate::board::runs::transcript(&log).iter().any(|e| e == &json!({"kind":"usage","text":line})));
}
