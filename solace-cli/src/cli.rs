//! The CLI facade over `routing`, `batching` and `projects`: the shared command
//! lists and `main`.

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
pub(super) mod batching;
pub(super) mod projects;
pub(super) mod routing;

pub(crate) use batching::{batch, is_question, looks_like_code, panic_reply, read_hint, reset_after_panic, verb_words};
pub(crate) use projects::{module_elsewhere, path_dependencies};
pub(crate) use routing::{next_step, operands, run_options, stdin, take};
#[cfg(test)]
pub(crate) use projects::{hinted, read_project, read_project_in};
#[cfg(test)]
pub(crate) use routing::{is_process, run_inner, with_root};

use crate::failure::{usage, Failure};
/// The largest reply, in bytes (about 8k tokens). Agents cut longer tool
/// output (Codex at about 10k tokens), so `inspect` and `view` trim by
/// priority to fit rather than being cut mid-document (D-14). `solace mcp
/// --budget <bytes>` sets it per agent.
pub(crate) const BUDGET: usize = 32_000;

// Commands by what they do. Every check that depends on the command uses these.
/// Plain reads; with `--project`, they read another project.
const READS: &[&str] = &["view", "inspect", "refs", "find", "grep", "files", "read", "design"];

/// Only for the human, never in a board run.
pub(crate) const HOST: &[&str] = &["agent-config", "serve", "create", "migrate"];

/// D-119: changes whose failure stops the call, so nothing after them runs on unsaved work.
pub(crate) const STOPS: &[&str] = &["submit", "delete", "extract", "merge", "write", "remove", "move"];

/// D-152: what a reply ends with when it held back a `task done`.
pub(crate) const HELD: &str = "`task done` waits for a test that passes and reaches every change: fix what the test reports, \
then send `task done` again with an updated summary.\n";

/// task:da797576d0cfaaac: a line left with no command once `--project <dir>` is
/// taken off is refused as that line's error; the rest of the call runs.
pub(crate) const PROJECT_ONLY: &str = "`--project` goes at the end of each command line it applies to, e.g. `view x.y --project <dir>`\n";

/// The reference's example of a first call, for a session start with no
/// ranked symbols (D-145).
pub(crate) const EXAMPLE_CALL: &str = "for example `inspect checkout.pay payments.capture --context`";

/// D-151: the most bytes of open work records a session start shows.
pub(crate) const WORK_CAP: usize = 4000;

/// D-145: what a reply ends with from the second single read in a row.
const READ_HINT: &str = "Hint: put independent reads in one call, one per line.\n";

/// The module name a probe runs as.
pub(crate) const PROBE: &str = "solace_probe";

pub(crate) fn main(mut args: Vec<String>) -> (String, i32) {
    let result = (|| {
        let mut options = Vec::new();
        while let Some(option) = take(&mut args, "--option")? {
            options.push(option);
        }
        let old = take(&mut args, "--old")?;
        if args.first().map(String::as_str) == Some("mcp") {
            let budget = match &args[1..] {
                [] => BUDGET,
                [flag, n] if flag == "--budget" => {
                    n.parse().map_err(|_| Failure(format!("--budget takes a number of bytes, got {n}"), 2))?
                }
                _ => return usage("mcp [--budget N]"),
            };
            crate::mcp::mcp(budget);
            return Ok(String::new());
        }
        run_options(&args, &mut stdin, old.as_deref(), BUDGET, &options)
    })();
    match result {
        Ok(out) => (out, 0),
        Err(Failure(msg, code)) => (format!("{msg}\n"), code),
    }
}
