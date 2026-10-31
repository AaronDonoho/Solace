//! Just enough Gleam for the Phase 1a store: split `gleam format`ted source
//! into top-level symbols, parse imports, and find the names a symbol uses.
//! Formatted Gleam puts every top-level item at column 0, which is what makes
//! a line-based splitter reliable. `gleam format` itself is the syntax check.

use crate::language::{Import, Item, Kind, Language, Module, Ref, body_start, split_args};
use crate::diagnostics::strip_ansi;
use crate::diagnostics::locate;
use crate::failure::fail;
use crate::diagnostics::FileMap;
use crate::failure::Failure;
use crate::diagnostics::Diag;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Gleam behind the language interface.
pub struct Gleam;

impl Language for Gleam {
    fn manifest(&self) -> &'static str {
        "gleam.toml"
    }
    fn tool(&self) -> &'static str {
        "gleam"
    }
    fn split(&self, source: &str) -> Module {
        split(source)
    }
    fn declaration(&self, line: &str) -> Option<(Kind, String)> {
        declaration(line)
    }
    fn is_leading(&self, line: &str) -> bool {
        is_leading(line)
    }
    fn imports(&self, header: &str) -> Vec<Import> {
        imports(header)
    }
    fn references(&self, code: &str) -> Vec<Ref> {
        references(code)
    }
    fn references_at(&self, code: &str) -> Vec<(Ref, usize)> {
        references_at(code)
    }
    fn references_spans(&self, code: &str) -> Vec<(Ref, usize, usize)> {
        references_spans(code)
    }
    fn bound_names(&self, code: &str) -> std::collections::HashSet<String> {
        bound_names(code)
    }
    fn constructors(&self, type_text: &str) -> Vec<String> {
        constructors(type_text)
    }
    fn record_fields(&self, type_text: &str) -> Vec<(String, Vec<Option<String>>)> {
        record_fields(type_text)
    }
    fn field_reads(&self, code: &str) -> Vec<(String, usize)> {
        field_reads(code)
    }
    fn pattern_names(&self, pattern: &str) -> Vec<String> {
        pattern_names(pattern)
    }
    fn strip_comments(&self, source: &str) -> String {
        strip_comments(source)
    }
    fn signature(&self, item: &Item) -> String {
        signature(item)
    }
    fn head(&self, item: &Item) -> String {
        head(item)
    }
    fn source_files(&self, dir: &Path) -> Result<Vec<String>, Failure> {
        source_files(dir)
    }
    fn valid_module_path(&self, m: &str) -> bool {
        valid_module_path(m)
    }
    fn format(&self, source: &str) -> Result<String, Failure> {
        format(source)
    }
    fn diagnostics(&self, output: &str, files: &[FileMap]) -> Vec<Diag> {
        diagnostics(output, files)
    }
    fn relocate(&self, line: &str, files: &[FileMap]) -> String {
        relocate(line, files)
    }
    fn failures(&self, output: &str) -> BTreeMap<String, (String, String)> {
        failures(output)
    }
    fn quiet(&self, output: &str) -> String {
        quiet(output)
    }
}

/// Kind and name when `line` starts a top-level item.
pub fn declaration(line: &str) -> Option<(Kind, String)> {
    let mut rest = line.strip_prefix("pub ").unwrap_or(line);
    rest = rest.strip_prefix("opaque ").unwrap_or(rest);
    let (kind, rest) = if let Some(r) = rest.strip_prefix("fn ") {
        (Kind::Fn, r)
    } else if let Some(r) = rest.strip_prefix("type ") {
        (Kind::Type, r)
    } else {
        let r = rest.strip_prefix("const ")?;
        (Kind::Const, r)
    };
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    (!name.is_empty()).then_some((kind, name))
}

/// Comment or attribute lines that belong to the item below them.
pub fn is_leading(line: &str) -> bool {
    (line.starts_with("//") && !line.starts_with("////")) || line.starts_with('@')
}

pub fn split(source: &str) -> Module {
    let lines: Vec<&str> = source.lines().collect();
    let mut starts = vec![]; // (first line incl. leading comments, kind, name)
    for (i, line) in lines.iter().enumerate() {
        if let Some((kind, name)) = declaration(line) {
            let mut first = i;
            while first > 0 && is_leading(lines[first - 1]) {
                first -= 1;
            }
            starts.push((first, kind, name));
        }
    }
    let header_end = starts.first().map_or(lines.len(), |s| s.0);
    let mut items = vec![];
    for (n, (first, kind, name)) in starts.iter().enumerate() {
        let end = starts.get(n + 1).map_or(lines.len(), |s| s.0);
        let text = lines[*first..end].join("\n").trim_end().to_string();
        items.push(Item { kind: *kind, name: name.clone(), text });
    }
    Module { header: lines[..header_end].join("\n").trim().to_string(), items }
}

/// Constructor names defined by a type's text.
pub fn constructors(type_text: &str) -> Vec<String> {
    type_text
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("  ")?;
            let first = rest.chars().next()?;
            first.is_uppercase().then(|| rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect())
        })
        .collect()
}

/// Each constructor of a type with its fields' labels, in position order.
pub fn record_fields(type_text: &str) -> Vec<(String, Vec<Option<String>>)> {
    let code = strip_comments(type_text);
    let mut out = vec![];
    for (at, line) in code.split_inclusive('\n').scan(0, |at, l| Some((std::mem::replace(at, *at + l.len()), l))) {
        let Some(rest) = line.strip_prefix("  ").filter(|r| r.starts_with(|c: char| c.is_uppercase())) else { continue };
        let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        let open = at + 2 + name.len();
        let fields = if code[open..].starts_with('(') { split_args(&code[open + 1..]) } else { vec![] };
        let label = |p: &str| {
            let (l, _) = p.split_once(':')?;
            let l = l.trim();
            (l.starts_with(|c: char| c.is_lowercase()) && l.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| l.to_string())
        };
        out.push((name, fields.iter().map(|(_, p)| label(p)).collect()));
    }
    out
}

/// An item's doc comment, whole (D-65), then its `head`. Other leading
/// comments are dropped.
pub fn signature(item: &Item) -> String {
    let doc: String =
        item.text.lines().take_while(|l| declaration(l).is_none()).filter(|l| l.starts_with("///")).map(|l| format!("{l}\n")).collect();
    doc + &head(item)
}

/// The declaration part of an item: a function up to its body, a type or
/// constant in full.
pub fn head(item: &Item) -> String {
    let decl_start = item.text.lines().position(|l| declaration(l).is_some()).unwrap_or(0);
    let decl: String = item.text.lines().skip(decl_start).collect::<Vec<_>>().join("\n");
    match item.kind {
        Kind::Fn => decl[..body_start(&decl).unwrap_or(decl.len())].trim_end().to_string(),
        _ => decl,
    }
}

pub fn imports(header: &str) -> Vec<Import> {
    let code = strip_comments(header);
    let mut out = vec![];
    let mut rest = code.as_str();
    while let Some(at) = rest.find("import ") {
        let at_line_start = at == 0 || rest[..at].ends_with('\n');
        rest = &rest[at + 7..];
        if !at_line_start {
            continue;
        }
        let module: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '/').collect();
        rest = &rest[module.len()..];
        let mut unqualified = vec![];
        if let Some(r) = rest.strip_prefix(".{") {
            let close = r.find('}').unwrap_or(r.len());
            for part in r[..close].split(',') {
                let part = part.trim();
                let part = part.strip_prefix("type ").unwrap_or(part).trim();
                if part.is_empty() {
                    continue;
                }
                let mut words = part.split(" as ");
                let name = words.next().unwrap().trim().to_string();
                let local = words.next().map_or(name.clone(), |s| s.trim().to_string());
                unqualified.push((name, local));
            }
            rest = &r[(close + 1).min(r.len())..];
        }
        let line_end = rest.find('\n').unwrap_or(rest.len());
        let alias = match rest[..line_end].trim().strip_prefix("as ") {
            Some(a) => a.trim().to_string(),
            None => module.rsplit('/').next().unwrap_or(&module).to_string(),
        };
        out.push(Import { module, alias, unqualified });
    }
    out
}

/// Source with string literals and comments blanked out with spaces. Line
/// breaks are kept and every dropped character becomes as many spaces as it
/// has bytes, so lines, columns and byte offsets still match the source's.
pub fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let pad = |out: &mut String, c: char| out.push_str(&" ".repeat(c.len_utf8()));
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push('"');
                while let Some(c) = chars.next() {
                    if c == '\\' {
                        pad(&mut out, c);
                        match chars.next() {
                            Some('\n') => out.push('\n'),
                            Some(e) => pad(&mut out, e),
                            None => {}
                        }
                    } else if c == '"' {
                        break;
                    } else if c == '\n' {
                        out.push('\n');
                    } else {
                        pad(&mut out, c);
                    }
                }
                out.push('"');
            }
            '/' if chars.peek() == Some(&'/') => {
                pad(&mut out, c);
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                    pad(&mut out, c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Every top-level name a piece of code mentions, as written. Names bound
/// locally (parameters, `let`, `use`, lambda parameters, case patterns) are
/// left out within their scope, so `let tax = tax.quote(x)` still refers to
/// the `tax` module but a later `tax.total` does not. Scopes follow the
/// formatter's layout: a lambda parameter lasts until the lambda's closing
/// brace; a `let`, `use` or case-pattern binding until the indentation drops
/// below the line that introduced it.
pub fn references(code: &str) -> Vec<Ref> {
    scan(code).0.into_iter().map(|r| r.0).collect()
}

/// `references`, each with the line (from 0) it is on.
pub fn references_at(code: &str) -> Vec<(Ref, usize)> {
    scan(code).0.into_iter().map(|(r, line, _)| (r, line)).collect()
}

/// `references`, each with its byte span in `code`: `alias.name` as a whole, or
/// the bare name.
pub fn references_spans(code: &str) -> Vec<(Ref, usize, usize)> {
    let stripped = strip_comments(code);
    let at: Vec<usize> = stripped.char_indices().map(|(b, _)| b).chain([stripped.len()]).collect();
    scan(code).0.into_iter().map(|(r, _, (a, b))| (r, at[a], at[b])).collect()
}

/// The names bound locally anywhere in `code`: parameters, `let`, `use` and
/// case patterns.
pub fn bound_names(code: &str) -> std::collections::HashSet<String> {
    let chars: Vec<char> = strip_comments(code).chars().collect();
    let line_end = |i: usize| chars[i..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| i + p);
    local_bindings(&chars, &line_end).into_iter().map(|(n, _, _)| n).collect()
}

/// The record fields a piece of code reads, each with its line: `x.field` on
/// a local `x`, a field of a field or of a call's result (`f(x).field`), and
/// labels in constructor patterns (`Quote(total_cents: t, ..) ->`).
// positional pattern fields (`let Square(n) = s`) aren't counted as reads.
pub fn field_reads(code: &str) -> Vec<(String, usize)> {
    scan(code).1
}

fn scan(code: &str) -> (Vec<(Ref, usize, (usize, usize))>, Vec<(String, usize)>) {
    let code = strip_comments(code);
    let chars: Vec<char> = code.chars().collect();
    let mut line_of = Vec::with_capacity(chars.len());
    let mut n = 0;
    for &c in &chars {
        line_of.push(n);
        n += (c == '\n') as usize;
    }
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let word_at = |i: usize| -> (String, usize) {
        let mut j = i;
        while j < chars.len() && is_ident(chars[j]) {
            j += 1;
        }
        (chars[i..j].iter().collect(), j)
    };
    let line_end = |i: usize| chars[i..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| i + p);
    let bindings = local_bindings(&chars, &line_end);
    let bound = |name: &str, at: usize| bindings.iter().any(|(n, from, to)| n == name && *from <= at && at < *to);
    let lower = |s: &str| s.starts_with(|c: char| c.is_lowercase());

    let (mut refs, mut reads) = (vec![], vec![]);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let starts_ident = (c.is_alphabetic() || c == '_') && (i == 0 || !is_ident(chars[i - 1]));
        if !starts_ident {
            i += 1;
            continue;
        }
        // `..base` in a record update is a value, not a field
        let after_dot = i > 0 && chars[i - 1] == '.' && !(i > 1 && chars[i - 2] == '.');
        let (name, j) = word_at(i);
        let label = j < chars.len() && chars[j] == ':' && chars.get(j + 1) != Some(&':');
        if after_dot && lower(&name) {
            reads.push((name.clone(), line_of[i]));
        }
        if j + 1 < chars.len() && chars[j] == '.' && chars[j + 1].is_alphabetic() && !after_dot {
            let (member, k) = word_at(j + 1);
            if !bound(&name, i) {
                refs.push((Ref::Qualified(name, member), line_of[i], (i, k)));
            } else if lower(&member) {
                reads.push((member, line_of[i]));
            }
            i = k;
        } else {
            if !after_dot && !label && !bound(&name, i) {
                refs.push((Ref::Bare(name), line_of[i], (i, j)));
            }
            i = j;
        }
    }
    // labels inside constructor patterns: case arms, and `let`/`use` patterns
    for (n, line) in code.split('\n').enumerate() {
        let t = line.trim_start();
        let pattern = if let Some(rest) = t.strip_prefix("let ").or_else(|| t.strip_prefix("use ")) {
            rest.split(['=', '<']).next().unwrap_or("")
        } else if is_arm(t) {
            &t[..t.find(" ->").unwrap()]
        } else {
            continue;
        };
        let (mut depth, mut word) = (0, String::new());
        for c in pattern.chars() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                ':' if depth > 0 && lower(&word) => reads.push((std::mem::take(&mut word), n)),
                _ => {}
            }
            if is_ident(c) {
                word.push(c)
            } else if c != ' ' {
                word.clear()
            }
        }
    }
    (refs, reads)
}

/// A case arm's line: `Pattern(a, b) ->` (the pattern before the arrow).
fn is_arm(t: &str) -> bool {
    t.find(" ->").is_some_and(|arrow| {
        !t.starts_with("fn") && !t.starts_with("let ") && !t.starts_with("use ") && !t[..arrow].contains('=') && !t[..arrow].contains("fn(")
    })
}

/// (name, first position in scope, first position out of scope), as char
/// positions.
fn local_bindings(chars: &[char], line_end: &dyn Fn(usize) -> usize) -> Vec<(String, usize, usize)> {
    let text: String = chars.iter().collect();
    let len = chars.len();
    let line_start = |i: usize| chars[..i].iter().rposition(|&c| c == '\n').map_or(0, |p| p + 1);
    let indent = |start: usize| chars[start..].iter().take_while(|&&c| c == ' ').count();
    // first later line (non-blank) indented less than `level` (or at most, if `inclusive`)
    let dedent = |from: usize, level: usize, inclusive: bool| -> usize {
        let mut i = line_end(from);
        while i < len {
            let start = i + 1;
            let n = indent(start);
            let blank = start + n >= len || chars[start + n] == '\n';
            if !blank && (n < level || (inclusive && n <= level)) {
                return start;
            }
            i = line_end(start);
        }
        len
    };
    let char_pos = |byte: usize| text[..byte].chars().count();
    let mut out = vec![];
    // let / use
    for (kw, stop) in [("let ", "="), ("use ", "<-")] {
        let mut from = 0;
        while let Some(p) = text[from..].find(kw) {
            let at = from + p;
            from = at + kw.len();
            if at > 0 && text[..at].chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            let rest = &text[from..];
            let Some(stop_at) = rest.find(stop) else { continue };
            let pattern = &rest[..stop_at];
            // drop type annotations: `let x: Int =`
            let pattern: String = pattern.split(':').next().unwrap_or("").to_string();
            let at_char = char_pos(at);
            let level = indent(line_start(at_char));
            // bound after the whole right-hand side, which may run over several
            // lines: `use customer <- result.try(\n  customer.Customer(..` still
            // refers to the module
            let bound_from = line_end(char_pos(from + stop_at)).max(dedent(at_char, level, true));
            let bound_to = dedent(at_char, level, false);
            for w in pattern_names(&pattern) {
                // the pattern itself names the binding, not a symbol
                out.push((w.clone(), char_pos(from), char_pos(from + stop_at)));
                out.push((w, bound_from, bound_to));
            }
        }
    }
    // function and lambda parameters
    let mut from = 0;
    while let Some(p) = text[from..].find("fn") {
        let at = from + p;
        from = at + 2;
        let before_ok = at == 0 || !text[..at].chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_');
        let rest = &text[from..];
        let Some(open) = rest.find('(') else { continue };
        let between = rest[..open].trim();
        if !before_ok || !between.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        // the body: from the first `{` after the parameters to its match
        let after_params = {
            let mut d = 0;
            let mut end = rest.len();
            for (k, c) in rest[open..].char_indices() {
                match c {
                    '(' => d += 1,
                    ')' => {
                        d -= 1;
                        if d == 0 {
                            end = open + k + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            end
        };
        let body_end = match rest[after_params..].find('{') {
            Some(b) => {
                let mut d = 0;
                let mut end = text.len();
                for (k, c) in rest[after_params + b..].char_indices() {
                    match c {
                        '{' => d += 1,
                        '}' => {
                            d -= 1;
                            if d == 0 {
                                end = from + after_params + b + k;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                char_pos(end)
            }
            None => len,
        };
        let mut depth = 0;
        let mut param = String::new();
        let mut params = vec![];
        for c in rest[open..].chars() {
            match c {
                '(' => {
                    depth += 1;
                    if depth == 1 {
                        continue;
                    }
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        params.push(std::mem::take(&mut param));
                        break;
                    }
                }
                ',' if depth == 1 => {
                    params.push(std::mem::take(&mut param));
                    continue;
                }
                _ => {}
            }
            param.push(c);
        }
        for p in params {
            let before_type = p.split(':').next().unwrap_or("");
            if let Some(name) = before_type.split_whitespace().last()
                && name.chars().next().is_some_and(|c| c.is_lowercase() || c == '_')
            {
                out.push((name.to_string(), char_pos(at), body_end));
            }
        }
    }
    // case arm patterns: `Pattern(a, b) ->` at the start of a line
    let mut pos = 0;
    for line in text.split('\n') {
        let t = line.trim_start();
        if is_arm(t) {
            let pattern = t[..t.find(" ->").unwrap()].split(" if ").next().unwrap_or("");
            let level = line.len() - t.len();
            let to = dedent(pos, level, true);
            for w in pattern_names(pattern) {
                out.push((w, pos, to));
            }
        }
        pos += line.chars().count() + 1;
    }
    out
}

/// The names a pattern binds: its lowercase words, leaving out the module
/// part of `mod.Ctor`.
pub fn pattern_names(s: &str) -> Vec<String> {
    let mut words = vec![];
    for w in s.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')) {
        for (k, part) in w.split('.').enumerate() {
            if k == 0 && w.contains('.') {
                continue;
            }
            if part.chars().next().is_some_and(|c| c.is_lowercase() || c == '_') && !KEYWORDS.contains(&part) {
                words.push(part.to_string());
            }
        }
    }
    words
}

const KEYWORDS: &[&str] = &["assert", "let", "use", "case", "if", "fn", "pub", "todo", "panic", "as", "echo"];

/// A Gleam module path: lowercase names of letters, digits and `_`, joined by `/`.
pub fn valid_module_path(m: &str) -> bool {
    m.split('/').all(|p| {
        p.starts_with(|c: char| c.is_ascii_lowercase()) && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    })
}

/// The Gleam files in a project's `src/` and `test/`, as sorted paths from `dir`.
pub fn source_files(dir: &Path) -> Result<Vec<String>, Failure> {
    let mut files = vec![];
    for top in ["src", "test"] {
        collect_gleam_files(&dir.join(top), &mut files)?;
    }
    let mut out: Vec<String> = files.iter().map(|f| f.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
    out.sort();
    Ok(out)
}

pub fn collect_gleam_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        // A symlinked folder isn't followed: deleting or writing through it could reach outside the project.
        if entry.file_type()?.is_dir() {
            collect_gleam_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "gleam") {
            out.push(path);
        }
    }
    Ok(())
}

/// Formats Gleam source, which doubles as the syntax check.
pub fn format(source: &str) -> Result<String, Failure> {
    // Formatting stdin needs no project folder; a fixed child folder keeps a
    // process-wide `Within` from moving the formatter's working directory.
    let mut child = Command::new("gleam")
        .args(["format", "--stdin"])
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(source.as_bytes())?;
    let out = child.wait_with_output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let msg = strip_ansi(&String::from_utf8_lossy(&out.stderr)).replace("<stdin>", "submitted text");
        fail(format!("Not saved: E0100 syntax error.\n\n{}", msg.trim()))
    }
}

/// Command output without build progress and compiler warnings (a warning
/// runs until the next error or until the program starts).
fn quiet(output: &str) -> String {
    let (mut out, mut warning) = (String::new(), false);
    for l in output.lines() {
        let t = l.trim();
        warning = (warning || l.starts_with("warning")) && !l.starts_with("error") && !t.starts_with("Running ");
        if !warning && !["Compiling ", "Compiled ", "Running ", "Downloading ", "Downloaded "].iter().any(|p| t.starts_with(p)) {
            out += &format!("{l}\n");
        }
    }
    out
}

/// Failed tests in `gleam test` output by `module.name` (Erlang prints
/// modules as `pilot@x_test`): whether wrong or unfinished, and the failure
/// block (`assert …`, `test:`, `code:`, `left:`, `right:`, `info:`).
pub fn failures(output: &str) -> BTreeMap<String, (String, String)> {
    let clean = strip_ansi(output);
    let mut out = BTreeMap::new();
    let mut block: Vec<&str> = vec![];
    for line in clean.lines().map(str::trim).chain([""]) {
        let start = ["todo ", "assert ", "let assert ", "panic "].iter().any(|p| line.starts_with(p));
        if (start || line.is_empty()) && !block.is_empty() {
            if let Some(name) = block.iter().find_map(|l| l.strip_prefix("test: ")) {
                let kind = if block[0].starts_with("todo ") { "unfinished" } else { "wrong" };
                out.insert(name.replace('@', "/"), (kind.into(), block.join("\n")));
            }
            block.clear();
        }
        if start || (!block.is_empty() && !line.is_empty()) {
            block.push(line);
        }
    }
    out
}

/// Rewrites `path/to/file.gleam:LINE` references in a line to symbol positions.
fn relocate(line: &str, files: &[FileMap]) -> String {
    let Some(end) = line.find(".gleam:") else {
        return line.to_string();
    };
    let start = line[..end].rfind(|c: char| c.is_whitespace()).map_or(0, |i| i + 1);
    let path = &line[start..end + 6];
    let rest = &line[end + 7..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let (Ok(n), Some((module, sym))) = (digits.parse::<usize>(), locate(files, path, digits.parse().unwrap_or(0))) else {
        return line.to_string();
    };
    let tail = &rest[digits.len()..];
    let tail = tail.strip_prefix(':').map_or(tail, |t| t.trim_start_matches(|c: char| c.is_ascii_digit()));
    let place = match sym {
        Some((q, first)) => format!("{q}, line {}", n - first + 1),
        None => format!("{module} imports"),
    };
    format!("{}{place}{tail}", &line[..start])
}

/// Splits compiler output into diagnostics located by symbol.
fn diagnostics(output: &str, files: &[FileMap]) -> Vec<Diag> {
    let mut blocks: Vec<Vec<&str>> = vec![];
    for line in output.lines() {
        let t = line.trim_start();
        if line.starts_with("error") || line.starts_with("warning") {
            blocks.push(vec![line]);
        } else if ["Compiling ", "Compiled ", "Checking ", "Running "].iter().any(|p| t.starts_with(p)) {
            continue;
        } else if let Some(b) = blocks.last_mut() {
            b.push(line);
        }
    }
    blocks
        .into_iter()
        .map(|b| {
            let error = b[0].starts_with("error");
            let mut module = None;
            let mut symbol = None;
            let mut offset = 0;
            let mut text = vec![];
            for line in &b {
                if let Some(loc) = line.trim_start().strip_prefix("┌─ ") {
                    let mut parts = loc.rsplitn(3, ':');
                    let (_col, row, path) = (parts.next(), parts.next(), parts.next());
                    if let (Some(path), Some(Ok(row))) = (path, row.map(|r| r.parse::<usize>()))
                        && let Some((m, sym)) = locate(files, path, row)
                    {
                        module = Some(m.to_string());
                        match sym {
                            Some((q, first)) => {
                                symbol = Some(q.to_string());
                                offset = first - 1;
                                text.push(format!("  at {q}, line {}", row - offset));
                            }
                            None => text.push(format!("  at {m} imports")),
                        }
                        continue;
                    }
                }
                // renumber the code excerpt to lines within the symbol
                let t = line.trim_start();
                let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
                if offset > 0
                    && !digits.is_empty()
                    && t[digits.len()..].starts_with(" │")
                    && let Ok(n) = digits.parse::<usize>()
                    && n > offset
                {
                    text.push(format!("{:>4}{}", n - offset, &t[digits.len()..]));
                    continue;
                }
                text.push(line.to_string());
            }
            Diag { error, module, symbol, text: text.join("\n").trim_end().to_string(), suggestions: vec![] }
        })
        .collect()
}

#[cfg(test)]
#[path = "gleam_tests.rs"]
mod tests;
