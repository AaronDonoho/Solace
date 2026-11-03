//! The embedded agent reference, and the views of it that session start and
//! `help` give. It is kept apart from the CLI so main.rs stays navigable.

use crate::board::runs::context;
use crate::board::runs::strategy_context;
/// The reference's sections, joined at compile time in reading order.
pub(crate) const REFERENCE: &str = concat!(
    include_str!("reference/workflow.txt"),
    include_str!("reference/reading.txt"),
    include_str!("reference/changing.txt"),
    include_str!("reference/checking.txt"),
    include_str!("reference/rust.txt"),
    include_str!("reference/files.txt"),
    include_str!("reference/skills.txt"),
    include_str!("reference/project.txt"),
    include_str!("reference/work.txt"),
    include_str!("reference/task-runs.txt"),
    include_str!("reference/strategy-runs.txt"),
    include_str!("reference/host.txt"),
);

/// The reference as agents get it at session start: the workflow and one line
/// per command (`help` has every option, D-46), and task commands only in a
/// board run (`in_task`).
pub(crate) fn agent_reference(in_task: bool) -> String {
    let (workflow, rest) = REFERENCE.split_once("\nCommands in full").unwrap();
    let (_, rest) = rest.split_once("\nTask runs").unwrap();
    let (task, rest) = rest.split_once("\nStrategy runs").unwrap();
    let (strategy, _host) = rest.split_once("\nHost commands").unwrap();
    if in_task {
        format!("{workflow}\nTask runs{task}")
    } else if strategy_context().is_some() {
        // The workflow is for changing code, which a strategy run never does.
        format!("Strategy runs{strategy}\nRun `help` for the read commands' options.\n")
    } else {
        // no task to ask or finish outside a board run (run 18)
        format!(
            "{}\n",
            workflow
                .replace("ask with `task ask`, or say so", "say so")
                .replace("every part\n   of the task is changed", "every part\n   of the change is made")
                .replace("the code is done: finish with `task done`.", "the code is done.")
                .replace("A project outside the task's reach: ask with `task ask` to add it.\nName each project's folder exactly as the task's scope lists it (for example\n`solace-cli/components`), say in a few words what you would change there,\nand tell the human to first add it in the task's Scope tab and then answer;\nno option's wording says that pressing it adds the project. Then make the\nchange there. ", "")
        )
    }
}

/// The reference's file-tier rows are not commands: `help`'s name list skips
/// them (D-144).
pub(crate) const FILE_TIERS: [&str; 5] = ["graph", "watched", "protected", "unwatched", "internal"];

/// The command names a row's term lists: the first word of each part split on
/// a `|` outside `<…>` and `[…]` (`view | inspect | refs | find`).
pub(crate) fn row_names(l: &str) -> Vec<&str> {
    let term = l.trim_start().split("  ").next().unwrap_or("");
    fn first(part: &str) -> Option<&str> {
        part.split_whitespace().next()
    }
    let (mut out, mut depth, mut start) = (vec![], 0usize, 0usize);
    for (i, c) in term.char_indices() {
        match c {
            '<' | '[' => depth += 1,
            '>' | ']' if depth > 0 => depth -= 1,
            '|' if depth == 0 => {
                out.extend(first(&term[start..i]));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.extend(first(&term[start..]));
    out
}

/// The first call a session start ends with (D-145); a strategy run gets no
/// workflow, so none.
pub(crate) fn next_call(call: &str, strategy: bool) -> String {
    if strategy { String::new() } else { format!("\nNext: read what the change needs in one call, one command per line: {call}.\n") }
}

/// `help [command]` for agents: every option, without host commands, and task
/// commands only in a board run; with a command, only the paragraph holding
/// its entry — a row indented by exactly two naming it (D-46 point 4, as
/// amended by D-144); with no entry, the run's command names.
pub(crate) fn agent_help(topic: Option<&str>) -> String {
    agent_help_for(topic, context().is_some(), strategy_context().is_some())
}

/// `agent_help` with the run's mode given, so a test can read a strategy run's help.
pub(crate) fn agent_help_for(topic: Option<&str>, in_task: bool, strategy_run: bool) -> String {
    let (_, full) = REFERENCE.split_once("Commands in full\n").unwrap();
    let (full, _host) = full.split_once("\nHost commands").unwrap();
    let (general, runs) = full.split_once("\nTask runs").unwrap();
    let (task, strategy) = runs.split_once("\nStrategy runs").unwrap();
    let full = if in_task {
        format!("{general}\nTask runs{task}")
    } else if strategy_run {
        // Only what a strategy run may do: the reads, and its own commands.
        let reading = general.split("\n\n").next().unwrap_or("");
        format!("{reading}\n\nStrategy runs{strategy}")
    } else {
        general.to_string()
    };
    let Some(topic) = topic else { return format!("{full}\n") };
    let hits = paragraphs_naming(&full, topic);
    if !hits.is_empty() {
        return hits.join("\n\n") + "\n";
    }
    if strategy_run {
        let worker = format!("{general}\nTask runs{task}");
        let hits = paragraphs_naming(&worker, topic);
        if !hits.is_empty() {
            return format!("Worker command: a strategy run can't run it.\n\n{}\n", hits.join("\n\n"));
        }
    }
    let mut list: Vec<&str> = vec![];
    for l in full.lines().filter(|l| l.starts_with("  ") && !l[2..].starts_with(' ')) {
        for w in row_names(l) {
            if w != "..." && !FILE_TIERS.contains(&w) && !list.contains(&w) {
                list.push(w);
            }
        }
    }
    format!("No command `{topic}`. Commands: {}\n", list.join(", "))
}

/// The paragraphs with a row naming `topic`.
fn paragraphs_naming<'a>(text: &'a str, topic: &str) -> Vec<&'a str> {
    let entry = |l: &str| l.starts_with("  ") && !l[2..].starts_with(' ') && row_names(l).iter().any(|w| *w == topic);
    text.split("\n\n").filter(|p| p.lines().any(entry)).collect()
}
