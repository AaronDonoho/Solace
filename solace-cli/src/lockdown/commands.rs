use std::time::Duration;
use std::time::Instant;
use std::process::Command;
use std::fs;
use std::path::Path;
use std::process::Stdio;
use crate::unique_id;
use crate::failure::Failure;
use crate::failure::Out;
use crate::lockdown::error;
#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
use crate::diagnostics;
use crate::migrate;

pub fn trim_output(text: &str, budget: usize) -> String {
    if text.len() <= budget {
        return text.into();
    }
    let lines: Vec<_> = text.lines().collect();
    let mut head = String::new();
    let mut kept = 0;
    for l in lines.iter().take(20) {
        if head.len() + l.len() + 100 > budget {
            break;
        }
        head += l;
        head.push('\n');
        kept += 1;
    }
    let mut tail = Vec::new();
    let mut used = head.len() + 80;
    for l in lines[kept..].iter().rev() {
        if used + l.len() + 1 > budget {
            break;
        }
        used += l.len() + 1;
        tail.push(*l);
    }
    tail.reverse();
    let result = format!("{head}... {} lines omitted ...\n{}\n", lines.len() - kept - tail.len(), tail.join("\n"));
    result[..result.floor_char_boundary(budget.min(result.len()))].into()
}

/// Whether `l` is a build script's warning, `warning: solace@0.1.0: …`: cargo prefixes it with
/// the package and its version, which no compiler warning has.
fn build_script_warning(l: &str) -> bool {
    let spec = l.strip_prefix("warning: ").unwrap_or_default().split(": ").next().unwrap_or_default();
    spec.split_once('@').is_some_and(|(_, version)| version.starts_with(|c: char| c.is_ascii_digit()))
}

/// A `run` or `build` reply's output (D-111): failures, errors, panics and
/// summaries, in order. Build progress, passing tests and the text of
/// warnings are left out; warnings stay as cargo's count line, or a count.
/// Only cargo's, libtest's and Gleam's own forms match, so other programs'
/// output passes through.
pub fn condense(output: &str) -> String {
    const PROGRESS: [&str; 13] = [
        "Compiling ",
        "Compiled ",
        "Checking ",
        "Finished ",
        "Running ",
        "Doc-tests ",
        "Downloading ",
        "Downloaded ",
        "Blocking ",
        "Updating ",
        "Locking ",
        "Adding ",
        "Fresh ",
    ];
    let (mut out, mut warnings, mut counted) = (String::new(), 0, false);
    // `None` outside a warning; `Some(true)` in one of Gleam's, framed by `┌─`
    let mut warning: Option<bool> = None;
    // a `test result` with nothing run is kept only when no other is
    let (mut results, mut empty) = (false, None);
    for l in output.lines() {
        let t = l.trim();
        // cargo's and Gleam's status lines are indented, the verb right-aligned
        let progress = l.starts_with(' ') && PROGRESS.iter().any(|p| t.starts_with(p));
        // libtest's `running 3 tests`
        let header = l
            .strip_prefix("running ")
            .and_then(|r| r.strip_suffix(" tests").or_else(|| r.strip_suffix(" test")))
            .is_some_and(|n| n.parse::<usize>().is_ok());
        if l.starts_with("warning:") {
            // cargo's `warning: `x` (bin "x") generated 3 warnings`
            if l.contains(" generated ") && l.contains(" warning") {
                counted = true;
                warning = None;
                out += &format!("{l}\n");
            } else if build_script_warning(l) {
                // A build script's warning is the script's own message, one line naming the step
                // to run, not a compiler note: it stays whole (D-159).
                warning = None;
                out += &format!("{l}\n");
            } else {
                warning = Some(false);
                warnings += 1;
            }
            continue;
        }
        match warning {
            // Gleam's has blank lines in its text: it runs until the build goes on
            Some(true) if !(progress || l.starts_with("error") || t.starts_with("test ")) => continue,
            Some(false) if t.contains('┌') => {
                warning = Some(true);
                continue;
            }
            // cargo's is its indented or numbered lines, up to a blank line
            Some(false) if t.is_empty() => {
                warning = None;
                continue;
            }
            Some(false) if l.starts_with(|c: char| c.is_whitespace() || c.is_ascii_digit()) => continue,
            _ => warning = None,
        }
        if progress || header {
            continue;
        }
        if t.starts_with("test result: ok. 0 passed; 0 failed;") {
            empty = Some(l);
            continue;
        }
        let passed = (t.starts_with("test") && (t.ends_with(" ... ok") || t.ends_with(" ... ignored")))
            || l.starts_with("ok    ")
            || (!t.is_empty() && t.chars().all(|c| c == '.'));
        if passed || (t.is_empty() && (out.is_empty() || out.ends_with("\n\n"))) {
            continue;
        }
        results |= t.starts_with("test result:");
        out += &format!("{l}\n");
    }
    let mut out = out.trim_end().to_string();
    if let Some(l) = empty.filter(|_| !results) {
        out = if out.is_empty() { l.to_string() } else { format!("{out}\n{l}") };
    }
    if !out.is_empty() {
        out.push('\n');
    }
    if warnings > 0 && !counted {
        out += &format!("{warnings} warning{} not shown\n", if warnings == 1 { "" } else { "s" });
    }
    out
}

/// Kills `child` and every process it started (its process group).
pub fn kill_group(child: &mut std::process::Child) {
    let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", child.id())]).status();
    let _ = child.kill();
    let _ = child.wait();
}

/// Waits for `child`, started in its own process group, up to `timeout`;
/// past it, kills the group and returns `None`.
pub fn wait_or_kill(child: &mut std::process::Child, timeout: Duration) -> std::io::Result<Option<std::process::ExitStatus>> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if start.elapsed() >= timeout {
            kill_group(child);
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Runs `argv` in `root`: the exit code and elapsed time, then the output
/// with terminal color codes removed, within `budget`, `condense`d for a
/// `run` or `build` reply (D-111).
pub fn command(root: &Path, argv: &[String], timeout: u64, budget: usize, condensed: bool, sandbox: bool) -> Out {
    use std::os::unix::process::CommandExt;
    let mut cmd = prepared(root, argv, sandbox)?;
    let log = root.join(".solace").join(format!("command-{}.log", unique_id()));
    let out = fs::File::create(&log)?;
    let mut child = cmd.process_group(0).stdout(out.try_clone()?).stderr(out).spawn()?;
    let start = Instant::now();
    let result = match wait_or_kill(&mut child, Duration::from_secs(timeout))? {
        Some(status) => Ok(format!("exit code: {} ({:.1}s)\n", status.code().unwrap_or(-1), start.elapsed().as_secs_f64())),
        None => error("E6012", &argv[0], format!("timed out after {timeout}s")),
    };
    let output = diagnostics::strip_ansi(&fs::read_to_string(&log)?);
    fs::remove_file(log)?;
    let output = if condensed { condense(&output) } else { output };
    match result {
        Ok(prefix) => Ok(prefix.clone() + &trim_output(&output, budget.saturating_sub(prefix.len()))),
        Err(Failure(message, code)) => Err(Failure(message + &trim_output(&output, budget), code)),
    }
}

/// Starts `argv` in `root` in the background, without waiting: the same
/// sandbox and process group as `command`, its output dropped, and a thread
/// reaping it (`cargo::warm`, D-158 point 3).
pub fn background(root: &Path, argv: &[String], sandbox: bool) -> Out {
    use std::os::unix::process::CommandExt;
    let mut cmd = prepared(root, argv, sandbox)?;
    let mut child = cmd.process_group(0).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(String::new())
}

/// Whether a `command` reply is from a command that exited with 0.
pub fn passed(out: &str) -> bool {
    out.starts_with("exit code: 0 (")
}

/// The command `command` and `background` start: the project's tool, with the
/// agent's task context removed (D-71).
fn prepared(root: &Path, argv: &[String], sandbox: bool) -> Result<std::process::Command, Failure> {
    fs::create_dir_all(root.join(".solace"))?;
    // `sandbox` is a Rust check, test or build on Linux (D-158 point 4); a
    // migration's original is read-only to commands in the copy (Spec 17).
    let mut cmd =
        if sandbox { migrate::cargo_sandboxed(argv, root)? } else { migrate::sandboxed(argv, root, migrate::protected(root).as_deref()) };
    if !sandbox {
        migrate::drop_task_context(&mut cmd);
    }
    Ok(cmd)
}
