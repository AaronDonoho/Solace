//! Post-move repair: applies the fixes rustc names after a move, but only
//! when they leave nothing to choose.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Range;

use crate::diagnostics::{self, Diag, SuggestionSpan};
use crate::extract::rust::plan::{self, RustMove};
use crate::extract::rust::{imports, rewrite, visibility};
use crate::failure::Failure;
use crate::{Index, Store};
#[cfg(test)]
#[path = "repair_tests.rs"]
mod tests;

const ROUNDS: usize = 4;

/// What a repaired move changed, for history and the reply.
#[derive(Default)]
pub(super) struct Repair {
    pub(super) symbols: HashMap<String, String>,
    pub(super) promoted: BTreeSet<String>,
    pub(super) raised: BTreeSet<(String, &'static str)>,
    pub(super) imports: BTreeSet<(String, String)>,
    pub(super) visibility_only: HashSet<String>,
}

/// The warnings after a repaired move, and what the repair changed.
pub(super) fn finish(s: &Store, before: &HashMap<String, String>, m: &RustMove<'_>) -> Result<(Vec<Diag>, Repair), Failure> {
    let mut ix = s.index()?;
    let mut diags = s.check_all(&ix)?;
    let old = OldBindings::new(m.ix);
    let mut repair = Repair::default();
    for _ in 0..ROUNDS {
        if crate::rust_check::new_errors(before, &diags).is_empty() {
            break;
        }
        let changed = apply_round(s, &ix, m, &old, before, &diags, &mut repair)?;
        if !changed {
            break;
        }
        ix = s.index()?;
        diags = s.check_all(&ix)?;
    }
    let warnings = crate::rust_check::post_move_diags(before, diags)?;
    Ok((warnings, repair))
}

#[derive(Default)]
struct Round {
    headers: HashMap<String, String>,
    edits: HashMap<String, Vec<(Range<usize>, String)>>,
    promote: HashSet<String>,
    fields: HashMap<String, HashSet<String>>,
    inline_modules: HashMap<(String, String), BTreeSet<String>>,
    header_modules: HashMap<(String, String), BTreeSet<String>>,
    imports: BTreeSet<(String, String)>,
    raised: BTreeSet<(String, &'static str)>,
}

fn apply_round(
    s: &Store,
    ix: &Index,
    m: &RustMove<'_>,
    old: &OldBindings,
    before: &HashMap<String, String>,
    diags: &[Diag],
    repair: &mut Repair,
) -> Result<bool, Failure> {
    let files = s.materialize(ix)?;
    let ids = super::id_indexes(ix);
    let mut round = Round::default();
    for diag in crate::rust_check::new_errors(before, diags) {
        let imported = apply_import(s, ix, &files, &mut round, diag, old, m)?;
        if !imported {
            apply_visibility(ix, &ids, &files, &mut round, diag);
        }
    }
    round.commit(s, ix, &ids, repair)
}

impl Round {
    fn commit(mut self, s: &Store, ix: &Index, ids: &HashMap<&str, usize>, repair: &mut Repair) -> Result<bool, Failure> {
        let mut changed = false;
        for ((module, name), callers) in &self.header_modules {
            let header = self.headers.get(module).cloned().unwrap_or(s.header(module)?);
            if let Some((text, visibility)) = visibility::raise_module(&header, name, module, callers) {
                self.raised.insert((module_name(module, name), visibility));
                self.headers.insert(module.clone(), text);
            }
        }
        for (module, header) in &self.headers {
            if &s.header(module)? != header {
                s.set_header(module, header)?;
                changed = true;
            }
        }
        for (id, edits) in std::mem::take(&mut self.edits) {
            let Some(&i) = ids.get(id.as_str()) else { continue };
            let symbol = &ix.syms[i];
            let text = rewrite::splice(&symbol.text, edits);
            let path_changed = text != symbol.text;
            let before_visibility = text.clone();
            let text = self.raise_symbol(symbol, text, repair);
            if text != before_visibility && !path_changed {
                repair.visibility_only.insert(id.clone());
            }
            if text != symbol.text {
                s.set_symbol_text(&symbol.id, &symbol.name, &text)?;
                repair.symbols.insert(id, text);
                changed = true;
            }
        }
        repair.imports.extend(self.imports);
        repair.raised.extend(self.raised);
        Ok(changed)
    }

    /// `text` with the visibility `symbol` needs for its fields, inline
    /// modules and itself.
    fn raise_symbol(&mut self, symbol: &crate::Sym, mut text: String, repair: &mut Repair) -> String {
        if let Some(fields) = self.fields.get(&symbol.id) {
            let before = text.clone();
            text = plan::promote_fields(&text, fields);
            if text != before {
                for field in fields {
                    repair.promoted.insert(format!("{}.{}", symbol.qname(), field));
                }
            }
        }
        for ((_, name), callers) in self.inline_modules.iter().filter(|((id, _), _)| id == &symbol.id) {
            if let Some((raised, visibility)) = visibility::raise_module(&text, name, &symbol.module, callers) {
                self.raised.insert((symbol.qname(), visibility));
                text = raised;
            }
        }
        if self.promote.contains(&symbol.id) {
            let raised = plan::make_visible(&text);
            if raised != text {
                repair.promoted.insert(symbol.qname());
                text = raised;
            }
        }
        text
    }
}

fn module_name(module: &str, name: &str) -> String {
    if module == "crate" { name.to_string() } else { format!("{module}::{name}") }
}

#[derive(Clone)]
pub(super) struct Candidate {
    pub(super) line: String,
    pub(super) path: Vec<String>,
    pub(super) trait_import: bool,
}

fn apply_import(
    s: &Store,
    ix: &Index,
    files: &[diagnostics::FileMap],
    round: &mut Round,
    diag: &Diag,
    old: &OldBindings,
    m: &RustMove<'_>,
) -> Result<bool, Failure> {
    let candidates = import_candidates(diag);
    if candidates.is_empty() {
        return Ok(false);
    }
    let module = diag.module.as_deref().unwrap_or("");
    if module.is_empty() {
        return Ok(false);
    }
    let Some(candidate) = choose(&candidates, old, &old_module(diag, m)) else { return Ok(false) };
    let mut changed = false;
    let header = round.headers.get(module).cloned().unwrap_or(s.header(module)?);
    let local = candidate.path.last().map(String::as_str).unwrap_or("");
    let keys = imports::import_keys(ix, &header, module)?;
    let already = keys.iter().any(|(_, path, glob)| !*glob && path == &candidate.path);
    if !already {
        let taken = super::target_names(ix, &header, module, |_, symbol| symbol.module == module)?;
        let line = if taken.contains(local) {
            if !candidate.trait_import {
                return Ok(false);
            }
            format!("use {} as _;", candidate.path.join("::"))
        } else {
            candidate.line.clone()
        };
        round.imports.insert((module.to_string(), line.clone()));
        round.headers.insert(module.to_string(), imports::add_import_line(&header, &line));
        changed = true;
    }
    for span in diag.suggestions.iter().filter(|s| s.message.contains("refer to it directly")).flat_map(|s| &s.spans) {
        let Some((id, range)) = symbol_span(ix, files, span) else { continue };
        round.edits.entry(id).or_default().push((range, span.replacement.clone().unwrap_or_default()));
        changed = true;
    }
    Ok(changed)
}

pub(super) fn import_candidates(diag: &Diag) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = vec![];
    for suggestion in diag.suggestions.iter().filter(|s| s.message.to_lowercase().contains("import")) {
        for span in &suggestion.spans {
            let Some(replacement) = &span.replacement else { continue };
            let Some(candidate) = candidate(replacement, &suggestion.message) else { continue };
            if out.iter().any(|c| c.line == candidate.line && c.path == candidate.path) {
                continue;
            }
            out.push(candidate);
        }
    }
    out
}

pub(super) fn candidate(replacement: &str, message: &str) -> Option<Candidate> {
    let line = replacement.trim();
    if !(line.starts_with("use ") || line.starts_with("pub use ")) {
        return None;
    }
    let sites = crate::language::rust_refs::import_sites_in(line).ok()?;
    let [site] = sites.as_slice() else { return None };
    if site.glob {
        return None;
    }
    Some(Candidate { line: line.to_string(), path: site.path.clone(), trait_import: message.contains("trait") })
}

fn choose<'a>(candidates: &'a [Candidate], old: &OldBindings, old_module: &str) -> Option<&'a Candidate> {
    if candidates.len() == 1 && (in_crate(&candidates[0].path) || candidates[0].trait_import) {
        return Some(&candidates[0]);
    }
    let name = candidates.first()?.path.last()?;
    let old_paths = old.paths(old_module, name);
    let matching: Vec<&Candidate> = candidates.iter().filter(|c| old_paths.iter().any(|path| path == &c.path)).collect();
    let [candidate] = matching.as_slice() else { return None };
    Some(candidate)
}

fn in_crate(path: &[String]) -> bool {
    matches!(path.first().map(String::as_str), Some("crate" | "self" | "super"))
}

fn old_module(diag: &Diag, m: &RustMove<'_>) -> String {
    let Some(symbol) = diag.symbol.as_deref() else { return diag.module.clone().unwrap_or_default() };
    if let Some(found) = m.ix.syms.iter().find(|s| s.qname() == symbol) {
        return found.module.clone();
    }
    let (module, name) = symbol.rsplit_once("::").unwrap_or(("", symbol));
    m.ix.syms
        .iter()
        .enumerate()
        .find(|(i, s)| m.moved[*i] && s.name == name && m.newmod[*i] == module)
        .map_or_else(|| diag.module.clone().unwrap_or_default(), |(_, s)| s.module.clone())
}

struct OldBindings {
    by_module: HashMap<(String, String), Vec<Vec<String>>>,
}

impl OldBindings {
    fn new(ix: &Index) -> Self {
        let mut by_module = HashMap::new();
        for module in &ix.modules {
            record_imports(&mut by_module, &module.path, &module.header);
        }
        for symbol in &ix.syms {
            if symbol.kind_text == "use" {
                record_imports(&mut by_module, &symbol.module, &symbol.text);
            } else if symbol.parent.is_none() {
                record_symbol(&mut by_module, &symbol.module, &symbol.name);
            }
        }
        Self { by_module }
    }

    fn paths(&self, module: &str, name: &str) -> Vec<Vec<String>> {
        self.by_module.get(&(module.to_string(), name.to_string())).cloned().unwrap_or_default()
    }
}

fn record_imports(by_module: &mut HashMap<(String, String), Vec<Vec<String>>>, module: &str, text: &str) {
    let Ok(sites) = crate::language::rust_refs::import_sites_in(text) else { return };
    for site in sites {
        if site.glob {
            continue;
        }
        by_module.entry((module.to_string(), site.local)).or_default().push(site.path);
    }
}

fn record_symbol(by_module: &mut HashMap<(String, String), Vec<Vec<String>>>, module: &str, name: &str) {
    let mut path = vec!["crate".to_string()];
    if module != "crate" {
        path.extend(module.split("::").map(str::to_string));
    }
    path.push(name.to_string());
    by_module.entry((module.to_string(), name.to_string())).or_default().push(path);
}

fn apply_visibility(ix: &Index, ids: &HashMap<&str, usize>, files: &[diagnostics::FileMap], round: &mut Round, diag: &Diag) {
    let Some((code, item)) = private_error(diag) else { return };
    let Some(span) = diag
        .suggestions
        .iter()
        .filter(|s| s.message.contains("defined here"))
        .flat_map(|s| &s.spans)
        .find(|span| span.replacement.is_none())
    else {
        return;
    };
    let Some((module, symbol)) = diagnostics::locate(files, &span.file_name, span.line_start) else { return };
    let caller = diag.module.clone().unwrap_or_default();
    if let Some((qname, _)) = symbol {
        let Some(i) = ix.syms.iter().position(|s| s.qname() == qname) else { return };
        let id = ix.syms[i].id.clone();
        round.edits.entry(id.clone()).or_default();
        if code == "E0616" {
            round.fields.entry(id).or_default().insert(item);
        } else if ix.syms[i].kind_text == "mod" {
            round.inline_modules.entry((id, item)).or_default().insert(caller);
        } else if plan::can_promote(ix, ids, i) {
            round.promote.insert(id);
        }
        return;
    }
    if code == "E0603" {
        round.header_modules.entry((module.to_string(), item)).or_default().insert(caller);
    }
}

fn private_error(diag: &Diag) -> Option<(&str, String)> {
    let title = diag.text.lines().next()?;
    let code = title.strip_prefix("error[")?.split_once(']')?.0;
    if !matches!(code, "E0603" | "E0624" | "E0616") {
        return None;
    }
    let item = title.split('`').nth(1)?.to_string();
    Some((code, item))
}

fn symbol_span(ix: &Index, files: &[diagnostics::FileMap], span: &SuggestionSpan) -> Option<(String, Range<usize>)> {
    let (_, symbol) = diagnostics::locate(files, &span.file_name, span.line_start)?;
    let (qname, first) = symbol?;
    let i = ix.syms.iter().position(|s| s.qname() == qname)?;
    let range = local_range(&ix.syms[i].text, first, span)?;
    Some((ix.syms[i].id.clone(), range))
}

fn local_range(text: &str, first_line: usize, span: &SuggestionSpan) -> Option<Range<usize>> {
    if span.line_start < first_line || span.line_end < span.line_start {
        return None;
    }
    let start = offset(text, span.line_start - first_line, span.column_start)?;
    let end = offset(text, span.line_end - first_line, span.column_end)?;
    (end >= start).then_some(start..end)
}

fn offset(text: &str, line: usize, column: usize) -> Option<usize> {
    let mut start = 0;
    for _ in 0..line {
        start += text[start..].find('\n')? + 1;
    }
    let end = text[start..].find('\n').map_or(text.len(), |at| start + at);
    let byte = text[start..end].char_indices().nth(column.saturating_sub(1)).map_or(end, |(at, _)| start + at);
    Some(byte)
}