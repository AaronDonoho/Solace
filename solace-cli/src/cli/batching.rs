use std::any::Any;
use std::path::Path;
use crate::failure::{Failure, Out};
use super::{HELD, PROJECT_ONLY, READ_HINT, READS, STOPS};
#[cfg(test)]
#[path = "batching_tests.rs"]
mod tests;
/// A command's words without `--project <folder>`, which may stand anywhere on the line.
pub(crate) fn verb_words(words: &[String]) -> Vec<&str> {
    let mut w: Vec<&str> = words.iter().map(String::as_str).collect();
    if let Some(i) = w.iter().position(|x| *x == "--project") {
        w.drain(i..(i + 2).min(w.len()));
    }
    w
}

pub(crate) fn bare_command(command: &str) -> &str {
    command.strip_prefix("solace").map_or(command, |c| c.trim_start())
}

/// A command's words, with the spaces in a trait impl's `<Type as Trait>` kept
/// in one word; an unbalanced `<` falls back to plain whitespace splitting.
pub(crate) fn command_words(command: &str) -> Vec<String> {
    let (mut words, mut word) = (Vec::new(), String::new());
    let (mut depth, mut prev) = (0usize, '\0');
    for c in command.chars() {
        match c {
            '<' => {
                depth += 1;
                word.push(c);
            }
            '>' if depth > 0 && prev != '-' => {
                depth -= 1;
                word.push(c);
            }
            c if c.is_whitespace() && depth > 0 => {
                if !word.chars().last().is_some_and(char::is_whitespace) {
                    word.push(' ');
                }
            }
            c if c.is_whitespace() && depth == 0 => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
        prev = c;
    }
    if !word.is_empty() {
        words.push(word);
    }
    if depth == 0 { words } else { command.split_whitespace().map(String::from).collect() }
}

/// Several commands, one per line, run in order; they share `budget`, since the
/// agent cuts the reply as a whole. D-119: after a failed change the rest are
/// skipped. D-152: after a test that failed or left a change no test reaches,
/// a `task done` is skipped. A repeated failed command names its copy. The
/// reply, and whether a command failed.
pub(crate) fn batch(commands: &[&str], budget: usize, run: &mut dyn FnMut(&[String], usize) -> Out) -> (String, bool) {
    let (mut out, mut failed, mut skipped, mut stopped) = (String::new(), vec![], vec![], false);
    let (mut waiting, mut held) = (None::<String>, false);
    for (at, &command) in commands.iter().enumerate() {
        let command = bare_command(command);
        if stopped {
            skipped.push(command.to_string());
            continue;
        }
        let words = command_words(command);
        let verb = verb_words(&words);
        if let Some(why) = &waiting
            && verb.starts_with(&["task", "done"])
        {
            skipped.push(format!("{command} ({why})"));
            held = true;
            continue;
        }
        if commands.len() > 1 {
            out += &format!("==== solace {command} ====\n");
        }
        if words.first().is_some_and(|w| w == "help") {
            out += &crate::reference::agent_help(words.get(1).map(|w| w.as_str()));
            continue;
        }
        // `--project` names where its command runs, not a command of its own.
        if verb.is_empty() && words.iter().any(|w| w == "--project") {
            out += PROJECT_ONLY;
            failed.push(failed_copy(command, commands, at));
            continue;
        }
        let test = verb.first() == Some(&"test");
        if test {
            crate::UNTESTED.with(|u| u.borrow_mut().take());
        }
        let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&words, budget.saturating_sub(out.len())))) {
            Ok(result) => result,
            Err(p) => {
                // A panic is this line's error, not the tool's: the lines after
                // it are skipped, and the next call starts clean.
                reset_after_panic();
                out += &panic_reply(&*p, Some(command));
                failed.push(failed_copy(command, commands, at));
                stopped = true;
                continue;
            }
        };
        if test && waiting.is_none() {
            waiting = match &result {
                Err(_) => Some("the test failed".into()),
                Ok(_) => crate::UNTESTED.with(|u| u.borrow_mut().take()).map(|names| format!("no test reaches {names}")),
            };
        }
        match result {
            Ok(o) => out += &o,
            Err(Failure(msg, _)) => {
                out += &msg;
                failed.push(failed_copy(command, commands, at));
                stopped = words.first().is_some_and(|w| STOPS.contains(&w.as_str()));
            }
        }
        if commands.len() > 1 && !out.ends_with('\n') {
            out.push('\n');
        }
    }
    // D-109, D-119: a batch names the commands that failed and those it skipped, for the agent and the run log.
    if commands.len() > 1 && !failed.is_empty() {
        out += &format!("{} of {} commands failed: {}", failed.len(), commands.len(), failed.join("; "));
        if !skipped.is_empty() {
            out += &format!("; skipped: {}", skipped.join("; "));
        }
        out.push('\n');
    } else if !skipped.is_empty() {
        out += &format!("Skipped: {}\n", skipped.join("; "));
    }
    if held {
        out += HELD;
    }
    crate::UNTESTED.with(|u| u.borrow_mut().take());
    (out, !failed.is_empty())
}

/// What a panic replies to the agent, whatever its payload carries: the message
/// (`&str` or `String`), and the line it happened in, if it was inside one.
pub(crate) fn panic_reply(p: &(dyn Any + Send), line: Option<&str>) -> String {
    let message = if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    };
    let at = line.map_or(String::new(), |l| format!(" running `{l}`"));
    format!("Solace hit an internal error{at}: {message}; nothing after it ran\n")
}

/// Drops what a caught panic left with this thread — the untested changes a
/// `test` named, and any outer transaction it held open — so the next call
/// starts clean. A transaction's own drop guard rolls back as the panic
/// unwinds; this covers a panic in a place without one.
pub(crate) fn reset_after_panic() {
    crate::UNTESTED.with(|u| u.borrow_mut().take());
    let _ = crate::COMMAND_DB.try_with(|c| {
        let Ok(mut dbs) = c.try_borrow_mut() else { return };
        for (_, db) in std::mem::take(&mut *dbs) {
            if !db.is_autocommit() {
                let _ = db.execute_batch("ROLLBACK");
            }
        }
    });
}

/// D-145: the hint a call's reply ends with, from the second call in a row
/// that held one read and succeeded; `streak` counts those calls, and any
/// other call starts it again.
pub(crate) fn read_hint(streak: &mut usize, commands: &[&str], failed: bool) -> &'static str {
    let read = match commands {
        [command] if !failed => {
            let mut words: Vec<&str> = command.split_whitespace().collect();
            if words.first() == Some(&"solace") {
                words.remove(0);
            }
            if let Some(i) = words.iter().position(|w| *w == "--project") {
                words.drain(i..(i + 2).min(words.len()));
            }
            words.first().is_some_and(|w| READS.contains(w))
        }
        _ => false,
    };
    *streak = if read { *streak + 1 } else { 0 };
    if *streak >= 2 { READ_HINT } else { "" }
}

/// A command that asks the human something: `task ask`, a strategy run's
/// proposal (D-93), or a run's write of a protected file (a settings proposal).
/// It must be the only command in its call.
pub(crate) fn is_question(line: &str) -> bool {
    let w: Vec<&str> = line.strip_prefix("solace").unwrap_or(line).split_whitespace().collect();
    match w.as_slice() {
        ["task", "ask", ..] | ["strategy", "task" | "feature" | "milestone" | "vision", ..] => true,
        ["write", rest @ ..] if crate::board::context().is_some() => {
            let root = crate::Store::root().ok();
            rest.iter().any(|p| root.as_deref().is_some_and(|r| crate::lockdown::settings_file(r, &crate::file_name(Path::new(p)))))
        }
        _ => false,
    }
}

/// A `command` line that is Gleam or Rust code, not a Solace command.
pub(crate) fn looks_like_code(line: &str) -> bool {
    let first = line.split_whitespace().next().unwrap_or("");
    matches!(first, "let" | "echo" | "assert" | "use" | "case" | "pub" | "fn" | "import" | "}" | "//") || first.contains('(')
}

/// Names a repeated command's copy (`check (copy 2)`), so its section can be
/// found; a command that appears once is named as written.
fn failed_copy(command: &str, commands: &[&str], at: usize) -> String {
    if commands.iter().filter(|c| bare_command(c) == command).count() < 2 {
        return command.to_string();
    }
    let copy = commands[..=at].iter().filter(|c| bare_command(c) == command).count();
    format!("{command} (copy {copy})")
}
