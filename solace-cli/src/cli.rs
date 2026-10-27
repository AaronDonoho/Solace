//! Command-line parsing: a command's words, its `--project` scope and its
//! `solace` prefix.
#[cfg(test)]
#[path = "cli_tests.rs"]
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
