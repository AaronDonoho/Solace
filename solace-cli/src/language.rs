//! A project's programming language, behind one interface: how its source
//! splits into symbols, what a symbol names, and how its tools format, check
//! and test the code. The store, ids, history, drafts, undo and patches don't
//! depend on it. Gleam is the language of a `gleam.toml` project
//! (`language/gleam.rs`); a Cargo package is read by `language/rust.rs` and
//! run by `language/cargo.rs` (D-158).

pub mod cargo;
pub mod gleam;
pub mod rust;
pub mod rust_refs;
mod paths;

use crate::diagnostics::FileMap;
use crate::failure::Failure;
use crate::diagnostics::Diag;
use gleam::Gleam;
use std::collections::BTreeMap;
use std::path::Path;
use crate::board::setup::language_enabled;
use crate::board::setup::language_reason;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Fn,
    Type,
    Const,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Fn => "fn",
            Kind::Type => "type",
            Kind::Const => "const",
        }
    }
    pub fn parse(s: &str) -> Kind {
        match s {
            "type" => Kind::Type,
            "const" => Kind::Const,
            _ => Kind::Fn,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Item {
    pub kind: Kind,
    pub name: String,
    /// Includes the doc comment and any comments directly above it.
    pub text: String,
}

pub struct Module {
    /// Module doc comment and imports: everything before the first item.
    pub header: String,
    pub items: Vec<Item>,
}

pub struct Import {
    pub module: String,
    pub alias: String,
    /// (name in the imported module, local name)
    pub unqualified: Vec<(String, String)>,
}

pub enum Ref {
    /// `alias.name`
    Qualified(String, String),
    Bare(String),
}

/// Names defined again after their first definition, once per repeat.
pub fn repeated(items: &[Item]) -> Vec<&str> {
    let mut seen = std::collections::HashSet::new();
    items.iter().filter(|i| !seen.insert(&i.name)).map(|i| i.name.as_str()).collect()
}

/// The parts of a bracketed list, split at its top-level commas: `s` starts
/// just after the opening bracket and ends at the matching close. Each part
/// comes with its byte offset in `s`; blank parts (trailing commas) are left out.
pub fn split_args(s: &str) -> Vec<(usize, String)> {
    let (mut depth, mut parts, mut start) = (0i32, vec![], 0);
    let mut end = s.len();
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => {
                end = i;
                break;
            }
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push((start, s[start..i].to_string()));
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push((start, s[start..end].to_string()));
    parts.into_iter().filter(|(_, p)| !p.trim().is_empty()).collect()
}

/// Where a function's body opens in `decl` (text from its `fn`): the first `{`
/// outside parentheses and brackets.
pub fn body_start(decl: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in decl.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '{' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

pub trait Language: Sync {
    /// The file at a project's root that makes it a project of this language.
    fn manifest(&self) -> &'static str;
    /// The program that builds, checks and tests a project.
    fn tool(&self) -> &'static str;

    // --- reading code
    fn split(&self, source: &str) -> Module;
    /// Kind and name when `line` starts a top-level item.
    fn declaration(&self, line: &str) -> Option<(Kind, String)>;
    /// Comment or attribute lines that belong to the item below them.
    fn is_leading(&self, line: &str) -> bool;
    fn imports(&self, header: &str) -> Vec<Import>;
    fn references(&self, code: &str) -> Vec<Ref>;
    fn references_at(&self, code: &str) -> Vec<(Ref, usize)>;
    /// `references`, each with its byte span in `code`.
    fn references_spans(&self, code: &str) -> Vec<(Ref, usize, usize)>;
    /// The names bound locally anywhere in `code`.
    fn bound_names(&self, code: &str) -> std::collections::HashSet<String>;
    fn constructors(&self, type_text: &str) -> Vec<String>;
    fn record_fields(&self, type_text: &str) -> Vec<(String, Vec<Option<String>>)>;
    fn field_reads(&self, code: &str) -> Vec<(String, usize)>;
    fn pattern_names(&self, pattern: &str) -> Vec<String>;
    /// Source with strings and comments blanked, offsets kept.
    fn strip_comments(&self, source: &str) -> String;
    fn signature(&self, item: &Item) -> String;
    fn head(&self, item: &Item) -> String;

    // --- reading a project
    /// The source files of the project in `dir`, as sorted paths from `dir`.
    fn source_files(&self, dir: &Path) -> Result<Vec<String>, Failure>;
    fn valid_module_path(&self, m: &str) -> bool;

    // --- the language's tools
    /// Formatted source; the syntax check too.
    fn format(&self, source: &str) -> Result<String, Failure>;
    /// The tool's check or test output as diagnostics located by symbol.
    fn diagnostics(&self, output: &str, files: &[FileMap]) -> Vec<Diag>;
    /// A line of tool output with its file positions as symbol positions.
    fn relocate(&self, line: &str, files: &[FileMap]) -> String;
    /// Failed tests in test output by `module.name`: whether wrong or
    /// unfinished, and the failure's text.
    fn failures(&self, output: &str) -> BTreeMap<String, (String, String)>;
    /// Tool output without build progress and warnings.
    fn quiet(&self, output: &str) -> String;
}

const LANGUAGES: &[&dyn Language] = &[&Gleam];

/// Whether the project at `root` is a Cargo package: `Cargo.toml` and no
/// `gleam.toml` (D-158), the rule `import` uses. Its code is held as symbols,
/// but its tools are `cargo`'s, not a `Language`'s.
pub fn is_rust(root: &Path) -> bool {
    root.join("Cargo.toml").exists() && !root.join("gleam.toml").exists()
}

/// The language of the project at `root`, from its files; `None` for a
/// project of plain files (D-60).
pub fn detect(root: &Path) -> Option<&'static dyn Language> {
    LANGUAGES.iter().copied().find(|l| root.join(l.manifest()).exists())
}

/// The language that reads a store of `lang`: a project of plain files holds
/// no symbols, so any language reads its empty store alike.
pub fn reading(lang: Option<&'static dyn Language>) -> &'static dyn Language {
    lang.unwrap_or(&Gleam)
}

/// `reading` the project at `root`.
pub fn of(root: &Path) -> &'static dyn Language {
    reading(detect(root))
}

#[cfg(test)]
#[path = "language_tests.rs"]
mod tests;

pub const GLEAM: &str = "gleam";

pub const RUST: &str = "rust";

/// The language's name as Settings shows it.
pub fn label(id: &str) -> &'static str {
    match id {
        GLEAM => "Gleam",
        RUST => "Rust",
        _ => "",
    }
}

/// Whether the language's toolchain is on `PATH`, regardless of whether the
/// language is turned on in settings.
pub fn installed(id: &str) -> bool {
    match id {
        GLEAM => on_path("gleam"),
        RUST => on_path("cargo"),
        _ => false,
    }
}

pub fn missing(id: &str) -> &'static str {
    match id {
        GLEAM => "Needs the Gleam toolchain (`gleam`)",
        RUST => "Needs the Rust toolchain (`cargo`)",
        _ => "Needs its toolchain",
    }
}

pub fn off(id: &str) -> &'static str {
    match id {
        GLEAM => "Gleam is turned off in Settings",
        RUST => "Rust is turned off in Settings",
        _ => "This language is turned off in Settings",
    }
}

fn on_path(program: &str) -> bool {
    crate::language::paths::search_path(program).is_some()
}

/// The language of `root`, as the system settings name it.
pub fn id(root: &Path) -> Option<&'static str> {
    if is_rust(root) {
        Some(RUST)
    } else if root.join("gleam.toml").exists() {
        Some(GLEAM)
    } else {
        None
    }
}

/// Why a language's projects can't use its tools, if they can't.
pub fn reason(id: &str) -> Result<Option<String>, Failure> {
    language_reason(id)
}

/// Whether the system settings leave the language on. An unreadable or
/// invalid settings file is an error, never a silent yes.
pub fn enabled(id: &str) -> Result<bool, Failure> {
    language_enabled(id)
}
