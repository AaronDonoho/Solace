//! Rust references (D-170): the names an item's text mentions, resolved
//! against its module's `use` lines and the crate's item names, so a Rust
//! store carries the same kind of reference graph Gleam's does.
//!
//! Names are read from the text's tokens, not with a `syn` visitor: a call
//! inside `assert!(…)` or `format!(…)` is a token in a macro's arguments, so a
//! visitor over the syntax tree would miss the tests it sits in. Resolution
//! separates exact uses from possible ones: a method name, a macro, a field,
//! or an unresolved path may name an item, so it stays possible. A local
//! binding, a module's own item, or a named import that hides a glob is not a
//! use.

use crate::{Failure, ModuleRow, Sym};
use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};
use std::collections::{HashMap, HashSet};
use syn::Meta;

/// A Rust store's reference graph: what each symbol uses, its reverse, each
/// reference's line, which references are only possible, and which symbols are
/// test code.
pub struct Graph {
    pub uses: Vec<Vec<usize>>,
    pub used_by: Vec<Vec<usize>>,
    pub at: Vec<Vec<(usize, usize)>>,
    /// The references that need type inference: (target, line), as in `at`.
    pub possible: Vec<Vec<(usize, usize)>>,
    pub test: Vec<bool>,
    /// The modules that hold tests (D-170).
    pub test_modules: HashSet<String>,
}

/// Builds the graph of `syms`, named as `modules` names them.
pub fn graph(syms: &[Sym], modules: &[ModuleRow]) -> Graph {
    let refs = Refs::new(syms, modules);
    let mut at: Vec<Vec<(usize, usize)>> = vec![vec![]; syms.len()];
    let mut possible: Vec<Vec<(usize, usize)>> = vec![vec![]; syms.len()];
    let mut uses: Vec<Vec<usize>> = vec![vec![]; syms.len()];
    for i in 0..syms.len() {
        // A `use` statement's own text names what it imports: that is an
        // import, not a use of the item.
        if syms[i].kind_text == "use" {
            continue;
        }
        let m = syms[i].module.as_str();
        let text = &syms[i].text;
        let mentions = scanned(text);
        let context = ScopeContext { refs: &refs, module: m, symbol: i, target: None };
        let scopes = scan_scopes(text, refs.container_name(i).is_some(), &context);
        for mention in mentions {
            if scopes.imports.iter().any(|import| import.start <= mention.range.start && mention.range.end <= import.end) {
                continue;
            }
            let (exact, maybe) = classify(&refs, m, i, &mention, &scopes);
            for (targets, is_possible) in [(exact, false), (maybe, true)] {
                for t in targets {
                    if t == i {
                        continue;
                    }
                    if !at[i].contains(&(t, mention.line)) {
                        at[i].push((t, mention.line));
                    }
                    if is_possible && !possible[i].contains(&(t, mention.line)) {
                        possible[i].push((t, mention.line));
                    }
                }
            }
        }
        for &(t, _) in &at[i] {
            if !uses[i].contains(&t) {
                uses[i].push(t);
            }
        }
    }
    let mut used_by = vec![vec![]; syms.len()];
    for (j, us) in uses.iter().enumerate() {
        for &u in us {
            used_by[u].push(j);
        }
    }
    let test_modules = test_modules(syms, modules);
    Graph { uses, used_by, at, possible, test: test_flags(syms, &test_modules), test_modules }
}

/// Whether `text` carries a test attribute: `#[test]`, or `#[<path>::test]`
/// such as `#[tokio::test]` (D-170).
pub fn has_test_attr(text: &str) -> bool {
    test_attr_in(&outer_attrs(text))
}

/// `text`'s own outer attributes, one `#[…]` group each: the attributes above
/// its signature, past the blank lines and doc comments, several on one line or
/// spanning lines. A mention inside the body — a doc comment, a string — is not
/// one, and a bracket inside a literal doesn't join groups (D-170).
fn outer_attrs(text: &str) -> Vec<String> {
    attrs_and_rest(text).0.into_iter().map(str::to_string).collect()
}

/// `text`'s leading `#[…]` groups and the text after them: the same scan as
/// `outer_attrs`, for a reader that needs what follows the attributes (D-170).
pub(crate) fn attrs_and_rest(text: &str) -> (Vec<&str>, &str) {
    let mut out = vec![];
    let mut rest = text;
    loop {
        rest = skip_trivia(rest);
        let Some(len) = attr_len(rest) else { return (out, rest) };
        out.push(&rest[..len]);
        rest = &rest[len..];
    }
}

/// `s` past the whitespace and comments in front of an item.
fn skip_trivia(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        if let Some(r) = s.strip_prefix("//") {
            s = r.split_once('\n').map_or("", |(_, rest)| rest);
            continue;
        }
        if let Some(r) = s.strip_prefix("/*") {
            s = &r[block_comment_len(r)..];
            continue;
        }
        return s;
    }
}

/// The byte length of the `#[…]` group at the front of `s`: its closing `]`,
/// found past string, raw-string and char literals. `None` when `s` doesn't
/// start with `#[` or the group doesn't close.
fn attr_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if !s.starts_with("#[") {
        return None;
    }
    let (mut depth, mut i) = (0i32, 0);
    while i < b.len() {
        match b[i] {
            b'"' => i = end_of_string(b, i),
            b'\'' => i = end_of_char(b, i),
            b'r' | b'b' | b'c' if raw_open(b, i) => i = end_of_raw(b, i),
            b'[' => {
                depth += 1;
                i += 1;
            }
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

/// Whether a raw string opens at `b[i]` (`r"`, `r#"`, `br#"`, `cr#"`).
fn raw_open(b: &[u8], i: usize) -> bool {
    let mut j = match b[i] {
        b'r' => i + 1,
        b'b' | b'c' if b.get(i + 1) == Some(&b'r') => i + 2,
        _ => return false,
    };
    while b.get(j) == Some(&b'#') {
        j += 1;
    }
    b.get(j) == Some(&b'"')
}

/// `i`'s `"` past the string it opens.
fn end_of_string(b: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'"' => return j + 1,
            _ => j += 1,
        }
    }
    b.len()
}

/// `i`'s raw string past its closing `"#…`; `b.len()` when it doesn't close.
fn end_of_raw(b: &[u8], i: usize) -> usize {
    let mut j = if b[i] == b'r' { i + 1 } else { i + 2 };
    let mut hashes = 0;
    while b.get(j) == Some(&b'#') {
        hashes += 1;
        j += 1;
    }
    j += 1;
    while j < b.len() {
        if b[j] == b'"' {
            let (mut k, mut n) = (j + 1, 0);
            while n < hashes && b.get(k) == Some(&b'#') {
                k += 1;
                n += 1;
            }
            if n == hashes {
                return k;
            }
        }
        j += 1;
    }
    b.len()
}

/// `i`'s `'` past the char literal it opens; a lifetime's `'` is one byte.
fn end_of_char(b: &[u8], i: usize) -> usize {
    if b.get(i + 1) == Some(&b'\\') {
        let mut j = i + 2;
        while j < b.len() && b[j] != b'\'' {
            j += 1;
        }
        return j + 1;
    }
    if b.get(i + 2) == Some(&b'\'') { i + 3 } else { i + 1 }
}

/// Whether any of `attrs` names a test.
fn test_attr_in(attrs: &[String]) -> bool {
    attrs.iter().any(|a| test_attr(a))
}

/// Whether `a` is a test attribute: `#[test]`, `#[rstest]`, or `#[<path>::test]`
/// such as `#[tokio::test]`.
fn test_attr(a: &str) -> bool {
    let Some(Meta::Path(p)) = meta_of(a) else { return false };
    p.is_ident("rstest") || p.segments.last().is_some_and(|s| s.ident == "test")
}

/// A `#[…]` attribute's body as a `syn` meta.
fn meta_of(a: &str) -> Option<Meta> {
    syn::parse_str::<Meta>(a.strip_prefix("#[")?.strip_suffix(']')?).ok()
}

/// Whether any of `attrs` makes the item test-only code: `#[cfg(test)]` or
/// `#[cfg(all(test, …))]`. `any(test, …)` and `not(test)` hold outside tests
/// too, so those items stay ordinary code.
fn cfg_test_in(attrs: &[String]) -> bool {
    attrs.iter().any(|a| cfg_test_attr(a))
}

/// Whether `a` is `#[cfg(…)]` whose condition needs test: `cfg(test)`, or
/// `cfg(all(…))` with a bare `test` among its own top-level arguments.
fn cfg_test_attr(a: &str) -> bool {
    let Some(Meta::List(l)) = meta_of(a) else { return false };
    l.path.is_ident("cfg") && nested(&l).iter().any(cfg_test)
}

/// Whether a `cfg` condition needs test: `test`, or `all(…)` with one among its
/// own arguments. `any(test, …)` and `not(test)` hold outside tests too, so
/// those items stay ordinary code.
fn cfg_test(m: &Meta) -> bool {
    match m {
        Meta::Path(p) => p.is_ident("test"),
        Meta::List(l) if l.path.is_ident("all") => nested(l).iter().any(cfg_test),
        _ => false,
    }
}

/// A meta list's own arguments, split at its commas.
fn nested(l: &syn::MetaList) -> Vec<Meta> {
    use syn::parse::Parser;
    syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated
        .parse2(l.tokens.clone())
        .map(|p| p.into_iter().collect())
        .unwrap_or_default()
}

/// A name an item's text mentions.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Mention {
    /// `a::b::c`, and a bare name as a one-segment path.
    Path(Vec<String>),
    /// A method call `.f(`.
    Method(String),
    /// A field access `.f` that is not a call.
    Field(String),
    /// A macro call `name!`.
    Macro(String),
    /// A name a format string captures, as in `format!("{name}")`.
    Format(String),
}

fn scan(stream: &TokenStream, text: &str, starts: &[usize], out: &mut Vec<Spanned>) {
    let trees: Vec<TokenTree> = stream.clone().into_iter().collect();
    let mut i = 0;
    let mut after_dot = false;
    while i < trees.len() {
        match &trees[i] {
            // A macro's arguments are a group: their calls are mentions too.
            TokenTree::Group(g) => {
                scan(&g.stream(), text, starts, out);
                after_dot = false;
                i += 1;
            }
            TokenTree::Punct(p) => {
                after_dot = p.as_char() == '.' && p.spacing() != Spacing::Joint && !previous_punct(&trees, i, '.');
                i += 1;
            }
            TokenTree::Literal(_) => {
                after_dot = false;
                i += 1;
            }
            TokenTree::Ident(id) => {
                if generic_binding(&trees, i) {
                    i += 1;
                    after_dot = false;
                    continue;
                }
                let line = id.span().start().line.saturating_sub(1);
                let start = byte_at(text, starts, id.span().start());
                let mut end = byte_at(text, starts, id.span().end());
                let dot = after_dot;
                let mut segs = vec![id.to_string()];
                let mut j = i + 1;
                while j + 2 < trees.len() && is_path_sep(&trees[j], &trees[j + 1]) {
                    if let TokenTree::Ident(n) = &trees[j + 2] {
                        segs.push(n.to_string());
                        end = byte_at(text, starts, n.span().end());
                        j += 3;
                    } else {
                        break;
                    }
                }
                let call = matches!(trees.get(j), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis);
                let macro_call = segs.len() == 1
                    && matches!(trees.get(j), Some(TokenTree::Punct(p)) if p.as_char() == '!' && p.spacing() != Spacing::Joint);
                let range = start..end;
                if macro_call {
                    let name = segs[0].clone();
                    out.push(Spanned { mention: Mention::Macro(name.clone()), line, range });
                    j += 1;
                    if let Some(index) = format_arg_index(&name)
                        && let Some(TokenTree::Group(g)) = trees.get(j)
                        && let Some(arg) = group_arg(g, index)
                    {
                        let named = named_format_args(g, index);
                        format_captures(&arg, text, starts, &named, out);
                    }
                } else if segs.len() > 1 {
                    out.push(Spanned { mention: Mention::Path(segs.clone()), line, range });
                } else if dot && call {
                    out.push(Spanned { mention: Mention::Method(segs[0].clone()), line, range });
                } else if dot {
                    out.push(Spanned { mention: Mention::Field(segs[0].clone()), line, range });
                } else {
                    out.push(Spanned { mention: Mention::Path(segs.clone()), line, range });
                }
                i = j;
                after_dot = false;
            }
        }
    }
}

fn is_path_sep(a: &TokenTree, b: &TokenTree) -> bool {
    matches!(a, TokenTree::Punct(p) if p.as_char() == ':') && matches!(b, TokenTree::Punct(p) if p.as_char() == ':')
}

fn leaf(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

/// The type a name mentions, for matching an `impl` to its type: `Store` for
/// `Store`, `Store<T>` and `<Store as Trait>`.
fn type_key(expr: &str) -> String {
    let e = expr.trim().trim_start_matches('<');
    let e = e.split(" as ").next().unwrap_or(e);
    let e = e.split('<').next().unwrap_or(e);
    leaf(e.trim()).to_string()
}

/// Whether a symbol names a type: `Kind` keeps only Gleam's three, so a Rust
/// store's kind is read from `kind_text` (D-158).
fn is_type(s: &Sym) -> bool {
    matches!(s.kind_text.as_str(), "struct" | "enum" | "union" | "type" | "trait")
}

/// One `use` line's import: the path to the imported item (`[]` for a glob's
/// module), the name it binds, and whether it imports a whole module.
struct Import {
    path: Vec<String>,
    local: String,
    glob: bool,
}

/// The `use` statements of a module header, which are not stored as symbols.
fn header_uses(header: &str) -> Vec<Import> {
    let mut out = vec![];
    let mut buf: Option<String> = None;
    for line in header.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with("//") || t.starts_with("/*") {
            continue;
        }
        match buf.as_mut() {
            Some(b) => {
                b.push(' ');
                b.push_str(t);
            }
            None => {
                let Some(at) = t.find("use ") else { continue };
                buf = Some(t[at + "use ".len()..].to_string());
            }
        }
        if let Some(i) = buf.as_ref().and_then(|b| b.find(';')) {
            let tree = buf.take().unwrap()[..i].to_string();
            out.extend(parse_use(&tree));
        }
    }
    out
}

fn parse_use(tree: &str) -> Vec<Import> {
    let mut out = vec![];
    use_entries(vec![], tree, &mut out);
    out
}

fn use_entries(prefix: Vec<String>, tree: &str, out: &mut Vec<Import>) {
    let tree = tree.trim();
    if tree.is_empty() {
        return;
    }
    if let Some(open) = tree.find('{') {
        let before = tree[..open].trim().trim_end_matches(':').trim();
        let mut base = prefix.clone();
        if !before.is_empty() {
            base.extend(segments(before));
        }
        let close = tree.rfind('}').unwrap_or(tree.len());
        for part in split_commas(&tree[open + 1..close]) {
            use_entries(base.clone(), &part, out);
        }
        return;
    }
    let (path_text, alias) = match tree.rsplit_once(" as ") {
        Some((p, a)) => (p.trim(), Some(a.trim().to_string())),
        None => (tree, None),
    };
    if path_text == "self" {
        if let Some(local) = prefix.last().cloned() {
            out.push(Import { path: prefix, local, glob: false });
        }
        return;
    }
    let mut segs = prefix;
    segs.extend(segments(path_text));
    if segs.is_empty() {
        return;
    }
    if segs.last().map(String::as_str) == Some("*") {
        segs.pop();
        out.push(Import { path: segs, local: "*".into(), glob: true });
        return;
    }
    let local = alias.unwrap_or_else(|| segs.last().cloned().unwrap_or_default());
    out.push(Import { path: segs, local, glob: false });
}

fn segments(s: &str) -> Vec<String> {
    s.trim().trim_start_matches("::").split("::").map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

fn split_commas(s: &str) -> Vec<String> {
    let (mut depth, mut out, mut start) = (0i32, vec![], 0);
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(s[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(s[start..].to_string());
    out.into_iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

/// A module's names, its imports, and its types' trait impls, for resolving.
struct Refs<'a> {
    syms: &'a [Sym],
    by_id: HashMap<&'a str, usize>,
    children: HashMap<&'a str, Vec<usize>>,
    macros: HashMap<&'a str, Vec<usize>>,
    module_paths: HashSet<&'a str>,
    inline_modules: HashMap<String, usize>,
    names: HashMap<&'a str, HashMap<&'a str, Vec<usize>>>,
    heads: HashMap<&'a str, HashMap<&'a str, Vec<usize>>>,
    imports: HashMap<&'a str, Vec<Import>>,
    container_imports: HashMap<&'a str, Vec<Import>>,
    leaves: HashMap<&'a str, Vec<usize>>,
    impls: HashMap<String, Vec<usize>>,
}

impl<'a> Refs<'a> {
    fn new(syms: &'a [Sym], modules: &'a [ModuleRow]) -> Refs<'a> {
        let mut r = Refs {
            syms,
            by_id: HashMap::new(),
            children: HashMap::new(),
            macros: HashMap::new(),
            module_paths: modules.iter().map(|m| m.path.as_str()).collect(),
            inline_modules: HashMap::new(),
            names: HashMap::new(),
            heads: HashMap::new(),
            imports: HashMap::new(),
            container_imports: HashMap::new(),
            leaves: HashMap::new(),
            impls: HashMap::new(),
        };
        for (i, s) in syms.iter().enumerate() {
            r.by_id.insert(s.id.as_str(), i);
            if let Some(p) = s.parent.as_deref() {
                r.children.entry(p).or_default().push(i);
            }
            if s.kind_text == "macro_rules!" || s.kind_text == "macro" {
                r.macros.entry(leaf(&s.name)).or_default().push(i);
            }
            if s.kind_text == "mod" {
                let path = inline_path(&s.module, &s.name);
                let file_path = if s.module == "crate" { s.name.as_str() } else { path.as_str() };
                // A `mod x;` declaration names a file module; only `mod x { … }` is inline.
                if !r.module_paths.contains(file_path) {
                    r.inline_modules.insert(path, i);
                }
            }
            r.names.entry(s.module.as_str()).or_default().entry(s.name.as_str()).or_default().push(i);
            if !s.name.contains("::") {
                r.heads.entry(s.module.as_str()).or_default().entry(s.name.as_str()).or_default().push(i);
            }
            r.leaves.entry(leaf(&s.name)).or_default().push(i);
            if s.kind_text == "impl"
                && let Some(ty) = s.name.strip_prefix('<').and_then(|n| n.split(" as ").next())
            {
                r.impls.entry(type_key(ty)).or_default().push(i);
            }
        }
        r.imports = modules.iter().map(|m| (m.path.as_str(), header_uses(&m.header))).collect();
        for s in syms.iter().filter(|s| s.kind_text == "use" || s.kind_text == "mod") {
            if s.kind_text == "use" {
                let imports = parse_use(&s.name);
                match enclosing_mod(&r, s.parent.as_deref()) {
                    Some(k) => r.container_imports.entry(syms[k].id.as_str()).or_default().extend(imports),
                    None => r.imports.entry(s.module.as_str()).or_default().extend(imports),
                }
            } else if let Ok(trees) = container_use_trees(&s.text) {
                r.container_imports.entry(s.id.as_str()).or_default().extend(trees.into_iter().flat_map(|(_, tree)| parse_use(&tree)));
            }
        }
        r
    }

    fn module_exists(&self, m: &str) -> bool {
        self.module_paths.contains(m) || self.inline_modules.contains_key(m)
    }

    fn parent_module(&self, m: &str) -> Option<String> {
        if let Some((p, _)) = m.rsplit_once("::") {
            return self.module_exists(p).then(|| p.to_string());
        }
        if m != "crate" && self.module_exists("crate") { Some("crate".to_string()) } else { None }
    }

    /// The module a path segment names, from `m`: `crate`, `self`, `super`, a
    /// module of the crate, or a module an import of `m` names.
    fn module_named(&self, m: &str, name: &str) -> Option<String> {
        module_named_in(self, m, None, name)
    }

    /// The module a `use` path names (`super::*` -> the parent module).
    fn resolve_use_module(&self, m: &str, scope: Option<usize>, path: &[String]) -> Option<String> {
        let (first, rest) = path.split_first()?;
        let mut cur = module_named_in(self, m, scope, first)?;
        for seg in rest {
            cur = self.module_named(&cur, seg)?;
        }
        Some(cur)
    }

    /// The module and item name a `use` path names.
    fn resolve_use(&self, m: &str, scope: Option<usize>, path: &[String]) -> Option<(String, String)> {
        let (first, rest) = path.split_first()?;
        if rest.is_empty() {
            return None;
        }
        // An unknown first segment may be another crate (the library's name in
        // an integration test): resolve the rest from this crate's root.
        let mut cur = match first.as_str() {
            "crate" | "self" | "super" => module_named_in(self, m, scope, first)?,
            other => module_named_in(self, m, scope, other).unwrap_or_else(|| "crate".to_string()),
        };
        for seg in &rest[..rest.len() - 1] {
            cur = self.module_named(&cur, seg)?;
        }
        Some((cur, rest.last()?.clone()))
    }

    fn symbols_named(&self, module: &str, name: &str) -> Vec<usize> {
        let mut out = vec![];
        let (base, prefix) = name_scope(self, module);
        if let Some(p) = prefix {
            if let Some(v) = self.names.get(base).and_then(|h| h.get(format!("{p}::{name}").as_str())) {
                out.extend(v);
            }
            return out;
        }
        if let Some(v) = self.heads.get(base).and_then(|h| h.get(name)) {
            out.extend(v);
        }
        if let Some(v) = self.names.get(base).and_then(|h| h.get(name)) {
            out.extend(v);
        }
        out
    }

    fn locals(&self, m: &str, name: &str) -> Vec<usize> {
        let (base, prefix) = name_scope(self, m);
        match prefix {
            Some(p) => self.names.get(base).and_then(|h| h.get(format!("{p}::{name}").as_str())).cloned().unwrap_or_default(),
            None => self.heads.get(base).and_then(|h| h.get(name)).cloned().unwrap_or_default(),
        }
    }

    /// The symbols of `m` whose name is `joined` or ends with `::joined`.
    fn names_in(&self, m: &str, joined: &str) -> Vec<usize> {
        let (base, prefix) = name_scope(self, m);
        let Some(map) = self.names.get(base) else { return vec![] };
        let joined = prefix.map_or_else(|| joined.to_string(), |p| format!("{p}::{joined}"));
        let mut out = vec![];
        for (name, idxs) in map {
            if *name == joined || name.ends_with(&format!("::{joined}")) {
                out.extend(idxs);
            }
        }
        out
    }

    /// The names `m`'s imports bring into scope, globs followed through the
    /// imported module's own imports.
    fn import_visible(&self, m: &str, i: usize, name: &str) -> Vec<usize> {
        let mut out = vec![];
        let mut seen = HashSet::new();
        self.import_visible_in(m, symbol_scope(self, i), name, &mut seen, &mut out);
        out
    }

    fn import_visible_in(
        &self,
        m: &str,
        scope: Option<usize>,
        name: &str,
        seen: &mut HashSet<(String, Option<usize>)>,
        out: &mut Vec<usize>,
    ) {
        if !seen.insert((m.to_string(), scope)) {
            return;
        }
        let Some(imps) = imports_for(self, m, scope) else { return };
        for imp in imps {
            if imp.glob {
                if let Some(g) = self.resolve_use_module(m, scope, &imp.path) {
                    // The module's own item hides what its globs bring in, so
                    // the nearer module's name stops the search there.
                    let found = self.symbols_named(&g, name);
                    if found.is_empty() {
                        self.import_visible_in(&g, None, name, seen, out);
                    } else {
                        out.extend(found);
                    }
                }
            } else if imp.local == name
                && let Some((m2, n)) = self.resolve_use(m, scope, &imp.path)
            {
                out.extend(self.symbols_named(&m2, &n));
            }
        }
    }

    /// The items of `tyname`'s trait impls (D-170): a type mention reaches
    /// them all, so `Display::fmt`, `Drop` and `From` behind `?` are seen.
    fn trait_items(&self, tyname: &str) -> Vec<usize> {
        let key = type_key(tyname);
        let mut out = vec![];
        for &k in self.impls.get(&key).map(Vec::as_slice).unwrap_or(&[]) {
            if let Some(kids) = self.children.get(self.syms[k].id.as_str()) {
                out.extend(kids);
            }
        }
        out
    }

    fn trait_items_named(&self, tyname: &str, name: &str) -> Vec<usize> {
        self.trait_items(tyname).into_iter().filter(|&j| leaf(&self.syms[j].name) == name).collect()
    }

    fn expand_types(&self, out: &mut Vec<usize>) {
        let types: Vec<usize> = out.iter().copied().filter(|&j| is_type(&self.syms[j])).collect();
        for ty in types {
            out.extend(self.trait_items(&self.syms[ty].name));
        }
    }

    /// The enclosing `impl` or `trait`'s name, for `Self::`.
    fn container_name(&self, i: usize) -> Option<String> {
        let mut p = self.syms[i].parent.clone();
        while let Some(id) = p {
            let &k = self.by_id.get(id.as_str())?;
            if self.syms[k].kind_text == "impl" || self.syms[k].kind_text == "trait" {
                return Some(self.syms[k].name.clone());
            }
            p = self.syms[k].parent.clone();
        }
        None
    }

    fn fallback(&self, segs: &[String]) -> Vec<usize> {
        if segs.len() < 2 {
            return vec![];
        }
        segs.last().and_then(|n| self.leaves.get(n.as_str())).cloned().unwrap_or_default()
    }

    fn path(&self, m: &str, i: usize, segs: &[String]) -> Vec<usize> {
        let mut segs = segs.to_vec();
        if segs[0] == "Self"
            && let Some(t) = self.container_name(i)
        {
            segs[0] = t;
        }
        let scope = symbol_scope(self, i);
        let (start, rest): (String, Vec<String>) = match segs[0].as_str() {
            "crate" | "self" | "super" => match module_named_in(self, m, scope, &segs[0]) {
                Some(p) => (p, segs[1..].to_vec()),
                None => return self.fallback(&segs),
            },
            _ => (scope_module(self, m, scope), segs.clone()),
        };
        let mut cur = start;
        let mut k = 0;
        while k < rest.len() {
            let child = if cur == "crate" { rest[k].clone() } else { format!("{cur}::{}", rest[k]) };
            if self.module_exists(&child) {
                cur = child;
                k += 1;
            } else if self.module_exists(&rest[k]) {
                cur = rest[k].clone();
                k += 1;
            } else {
                break;
            }
        }
        if k >= rest.len() {
            return vec![];
        }
        let joined = rest[k..].join("::");
        let mut out = self.names_in(&cur, &joined);
        if k == 0 {
            let head = rest[0].clone();
            for idx in self.locals(&cur, &head).into_iter().chain(self.import_visible(&cur, i, &head)) {
                let mo = self.syms[idx].module.clone();
                out.extend(self.names_in(&mo, &joined));
            }
        }
        if rest.len() >= 2
            && let Some(name) = rest.last()
        {
            out.extend(self.trait_items_named(&rest[0], name));
        }
        if out.is_empty() { self.fallback(&segs) } else { out }
    }

    fn resolve(&self, m: &str, i: usize, mention: &Mention) -> Vec<usize> {
        let mut out = match mention {
            // A method name may belong to any type: link to all of them.
            Mention::Method(f) => self.leaves.get(f.as_str()).cloned().unwrap_or_default(),
            Mention::Macro(n) => self.macros.get(n.as_str()).cloned().unwrap_or_default(),
            Mention::Field(f) => self.bare(i, m, f),
            Mention::Path(segs) if segs.len() == 1 => self.bare(i, m, &segs[0]),
            Mention::Format(name) => self.bare(i, m, name),
            Mention::Path(segs) => self.path(m, i, segs),
        };
        self.expand_types(&mut out);
        let mut seen = vec![];
        out.retain(|&j| {
            j != i
                && if seen.contains(&j) {
                    false
                } else {
                    seen.push(j);
                    true
                }
        });
        out
    }

    /// Items beside `i` in the `#[cfg(test)]` module holding it, named `name`.
    fn test_siblings(&self, i: usize, name: &str) -> Vec<usize> {
        let mut chain = vec![];
        let mut in_tests = false;
        let mut parent = self.syms[i].parent.as_deref();
        while let Some(id) = parent {
            let Some(&k) = self.by_id.get(id) else { return vec![] };
            if self.syms[k].kind_text != "mod" {
                parent = self.syms[k].parent.as_deref();
                continue;
            }
            chain.push(id);
            if cfg_test_in(&outer_attrs(&self.syms[k].text)) {
                in_tests = true;
                break;
            }
            parent = self.syms[k].parent.as_deref();
        }
        if !in_tests {
            return vec![];
        }
        for id in chain {
            let kids = self.children.get(id).map(Vec::as_slice).unwrap_or_default();
            let hits: Vec<usize> = kids.iter().copied().filter(|&c| leaf(&self.syms[c].name) == name).collect();
            if !hits.is_empty() {
                return hits;
            }
        }
        vec![]
    }

    /// Resolve a bare name with the caller's inline-module items shadowing
    /// names its `use super::*` imports bring into scope.
    fn bare(&self, i: usize, m: &str, name: &str) -> Vec<usize> {
        let siblings = self.test_siblings(i, name);
        if !siblings.is_empty() {
            return siblings;
        }
        let mut out = self.locals(m, name);
        out.extend(self.import_visible(m, i, name));
        out
    }
}

/// Whether each symbol is test code (D-170): the `tests/` folder, anything
/// under a `#[cfg(test)]` module, and `#[test]` functions.
fn test_flags(syms: &[Sym], test_mods: &HashSet<String>) -> Vec<bool> {
    let attrs: Vec<Vec<String>> = syms.iter().map(|s| outer_attrs(&s.text)).collect();
    let by_id: HashMap<&str, usize> = syms.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
    syms.iter()
        .enumerate()
        .map(|(i, s)| {
            if test_mods.contains(&s.module) || cfg_test_in(&attrs[i]) || test_attr_in(&attrs[i]) {
                return true;
            }
            // Only a `mod` ancestor makes an item test code: an `impl` holding
            // one `#[cfg(test)]` method is not itself test code.
            let mut p = s.parent.clone();
            while let Some(id) = p {
                let Some(&k) = by_id.get(id.as_str()) else { break };
                if syms[k].kind_text == "mod" && cfg_test_in(&attrs[k]) {
                    return true;
                }
                p = syms[k].parent.clone();
            }
            false
        })
        .collect()
}

/// The `mod x;` declarations a header makes whose `#[cfg(test)]` precedes
/// them, as (the module they sit in, the file module's name).
fn header_cfg_mods(header: &str, parent: &str, out: &mut HashSet<(String, String)>) {
    let mut flag = false;
    for line in header.lines() {
        // The attributes are read one group at a time, so several on a line and a
        // `mod x;` on the same line as its attributes both count (D-170).
        let (attrs, rest) = attrs_and_rest(line.trim_start());
        if attrs.iter().any(|a| cfg_test_attr(a)) {
            flag = true;
        }
        let t = rest.trim_end();
        if t.is_empty() {
            continue;
        }
        if let Some(name) = mod_decl_name(t) {
            if flag {
                out.insert((parent.to_string(), name));
            }
            flag = false;
            continue;
        }
        if !(t.starts_with("use ") || t.starts_with("pub use") || t.starts_with("extern crate")) {
            flag = false;
        }
    }
}

fn mod_decl_name(line: &str) -> Option<String> {
    let line = line.strip_prefix("pub ").unwrap_or(line);
    let rest = line.strip_prefix("mod ")?.trim();
    if !rest.ends_with(';') {
        return None;
    }
    let name = rest.trim_end_matches(';').trim();
    if name.is_empty() || name.contains('{') { None } else { Some(name.to_string()) }
}

#[cfg(test)]
#[path = "rust_refs_tests.rs"]
mod tests;

/// A mention, its 0-based line, and its byte range in the item's text.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Spanned {
    mention: Mention,
    line: usize,
    range: std::ops::Range<usize>,
}

/// The names `text` mentions, with the byte range of each.
fn scanned(text: &str) -> Vec<Spanned> {
    let Ok(stream) = text.parse::<TokenStream>() else { return vec![] };
    let starts = line_starts(text);
    let mut out = vec![];
    scan(&stream, text, &starts, &mut out);
    out
}

/// Where each line starts in `text`.
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    starts
}

/// The byte offset of a token position. A column counts characters, not bytes.
fn byte_at(text: &str, starts: &[usize], at: proc_macro2::LineColumn) -> usize {
    let start = starts.get(at.line.saturating_sub(1)).copied().unwrap_or(text.len());
    let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
    let add = text[start..end].char_indices().nth(at.column).map_or(end - start, |(i, _)| i);
    start + add
}

/// A name that is bound, or a declaration that must not be rewritten, over a
/// byte range of an item's text.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    name: String,
    range: std::ops::Range<usize>,
    /// Whether the name is a type or module, so it also shadows the start of
    /// a longer path.
    types: bool,
}

/// The names an item's text binds, with the part of the text each covers.
struct Scopes {
    bindings: Vec<Binding>,
    imports: Vec<std::ops::Range<usize>>,
    sites: Vec<Site>,
}

fn scan_scopes(text: &str, member: bool, context: &ScopeContext<'_, '_>) -> Scopes {
    let mut scopes = Scopes { bindings: vec![], imports: vec![], sites: vec![] };
    if let Ok(stream) = text.parse::<TokenStream>() {
        let trees: Vec<TokenTree> = stream.into_iter().collect();
        let starts = line_starts(text);
        collect_scopes(&trees, 0, text.len(), member, text, &starts, context, &mut scopes);
    }
    scopes
}

/// Whether a binding hides the path `segs` at `at`: any binding hides a bare
/// name, only a type or module hides the start of a longer path.
fn shadowed(scopes: &Scopes, segs: &[String], at: usize) -> bool {
    let Some(name) = segs.first() else { return false };
    scopes.bindings.iter().any(|b| &b.name == name && b.range.contains(&at) && (segs.len() == 1 || b.types))
}

/// The mention ranges that are declarations or are shadowed by a binding; a
/// `member` of an `impl` or `trait` does not bind its own name. A `use` inside
/// a block is a site or shadows its local name, never an ordinary path mention.
fn bound(text: &str, mentions: &[Spanned], member: bool, context: &ScopeContext<'_, '_>) -> (HashSet<std::ops::Range<usize>>, Vec<Site>) {
    let scopes = scan_scopes(text, member, context);
    let in_import =
        |range: &std::ops::Range<usize>| scopes.imports.iter().any(|import| import.start <= range.start && range.end <= import.end);
    let hidden = mentions
        .iter()
        .filter(|m| match &m.mention {
            Mention::Macro(_) | Mention::Field(_) => true,
            Mention::Path(segs) => in_import(&m.range) || shadowed(&scopes, segs, m.range.start),
            Mention::Format(name) => shadowed(&scopes, std::slice::from_ref(name), m.range.start),
            Mention::Method(_) => false,
        })
        .map(|m| m.range.clone())
        .collect();
    (hidden, scopes.sites)
}

/// A name in a pattern: a binding, or a field name that is only an exclusion.
struct PatternName {
    name: String,
    range: std::ops::Range<usize>,
    binds: bool,
}

fn pattern_names(trees: &[TokenTree], text: &str, starts: &[usize]) -> Vec<PatternName> {
    let mut out = vec![];
    collect_pattern(trees, text, starts, &mut out);
    out
}

fn collect_pattern(trees: &[TokenTree], text: &str, starts: &[usize], out: &mut Vec<PatternName>) {
    let mut i = 0;
    while i < trees.len() {
        match &trees[i] {
            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
                struct_pattern(g, text, starts, out);
                i += 1;
            }
            TokenTree::Group(g) => {
                let kids: Vec<TokenTree> = g.stream().into_iter().collect();
                collect_pattern(&kids, text, starts, out);
                i += 1;
            }
            TokenTree::Ident(id) => {
                let name = id.to_string();
                let binds = !is_keyword(&name)
                    && !name.starts_with('_')
                    && !name.chars().next().is_some_and(char::is_uppercase)
                    && !path_adjacent(trees, i);
                if binds {
                    out.push(PatternName { name, range: token_range(&trees[i], text, starts), binds: true });
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
}

fn struct_pattern(group: &proc_macro2::Group, text: &str, starts: &[usize], out: &mut Vec<PatternName>) {
    let kids: Vec<TokenTree> = group.stream().into_iter().collect();
    for part in comma_parts(&kids) {
        if let Some(TokenTree::Ident(id)) = part.first() {
            let name = id.to_string();
            if !is_keyword(&name) && !name.starts_with('_') && !name.chars().next().is_some_and(char::is_uppercase) {
                let range = token_range(&part[0], text, starts);
                if is_colon(part.get(1)) {
                    out.push(PatternName { name, range, binds: false });
                    collect_pattern(&part[2..], text, starts, out);
                } else {
                    out.push(PatternName { name, range, binds: true });
                }
                continue;
            }
        }
        collect_pattern(part, text, starts, out);
    }
}

fn add_pattern(names: Vec<PatternName>, from: usize, to: usize, out: &mut Vec<Binding>) {
    for name in names {
        out.push(Binding { name: name.name.clone(), range: name.range, types: false });
        if name.binds {
            out.push(Binding { name: name.name, range: from..to, types: false });
        }
    }
}

fn comma_parts(trees: &[TokenTree]) -> Vec<&[TokenTree]> {
    let (mut angle, mut start, mut out) = (0i32, 0, vec![]);
    for (i, tree) in trees.iter().enumerate() {
        if let TokenTree::Punct(p) = tree {
            match p.as_char() {
                '<' => angle += 1,
                '>' if angle > 0 => angle -= 1,
                ',' if angle == 0 => {
                    out.push(&trees[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
    }
    out.push(&trees[start..]);
    out.into_iter().filter(|p| !p.is_empty()).collect()
}

fn before_colon(trees: &[TokenTree]) -> &[TokenTree] {
    for (i, tree) in trees.iter().enumerate() {
        if is_colon(Some(tree)) && !is_colon(trees.get(i.wrapping_sub(1))) && !is_colon(trees.get(i + 1)) {
            return &trees[..i];
        }
    }
    trees
}

fn is_colon(tree: Option<&TokenTree>) -> bool {
    matches!(tree, Some(TokenTree::Punct(p)) if p.as_char() == ':')
}

fn path_adjacent(trees: &[TokenTree], i: usize) -> bool {
    let sep = |a: Option<&TokenTree>, b: Option<&TokenTree>| {
        matches!(a, Some(TokenTree::Punct(p)) if p.as_char() == ':') && matches!(b, Some(TokenTree::Punct(p)) if p.as_char() == ':')
    };
    (i >= 2 && sep(trees.get(i - 2), trees.get(i - 1))) || sep(trees.get(i + 1), trees.get(i + 2))
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "union"
            | "unsafe"
            | "use"
            | "where"
            | "while"
    )
}

fn token_range(tree: &TokenTree, text: &str, starts: &[usize]) -> std::ops::Range<usize> {
    byte_at(text, starts, tree.span().start())..byte_at(text, starts, tree.span().end())
}

fn find_word(trees: &[TokenTree], from: usize, word: &str) -> Option<usize> {
    (from..trees.len()).find(|&i| matches!(&trees[i], TokenTree::Ident(id) if id.to_string() == word))
}

fn find_punct(trees: &[TokenTree], from: usize, c: char) -> Option<usize> {
    (from..trees.len()).find(|&i| matches!(&trees[i], TokenTree::Punct(p) if p.as_char() == c))
}

fn find_arrow(trees: &[TokenTree], from: usize) -> Option<usize> {
    (from..trees.len().saturating_sub(1)).find(|&i| {
        matches!(&trees[i], TokenTree::Punct(p) if p.as_char() == '=') && matches!(&trees[i + 1], TokenTree::Punct(p) if p.as_char() == '>')
    })
}

fn next_group(trees: &[TokenTree], from: usize, delimiter: Delimiter) -> Option<usize> {
    (from..trees.len()).find(|&i| matches!(&trees[i], TokenTree::Group(g) if g.delimiter() == delimiter))
}

fn is_block_brace(trees: &[TokenTree], group: usize) -> bool {
    let mut i = group;
    while i > 0 {
        i -= 1;
        match &trees[i] {
            TokenTree::Ident(id)
                if matches!(
                    id.to_string().as_str(),
                    "if" | "while"
                        | "for"
                        | "match"
                        | "loop"
                        | "else"
                        | "fn"
                        | "impl"
                        | "trait"
                        | "struct"
                        | "enum"
                        | "union"
                        | "mod"
                        | "extern"
                        | "unsafe"
                        | "async"
                        | "const"
                        | "static"
                ) =>
            {
                return true;
            }
            TokenTree::Punct(p) if p.as_char() == ';' => return false,
            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => return false,
            _ => {}
        }
    }
    false
}

fn field_names(group: &proc_macro2::Group, text: &str, starts: &[usize], out: &mut Vec<Binding>) {
    let kids: Vec<TokenTree> = group.stream().into_iter().collect();
    for part in comma_parts(&kids) {
        if matches!(part.first(), Some(TokenTree::Punct(p)) if p.as_char() == '.') {
            continue;
        }
        let mut i = 0;
        while i < part.len() {
            match &part[i] {
                TokenTree::Punct(p) if p.as_char() == '#' => {
                    i += 1;
                    if matches!(part.get(i), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket) {
                        i += 1;
                    }
                }
                TokenTree::Ident(id) if id.to_string() == "pub" => {
                    i += 1;
                    if matches!(part.get(i), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis) {
                        i += 1;
                    }
                }
                TokenTree::Ident(id) => {
                    let name = id.to_string();
                    if !is_keyword(&name) && !name.starts_with('_') {
                        out.push(Binding { name, range: token_range(&part[i], text, starts), types: false });
                    }
                    break;
                }
                _ => i += 1,
            }
        }
    }
}

fn collect_scopes(
    trees: &[TokenTree],
    scope_start: usize,
    scope_end: usize,
    container: bool,
    text: &str,
    starts: &[usize],
    context: &ScopeContext<'_, '_>,
    out: &mut Scopes,
) {
    let scope = scope_start..scope_end;
    let mut i = 0;
    while i < trees.len() {
        match &trees[i] {
            TokenTree::Group(_) => nested_scopes(trees, i, &scope, text, starts, context, out),
            TokenTree::Ident(_) => {
                word_bindings(trees, i, &scope, container, text, starts, context, out);
                literal_fields(trees, i, text, starts, &mut out.bindings);
            }
            TokenTree::Punct(p) if p.as_char() == '|' && closure_start(trees, i) => {
                i = closure_binding(trees, i, text, starts, &mut out.bindings)
            }
            _ => {}
        }
        i += 1;
    }
}

fn previous_is(trees: &[TokenTree], i: usize, word: &str) -> bool {
    matches!(i.checked_sub(1).and_then(|j| trees.get(j)), Some(TokenTree::Ident(id)) if id.to_string() == word)
}

fn next_is(trees: &[TokenTree], i: usize, word: &str) -> bool {
    matches!(trees.get(i + 1), Some(TokenTree::Ident(id)) if id.to_string() == word)
}

fn closure_start(trees: &[TokenTree], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    match &trees[i - 1] {
        TokenTree::Ident(id) => matches!(id.to_string().as_str(), "move" | "return" | "else" | "async"),
        TokenTree::Punct(p) => match p.as_char() {
            '|' => matches!(p.spacing(), proc_macro2::Spacing::Alone),
            c => matches!(c, '=' | '(' | ',' | '{' | ';' | '>' | '<' | '&' | ':' | '!' | '+' | '-' | '*'),
        },
        TokenTree::Group(_) => false,
        TokenTree::Literal(_) => false,
    }
}

fn closure_body(trees: &[TokenTree], from: usize, text: &str, starts: &[usize], bar_end: usize) -> usize {
    if let Some(TokenTree::Group(g)) = trees.get(from)
        && g.delimiter() == Delimiter::Brace
    {
        return token_range(&trees[from], text, starts).end;
    }
    let stop = find_punct(trees, from, ',').or_else(|| find_punct(trees, from, ';')).unwrap_or(trees.len());
    if stop > from { token_range(&trees[stop - 1], text, starts).end } else { bar_end }
}

fn owner_keyword(trees: &[TokenTree], group: usize) -> Option<String> {
    let mut i = group;
    while i > 0 {
        i -= 1;
        match &trees[i] {
            TokenTree::Ident(id) => {
                let word = id.to_string();
                if matches!(word.as_str(), "impl" | "trait" | "fn" | "struct" | "enum" | "union" | "mod" | "extern") {
                    return Some(word);
                }
            }
            TokenTree::Punct(p) if p.as_char() == ';' => return None,
            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => return None,
            _ => {}
        }
    }
    None
}

fn match_arms(trees: &[TokenTree], text: &str, starts: &[usize], out: &mut Vec<Binding>) {
    let mut from = 0;
    while from < trees.len() {
        let Some(arrow) = find_arrow(trees, from) else { break };
        let pattern = &trees[from..arrow];
        let guard = find_word(pattern, 0, "if").unwrap_or(pattern.len());
        let pattern = &pattern[..guard];
        let end = pattern.last().map_or(0, |t| token_range(t, text, starts).end);
        let after = arrow + 2;
        let (body_end, next) = match trees.get(after) {
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => (token_range(&trees[after], text, starts).end, after + 1),
            _ => {
                let comma = find_punct(trees, after, ',').unwrap_or(trees.len());
                let end = if comma > after { token_range(&trees[comma - 1], text, starts).end } else { end };
                (end, comma + 1)
            }
        };
        add_pattern(pattern_names(pattern, text, starts), end, body_end, out);
        from = next;
    }
}

/// How an exact resolution reached its target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Via {
    /// A name defined in the mentioning module.
    Own,
    /// A name a `use` line binds, with the local name it binds.
    Import(String),
    /// A name a glob import brings into scope.
    Glob,
    /// A qualified path, including `self::`, `super::` and `crate::`.
    Path,
}

/// One place a moved item is named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    /// The bytes to replace.
    pub range: std::ops::Range<usize>,
    /// The `as name` part of an aliased import, when there is one.
    pub alias: Option<std::ops::Range<usize>>,
    /// The path as written.
    pub segs: Vec<String>,
    /// How many leading `segs` are written before `range`, in an enclosing
    /// `use` group's prefix.
    pub outside: usize,
    /// The item the mention resolves to.
    pub target: usize,
    /// How the name reached that item.
    pub via: Via,
    /// The symbol whose text holds the range; `None` for a module header.
    pub symbol: Option<usize>,
    /// Whether the site is a name captured inside a format string.
    pub format: bool,
}

/// Every unshadowed path mention of `syms[target]`, in any symbol's text.
#[cfg(test)]
pub fn rewrite_sites(syms: &[Sym], modules: &[ModuleRow], target: usize) -> Vec<Site> {
    let refs = Refs::new(syms, modules);
    rewrite_sites_in(&refs, syms, target)
}

/// Resolve one path mention to one item, without a last-name fallback.
fn resolve_exact(refs: &Refs, m: &str, i: usize, segs: &[String]) -> Option<(usize, Via)> {
    if segs.is_empty() {
        return None;
    }
    if segs.len() == 1 {
        return bare_exact(refs, m, i, &segs[0]);
    }
    let mut segs = segs.to_vec();
    if segs[0] == "Self" {
        segs[0] = refs.container_name(i)?;
        return associated_exact(refs, m, &segs);
    }
    let scope = symbol_scope(refs, i);
    let (module, rest, via) = match segs[0].as_str() {
        "crate" | "self" | "super" => (module_named_in(refs, m, scope, &segs[0])?, &segs[1..], Via::Path),
        head => {
            if let Some(module) = module_named_in(refs, m, scope, head) {
                (module, &segs[1..], Via::Path)
            } else if item_visible(refs, m, i, head) {
                return associated_exact(refs, m, &segs);
            } else {
                return None;
            }
        }
    };
    exact_in(refs, &module, rest, via)
}

fn bare_exact(refs: &Refs, m: &str, i: usize, name: &str) -> Option<(usize, Via)> {
    let own = item_targets(refs, refs.locals(m, name));
    if !own.is_empty() {
        return one_target(own, Via::Own);
    }
    if let Some(named) = named_imports(refs, m, i, name) {
        return one_target(item_targets(refs, named), Via::Import(name.to_string()));
    }
    one_target(item_targets(refs, refs.import_visible(m, i, name)), Via::Glob)
}

fn item_visible(refs: &Refs, m: &str, i: usize, name: &str) -> bool {
    !refs.locals(m, name).is_empty() || !refs.import_visible(m, i, name).is_empty()
}

fn associated_exact(refs: &Refs, m: &str, segs: &[String]) -> Option<(usize, Via)> {
    one_target(item_targets(refs, refs.names_in(m, &segs.join("::"))), Via::Path)
}

fn exact_in(refs: &Refs, module: &str, rest: &[String], via: Via) -> Option<(usize, Via)> {
    let mut cur = module.to_string();
    let mut k = 0;
    while k < rest.len() {
        if let Some(next) = refs.module_named(&cur, &rest[k]) {
            if k + 1 == rest.len() {
                let item = refs.symbols_named(&cur, &rest[k]);
                return one_target(item_targets(refs, item), via);
            }
            cur = next;
            k += 1;
        } else {
            break;
        }
    }
    if k == rest.len() {
        return None;
    }
    let joined = rest[k..].join("::");
    let targets = if joined.contains("::") { refs.names_in(&cur, &joined) } else { refs.symbols_named(&cur, &joined) };
    let targets = item_targets(refs, targets);
    let targets = if targets.is_empty() && !joined.contains("::") { reexport_targets(refs, &cur, &joined) } else { targets };
    one_target(targets, via)
}

fn one_target(mut targets: Vec<usize>, via: Via) -> Option<(usize, Via)> {
    targets.sort_unstable();
    targets.dedup();
    match targets.as_slice() {
        [one] => Some((*one, via)),
        _ => None,
    }
}

/// The `use` tree sites of `module`: its header's lines and its `use` symbols.
pub fn use_sites(syms: &[Sym], modules: &[ModuleRow], module: &str) -> Result<Vec<Site>, Failure> {
    let refs = Refs::new(syms, modules);
    let mut out = vec![];
    let unparsed = unparsed_imports(module);
    if let Some(row) = modules.iter().find(|m| m.path == module) {
        for (base, tree) in container_use_trees(&row.header).map_err(&unparsed)? {
            use_tree_sites(&refs, module, base, &tree, None, None, &mut out);
        }
    }
    for (i, symbol) in syms.iter().enumerate().filter(|(_, s)| s.module == module && (s.kind_text == "use" || s.kind_text == "mod")) {
        for (base, tree) in container_use_trees(&symbol.text).map_err(&unparsed)? {
            use_tree_sites(&refs, module, base, &tree, Some(i), None, &mut out);
        }
    }
    Ok(out)
}

/// Each `use` tree in `text`, with the byte offset where the tree starts.
fn use_trees(text: &str) -> Result<Vec<(usize, &str)>, syn::Error> {
    Ok(crate::language::rust::import_tree_ranges(text)?.into_iter().map(|range| (range.start, &text[range])).collect())
}

fn use_tree_sites(refs: &Refs, module: &str, base: usize, text: &str, symbol: Option<usize>, target: Option<usize>, out: &mut Vec<Site>) {
    let starts = line_starts(text);
    let Ok(tree) = syn::parse_str::<syn::UseTree>(text) else { return };
    let scope = symbol.and_then(|i| symbol_scope(refs, i));
    recurse_use_tree(refs, module, scope, &tree, &mut vec![], None, base, text, &starts, symbol, target, out);
}

fn use_span(text: &str, starts: &[usize], span: proc_macro2::Span) -> std::ops::Range<usize> {
    byte_at(text, starts, span.start())..byte_at(text, starts, span.end())
}

fn recurse_use_tree(
    refs: &Refs,
    module: &str,
    scope: Option<usize>,
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    root: Option<(usize, usize)>,
    base: usize,
    text: &str,
    starts: &[usize],
    symbol: Option<usize>,
    target: Option<usize>,
    out: &mut Vec<Site>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            let at = base + use_span(text, starts, path.ident.span()).start;
            let root = Some(root.unwrap_or((at, prefix.len())));
            prefix.push(path.ident.to_string());
            recurse_use_tree(refs, module, scope, &path.tree, prefix, root, base, text, starts, symbol, target, out);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let leaf = name.ident.to_string();
            if leaf == "self" {
                return;
            }
            let mut segs = prefix.clone();
            segs.push(leaf.clone());
            if let Some(found) = use_site_target(refs, module, scope, &segs, target) {
                let span = use_span(text, starts, name.ident.span());
                let (start, outside) = root.unwrap_or((base + span.start, prefix.len()));
                out.push(Site {
                    range: start..base + span.end,
                    alias: None,
                    segs,
                    outside,
                    target: found,
                    via: Via::Import(leaf),
                    symbol,
                    format: false,
                });
            }
        }
        syn::UseTree::Rename(rename) => {
            let leaf = rename.ident.to_string();
            if leaf == "self" {
                return;
            }
            let mut segs = prefix.clone();
            segs.push(leaf);
            if let Some(found) = use_site_target(refs, module, scope, &segs, target) {
                let span = use_span(text, starts, rename.ident.span());
                let (start, outside) = root.unwrap_or((base + span.start, prefix.len()));
                let alias_start = base + byte_at(text, starts, rename.as_token.span.start());
                let alias_end = base + use_span(text, starts, rename.rename.span()).end;
                let alias = alias_start..alias_end;
                let via = Via::Import(rename.rename.to_string());
                out.push(Site {
                    range: start..base + span.end,
                    alias: Some(alias),
                    segs,
                    outside,
                    target: found,
                    via,
                    symbol,
                    format: false,
                });
            }
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                recurse_use_tree(refs, module, scope, item, prefix, None, base, text, starts, symbol, target, out);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

fn use_target(refs: &Refs, module: &str, scope: Option<usize>, segs: &[String]) -> Option<usize> {
    match use_targets(refs, module, scope, segs).as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

/// Targets a name can denote: an `impl` row is a container, not an item.
fn item_targets(refs: &Refs, targets: Vec<usize>) -> Vec<usize> {
    targets.into_iter().filter(|&i| !matches!(refs.syms[i].kind_text.as_str(), "impl" | "use" | "mod" | "extern")).collect()
}

/// The byte just past a group's opening delimiter.
fn group_start(tree: &TokenTree, text: &str, starts: &[usize]) -> usize {
    match tree {
        TokenTree::Group(group) => byte_at(text, starts, group.span_open().end()),
        _ => 0,
    }
}

fn previous_punct(trees: &[TokenTree], i: usize, c: char) -> bool {
    matches!(i.checked_sub(1).and_then(|j| trees.get(j)), Some(TokenTree::Punct(p)) if p.as_char() == c)
}

/// A `let` in `if let`, `while let` or a let chain, which binds only in the body.
fn is_condition_let(trees: &[TokenTree], i: usize) -> bool {
    previous_is(trees, i, "if") || previous_is(trees, i, "while") || previous_punct(trees, i, '&')
}

/// The name an item keyword at `at` declares; `None` for `'static`, `*const`,
/// `&raw const`, `fn(…)` types and a method named like a keyword.
fn declared_name(trees: &[TokenTree], at: usize) -> Option<usize> {
    if ['\'', '*', '.'].iter().any(|&c| previous_punct(trees, at, c)) || previous_is(trees, at, "raw") {
        return None;
    }
    let name = if next_is(trees, at, "mut") { at + 2 } else { at + 1 };
    match trees.get(name) {
        Some(TokenTree::Ident(id)) if !is_keyword(&id.to_string()) => Some(name),
        _ => None,
    }
}

/// A function's parameter list: the first parenthesized group outside its generics.
fn params_group(trees: &[TokenTree], from: usize) -> Option<usize> {
    let mut depth = 0usize;
    for i in from..trees.len() {
        match &trees[i] {
            TokenTree::Punct(p) if p.as_char() == '<' => depth += 1,
            TokenTree::Punct(p) if p.as_char() == '>' && !previous_punct(trees, i, '-') => depth = depth.saturating_sub(1),
            TokenTree::Punct(p) if p.as_char() == ';' => return None,
            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => return None,
            TokenTree::Group(g) if depth == 0 && g.delimiter() == Delimiter::Parenthesis => return Some(i),
            _ => {}
        }
    }
    None
}

/// The longest prefix of a path mention in `syms[i]` that names `target`.
fn path_site(refs: &Refs, i: usize, range: &std::ops::Range<usize>, segs: &[String], target: usize, format: bool) -> Option<Site> {
    let symbol = &refs.syms[i];
    (1..=segs.len()).rev().find_map(|k| {
        let (found, via) = resolve_exact(refs, &symbol.module, i, &segs[..k])?;
        if found != target {
            return None;
        }
        let end = if k == segs.len() { range.end } else { prefix_end(&symbol.text, range, &segs[..k])? };
        Some(Site { range: range.start..end, alias: None, segs: segs[..k].to_vec(), outside: 0, target, via, symbol: Some(i), format })
    })
}

/// The byte just past `segs`, the leading segments of the path in `range`.
fn prefix_end(text: &str, range: &std::ops::Range<usize>, segs: &[String]) -> Option<usize> {
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut at = range.start;
    for seg in segs {
        let found = text[at..range.end]
            .match_indices(seg.as_str())
            .map(|(p, _)| at + p)
            .find(|&p| !text[..p].ends_with(ident) && !text[p + seg.len()..].starts_with(ident))?;
        at = found + seg.len();
    }
    Some(at)
}

/// The items `m`'s non-glob imports bind to the local `name`, or `None` when
/// no named import binds it.
fn named_imports(refs: &Refs, m: &str, i: usize, name: &str) -> Option<Vec<usize>> {
    let scope = symbol_scope(refs, i);
    let imps = imports_for(refs, m, scope)?;
    let named: Vec<&Import> = imps.iter().filter(|imp| !imp.glob && imp.local == name).collect();
    if named.is_empty() {
        return None;
    }
    Some(
        named
            .into_iter()
            .filter_map(|imp| refs.resolve_use(m, scope, &imp.path))
            .flat_map(|(module, leaf)| refs.symbols_named(&module, &leaf))
            .collect(),
    )
}

/// The bindings inside the group at `i`: a brace group is its own scope.
fn nested_scopes(
    trees: &[TokenTree],
    i: usize,
    scope: &std::ops::Range<usize>,
    text: &str,
    starts: &[usize],
    context: &ScopeContext<'_, '_>,
    out: &mut Scopes,
) {
    let TokenTree::Group(g) = &trees[i] else { return };
    let nested = g.delimiter() == Delimiter::Brace;
    let (start, end) =
        if nested { (group_start(&trees[i], text, starts), token_range(&trees[i], text, starts).end) } else { (scope.start, scope.end) };
    let inside = nested && matches!(owner_keyword(trees, i).as_deref(), Some("impl" | "trait"));
    let kids: Vec<TokenTree> = g.stream().into_iter().collect();
    collect_scopes(&kids, start, end, inside, text, starts, context, out);
}

/// The bindings the keyword at `i` introduces, if it is one that binds.
fn word_bindings(
    trees: &[TokenTree],
    i: usize,
    scope: &std::ops::Range<usize>,
    container: bool,
    text: &str,
    starts: &[usize],
    context: &ScopeContext<'_, '_>,
    out: &mut Scopes,
) {
    match trees[i].to_string().as_str() {
        "let" if is_condition_let(trees, i) => condition_let_binding(trees, i, text, starts, &mut out.bindings),
        "let" => let_binding(trees, i, scope.end, text, starts, &mut out.bindings),
        "for" => for_binding(trees, i, text, starts, &mut out.bindings),
        "match" => match_binding(trees, i, text, starts, &mut out.bindings),
        "use" => use_binding(trees, i, scope.end, text, starts, context, out),
        "fn" => fn_binding(trees, i, scope, container, text, starts, &mut out.bindings),
        "struct" | "enum" | "union" | "const" | "static" | "type" | "trait" => {
            item_binding(trees, i, scope, container, text, starts, &mut out.bindings)
        }
        _ => {}
    }
}

/// `if let` and `while let`: the pattern binds in the block that follows.
fn condition_let_binding(trees: &[TokenTree], i: usize, text: &str, starts: &[usize], out: &mut Vec<Binding>) {
    let Some(sep) = find_punct(trees, i + 1, '=') else { return };
    let Some(body) = next_group(trees, sep + 1, Delimiter::Brace) else { return };
    let names = pattern_names(before_colon(&trees[i + 1..sep]), text, starts);
    add_pattern(names, group_start(&trees[body], text, starts), token_range(&trees[body], text, starts).end, out);
}

/// A `let` statement: the pattern binds from its `;` to the end of the scope.
fn let_binding(trees: &[TokenTree], i: usize, scope_end: usize, text: &str, starts: &[usize], out: &mut Vec<Binding>) {
    let sep = find_punct(trees, i + 1, '=').or_else(|| find_punct(trees, i + 1, ';')).unwrap_or(trees.len());
    let pattern = before_colon(&trees[i + 1..sep]);
    let end = find_punct(trees, i + 1, ';').map_or(scope_end, |s| token_range(&trees[s], text, starts).end);
    add_pattern(pattern_names(pattern, text, starts), end, scope_end, out);
}

/// A `for` loop: the pattern binds in the loop's body. `for<'a>` and an
/// `impl … for` header are not loops.
fn for_binding(trees: &[TokenTree], i: usize, text: &str, starts: &[usize], out: &mut Vec<Binding>) {
    if in_impl_header(trees, i) || previous_punct(trees, i + 2, '<') {
        return;
    }
    let Some(at) = find_word(trees, i + 1, "in") else { return };
    let Some(body) = next_group(trees, at + 1, Delimiter::Brace) else { return };
    let names = pattern_names(&trees[i + 1..at], text, starts);
    add_pattern(names, group_start(&trees[body], text, starts), token_range(&trees[body], text, starts).end, out);
}

/// Whether `impl` comes before `i` in the same statement.
fn in_impl_header(trees: &[TokenTree], i: usize) -> bool {
    trees[..i]
        .iter()
        .rev()
        .take_while(|t| match t {
            TokenTree::Punct(p) => p.as_char() != ';',
            TokenTree::Group(g) => g.delimiter() != Delimiter::Brace,
            _ => true,
        })
        .any(|t| matches!(t, TokenTree::Ident(id) if id.to_string() == "impl"))
}

/// A `match`: each arm's pattern binds in that arm.
fn match_binding(trees: &[TokenTree], i: usize, text: &str, starts: &[usize], out: &mut Vec<Binding>) {
    let Some(body) = next_group(trees, i + 1, Delimiter::Brace) else { return };
    let TokenTree::Group(g) = &trees[body] else { return };
    let kids: Vec<TokenTree> = g.stream().into_iter().collect();
    match_arms(&kids, text, starts, out);
}

/// A `use` inside a block: a name that resolves anywhere but the target binds
/// from its `;` to the end of the scope; a name that resolves to the target is
/// an import site instead.
fn use_binding(
    trees: &[TokenTree],
    i: usize,
    scope_end: usize,
    text: &str,
    starts: &[usize],
    context: &ScopeContext<'_, '_>,
    out: &mut Scopes,
) {
    let Some(semi) = find_punct(trees, i + 1, ';') else { return };
    if semi == i + 1 {
        return;
    }
    let semi_range = token_range(&trees[semi], text, starts);
    let tree_start = token_range(&trees[i + 1], text, starts).start;
    let tree = &text[tree_start..semi_range.start];
    out.imports.push(tree_start..semi_range.start);
    use_tree_sites(context.refs, context.module, tree_start, tree, Some(context.symbol), context.target, &mut out.sites);
    for import in parse_use(tree).into_iter().filter(|import| !import.glob) {
        if block_use_binds(context.refs, context.module, symbol_scope(context.refs, context.symbol), context.target, &import.path) {
            out.bindings.push(Binding { name: import.local, range: semi_range.end..scope_end, types: true });
        }
    }
}

/// A `fn`: its name binds in the scope, its parameters in its body.
fn fn_binding(
    trees: &[TokenTree],
    i: usize,
    scope: &std::ops::Range<usize>,
    container: bool,
    text: &str,
    starts: &[usize],
    out: &mut Vec<Binding>,
) {
    let Some(name) = declared_name(trees, i) else { return };
    out.extend(declaration(trees, name, scope, container, false, text, starts));
    let Some(params) = params_group(trees, name + 1) else { return };
    let TokenTree::Group(g) = &trees[params] else { return };
    let body = next_group(trees, params + 1, Delimiter::Brace);
    let body = body.map(|b| token_range(&trees[b], text, starts)).unwrap_or(scope.end..scope.end);
    let kids: Vec<TokenTree> = g.stream().into_iter().collect();
    for part in comma_parts(&kids) {
        add_pattern(pattern_names(before_colon(part), text, starts), body.start, body.end, out);
    }
}

/// A local `struct`, `enum`, `union`, `const`, `static`, `type` or `trait`:
/// its name binds in the scope; a struct's or enum's fields are declarations.
fn item_binding(
    trees: &[TokenTree],
    i: usize,
    scope: &std::ops::Range<usize>,
    container: bool,
    text: &str,
    starts: &[usize],
    out: &mut Vec<Binding>,
) {
    let Some(name) = declared_name(trees, i) else { return };
    let word = trees[i].to_string();
    let types = !matches!(word.as_str(), "const" | "static");
    out.extend(declaration(trees, name, scope, container, types, text, starts));
    if matches!(word.as_str(), "struct" | "enum" | "union")
        && let Some(body) = next_group(trees, name + 1, Delimiter::Brace)
        && let TokenTree::Group(g) = &trees[body]
    {
        field_names(g, text, starts, out);
    }
}

/// A declared name's own token, and outside an `impl` or `trait` the scope
/// it binds.
fn declaration(
    trees: &[TokenTree],
    name: usize,
    scope: &std::ops::Range<usize>,
    container: bool,
    types: bool,
    text: &str,
    starts: &[usize],
) -> Vec<Binding> {
    let word = trees[name].to_string();
    let mut out = vec![Binding { name: word.clone(), range: token_range(&trees[name], text, starts), types }];
    if !container {
        out.push(Binding { name: word, range: scope.clone(), types });
    }
    out
}

/// A struct literal's or pattern's field names after a name at `i`.
fn literal_fields(trees: &[TokenTree], i: usize, text: &str, starts: &[usize], out: &mut Vec<Binding>) {
    let word = trees[i].to_string();
    if let Some(TokenTree::Group(g)) = trees.get(i + 1)
        && g.delimiter() == Delimiter::Brace
        && (!is_keyword(&word) || word == "Self")
        && !is_block_brace(trees, i + 1)
    {
        field_names(g, text, starts, out);
    }
}

/// A closure at `i`: its parameters bind in its body. Returns the index of the
/// closing `|`, or `i` when there is none.
fn closure_binding(trees: &[TokenTree], i: usize, text: &str, starts: &[usize], out: &mut Vec<Binding>) -> usize {
    let Some(close) = (i + 1..trees.len()).find(|&j| matches!(&trees[j], TokenTree::Punct(p) if p.as_char() == '|')) else {
        return i;
    };
    let names = pattern_names(&trees[i + 1..close], text, starts);
    let bar_end = token_range(&trees[close], text, starts).end;
    let body = closure_body(trees, close + 1, text, starts, bar_end);
    add_pattern(names, bar_end, body, out);
    close
}

/// The exact rewrite a scope scan is for.
struct ScopeContext<'a, 'b> {
    refs: &'a Refs<'b>,
    module: &'a str,
    symbol: usize,
    target: Option<usize>,
}

/// The item a block `use` names in this crate; a foreign or unknown head names
/// an item the target cannot be.
fn block_use_target(refs: &Refs, module: &str, scope: Option<usize>, path: &[String]) -> Option<usize> {
    match block_use_targets(refs, module, scope, path)?.as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

fn use_targets(refs: &Refs, module: &str, scope: Option<usize>, segs: &[String]) -> Vec<usize> {
    let Some((module, name)) = refs.resolve_use(module, scope, segs) else { return vec![] };
    let mut targets = item_targets(refs, refs.symbols_named(&module, &name));
    targets.sort_unstable();
    targets.dedup();
    targets
}

fn block_use_targets(refs: &Refs, module: &str, scope: Option<usize>, path: &[String]) -> Option<Vec<usize>> {
    let first = path.first()?;
    if !matches!(first.as_str(), "crate" | "self" | "super") && module_named_in(refs, module, scope, first).is_none() {
        return None;
    }
    Some(use_targets(refs, module, scope, path))
}

fn block_use_binds(refs: &Refs, module: &str, scope: Option<usize>, target: Option<usize>, path: &[String]) -> bool {
    let Some(target) = target else { return true };
    let Some(targets) = block_use_targets(refs, module, scope, path) else { return true };
    match targets.as_slice() {
        [] => true,
        [one] => *one != target,
        _ => false,
    }
}

fn use_site_target(refs: &Refs, module: &str, scope: Option<usize>, segs: &[String], target: Option<usize>) -> Option<usize> {
    let found = if target.is_some() { block_use_target(refs, module, scope, segs) } else { use_target(refs, module, scope, segs) };
    found.filter(|&found| target.is_none_or(|want| found == want))
}

/// The failure for `module`'s imports that don't parse.
fn unparsed_imports(module: &str) -> impl Fn(syn::Error) -> Failure + '_ {
    move |e| Failure(format!("imports in {module} don't parse: {e}"), 1)
}

/// One `use` tree's binding, and where it is written.
#[derive(Clone, Debug)]
pub(crate) struct ImportSite {
    pub local: String,
    pub path: Vec<String>,
    pub range: std::ops::Range<usize>,
    pub alias: Option<std::ops::Range<usize>>,
    pub outside: usize,
    /// Whether this imports every name of `path` (`use path::*`).
    pub glob: bool,
    /// The outer attributes written before the `use`, such as `#[cfg(test)]`.
    pub attributes: String,
}

/// Every import binding of `module`: its header's lines, its `use` symbols and
/// the imports an inline module holds before its first item.
pub(crate) fn import_sites(syms: &[Sym], modules: &[ModuleRow], module: &str, root_only: bool) -> Result<Vec<ImportSite>, Failure> {
    let unparsed = unparsed_imports(module);
    let mut out = vec![];
    if let Some(row) = modules.iter().find(|m| m.path == module) {
        out.extend(import_sites_in(&row.header).map_err(&unparsed)?);
    }
    for symbol in syms.iter().filter(|s| {
        s.module == module
            && if root_only { s.kind_text == "use" && s.parent.is_none() } else { s.kind_text == "use" || s.kind_text == "mod" }
    }) {
        out.extend(import_sites_in(&symbol.text).map_err(&unparsed)?);
    }
    Ok(out)
}

/// The import bindings `text` holds.
pub(crate) fn import_sites_in(text: &str) -> Result<Vec<ImportSite>, syn::Error> {
    let mut out = vec![];
    for (base, tree, attributes) in container_use_trees_with_attributes(text)? {
        let Ok(parsed) = syn::parse_str::<syn::UseTree>(&tree) else { continue };
        let starts = line_starts(&tree);
        collect_import_sites(&parsed, &mut vec![], None, base, &tree, &starts, &attributes, &mut out);
    }
    Ok(out)
}

/// Each `use` tree in `text`, with where it starts. A container's stored text
/// can stop before its closing brace, so a failed parse is retried closed.
fn container_use_trees(text: &str) -> Result<Vec<(usize, String)>, syn::Error> {
    Ok(container_trees(text)?.1)
}

fn collect_import_sites(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    root: Option<(usize, usize)>,
    base: usize,
    text: &str,
    starts: &[usize],
    attributes: &str,
    out: &mut Vec<ImportSite>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            let at = base + use_span(text, starts, path.ident.span()).start;
            let root = Some(root.unwrap_or((at, prefix.len())));
            prefix.push(path.ident.to_string());
            collect_import_sites(&path.tree, prefix, root, base, text, starts, attributes, out);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let leaf = name.ident.to_string();
            let span = use_span(text, starts, name.ident.span());
            if leaf == "self" {
                let Some(local) = prefix.last().cloned() else { return };
                let (start, outside) = root.unwrap_or((base + span.start, 1));
                out.push(ImportSite {
                    local,
                    path: prefix.clone(),
                    range: start..base + span.end,
                    alias: None,
                    outside,
                    glob: false,
                    attributes: attributes.to_string(),
                });
                return;
            }
            let mut path = prefix.clone();
            path.push(leaf.clone());
            let (start, outside) = root.unwrap_or((base + span.start, prefix.len()));
            out.push(ImportSite {
                local: leaf,
                path,
                range: start..base + span.end,
                alias: None,
                outside,
                glob: false,
                attributes: attributes.to_string(),
            });
        }
        syn::UseTree::Rename(rename) => {
            let leaf = rename.ident.to_string();
            if leaf == "self" {
                return;
            }
            let mut path = prefix.clone();
            path.push(leaf);
            let span = use_span(text, starts, rename.ident.span());
            let (start, outside) = root.unwrap_or((base + span.start, prefix.len()));
            let alias_start = base + byte_at(text, starts, rename.as_token.span.start());
            let alias_end = base + use_span(text, starts, rename.rename.span()).end;
            out.push(ImportSite {
                local: rename.rename.to_string(),
                path,
                range: start..base + span.end,
                alias: Some(alias_start..alias_end),
                outside,
                glob: false,
                attributes: attributes.to_string(),
            });
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_import_sites(item, prefix, None, base, text, starts, attributes, out);
            }
        }
        syn::UseTree::Glob(glob) => {
            let span = use_span(text, starts, glob.star_token.span);
            let (start, outside) = root.unwrap_or((base + span.start, prefix.len()));
            out.push(ImportSite {
                local: "*".to_string(),
                path: prefix.clone(),
                range: start..base + span.end,
                alias: None,
                outside,
                glob: true,
                attributes: attributes.to_string(),
            });
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ImplOwner {
    One(usize),
    Other,
    Ambiguous(Vec<usize>),
}

fn impl_self_type(name: &str) -> Option<Vec<String>> {
    let ty = name.trim();
    let ty = if let Some(rest) = ty.strip_prefix('<') {
        rest.split_once(" as ").map_or_else(|| rest.trim_end_matches('>').trim(), |(ty, _)| ty.trim())
    } else {
        ty
    };
    type_path(&syn::parse_str(ty).ok()?)
}

fn type_path(ty: &syn::Type) -> Option<Vec<String>> {
    match ty {
        syn::Type::Path(p) if p.qself.is_none() => Some(p.path.segments.iter().map(|s| s.ident.to_string()).collect()),
        syn::Type::Reference(r) => type_path(&r.elem),
        syn::Type::Paren(p) => type_path(&p.elem),
        syn::Type::Group(g) => type_path(&g.elem),
        syn::Type::Ptr(p) => type_path(&p.elem),
        syn::Type::Slice(s) => type_path(&s.elem),
        syn::Type::Array(a) => type_path(&a.elem),
        _ => None,
    }
}

fn exact_targets(refs: &Refs, m: &str, i: usize, segs: &[String]) -> Vec<usize> {
    if segs.is_empty() {
        return vec![];
    }
    if segs.len() == 1 {
        return bare_targets(refs, m, i, &segs[0]);
    }
    let mut segs = segs.to_vec();
    if segs[0] == "Self" {
        let Some(name) = refs.container_name(i) else {
            return vec![];
        };
        segs[0] = name;
        return associated_targets(refs, m, &segs);
    }
    let scope = symbol_scope(refs, i);
    let (module, rest) = match segs[0].as_str() {
        "crate" | "self" | "super" => match module_named_in(refs, m, scope, &segs[0]) {
            Some(module) => (module, &segs[1..]),
            None => return vec![],
        },
        head => {
            if let Some(module) = module_named_in(refs, m, scope, head) {
                (module, &segs[1..])
            } else if item_visible(refs, m, i, head) {
                return associated_targets(refs, m, &segs);
            } else {
                return vec![];
            }
        }
    };
    exact_targets_in(refs, &module, rest)
}

fn bare_targets(refs: &Refs, m: &str, i: usize, name: &str) -> Vec<usize> {
    let own = item_targets(refs, refs.locals(m, name));
    if !own.is_empty() {
        return own;
    }
    if let Some(named) = named_imports(refs, m, i, name) {
        return item_targets(refs, named);
    }
    item_targets(refs, refs.import_visible(m, i, name))
}

fn associated_targets(refs: &Refs, m: &str, segs: &[String]) -> Vec<usize> {
    item_targets(refs, refs.names_in(m, &segs.join("::")))
}

fn exact_targets_in(refs: &Refs, module: &str, rest: &[String]) -> Vec<usize> {
    let mut cur = module.to_string();
    let mut k = 0;
    while k < rest.len() {
        if let Some(next) = refs.module_named(&cur, &rest[k]) {
            cur = next;
            k += 1;
        } else {
            break;
        }
    }
    if k == rest.len() {
        return vec![];
    }
    let joined = rest[k..].join("::");
    let targets = if joined.contains("::") { refs.names_in(&cur, &joined) } else { refs.symbols_named(&cur, &joined) };
    item_targets(refs, targets)
}

/// The trait an `impl` header names and its resolved type, when both are exact.
pub(crate) fn impl_trait(syms: &[Sym], modules: &[ModuleRow], i: usize) -> Option<(usize, Vec<String>)> {
    let rest = syms[i].name.trim().strip_prefix('<')?.strip_suffix('>')?;
    let (_, trait_name) = rest.split_once(" as ")?;
    let trait_name = trait_name.trim();
    let segs = type_path(&syn::parse_str(trait_name).ok()?)?;
    let refs = Refs::new(syms, modules);
    let mut targets = exact_targets(&refs, &syms[i].module, i, &segs);
    targets.sort_unstable();
    targets.dedup();
    match targets.as_slice() {
        [one] => Some((*one, segs)),
        _ => None,
    }
}

/// Resolve several impl owners against one reference index.
pub(crate) fn impl_owner_batch(syms: &[Sym], modules: &[ModuleRow], indices: &[usize]) -> Vec<ImplOwner> {
    let refs = Refs::new(syms, modules);
    indices.iter().map(|&i| impl_owner_in(&refs, i)).collect()
}

fn impl_owner_in(refs: &Refs<'_>, i: usize) -> ImplOwner {
    let Some(segs) = impl_self_type(&refs.syms[i].name) else {
        return ImplOwner::Other;
    };
    let mut targets = exact_targets(refs, &refs.syms[i].module, i, &segs);
    targets.sort_unstable();
    targets.dedup();
    match targets.as_slice() {
        [one] => ImplOwner::One(*one),
        [] => ImplOwner::Other,
        many => ImplOwner::Ambiguous(many.to_vec()),
    }
}

fn enclosing_mod<'a>(refs: &Refs<'a>, mut parent: Option<&'a str>) -> Option<usize> {
    while let Some(id) = parent {
        let k = *refs.by_id.get(id)?;
        if refs.syms[k].kind_text == "mod" {
            return Some(k);
        }
        parent = refs.syms[k].parent.as_deref();
    }
    None
}

fn symbol_scope<'a>(refs: &Refs<'a>, i: usize) -> Option<usize> {
    if refs.syms[i].kind_text == "mod" { Some(i) } else { enclosing_mod(refs, refs.syms[i].parent.as_deref()) }
}

fn imports_for<'a, 'b>(refs: &'a Refs<'b>, m: &str, scope: Option<usize>) -> Option<&'a Vec<Import>> {
    match scope {
        Some(k) => refs.container_imports.get(refs.syms[k].id.as_str()),
        None => refs.inline_modules.get(m).map_or_else(|| refs.imports.get(m), |&k| refs.container_imports.get(refs.syms[k].id.as_str())),
    }
}

fn module_named_in(refs: &Refs, m: &str, scope: Option<usize>, name: &str) -> Option<String> {
    match name {
        "crate" => return Some("crate".to_string()),
        "self" => return Some(scope_module(refs, m, scope)),
        "super" => return scope.map_or_else(|| refs.parent_module(m), |k| Some(super_module(refs, m, k))),
        _ => {}
    }
    let current = scope_module(refs, m, scope);
    // The crate root is named `crate` on its own: its children are top level.
    let child = if current == "crate" && refs.module_exists(name) { name.to_string() } else { inline_path(&current, name) };
    if refs.module_exists(&child) {
        return Some(child);
    }
    let imps = imports_for(refs, m, scope)?;
    for imp in imps.iter().filter(|i| !i.glob && i.local == name) {
        if let Some(m2) = refs.resolve_use_module(m, scope, &imp.path) {
            return Some(m2);
        }
    }
    None
}

fn mentioned_names_in(refs: &Refs, syms: &[Sym]) -> Vec<HashSet<String>> {
    syms.iter()
        .enumerate()
        .map(|(i, symbol)| {
            if symbol.kind_text == "use" {
                return HashSet::new();
            }
            let text = &symbol.text;
            let mentions = scanned(text);
            let (attrs, rest) = attrs_and_rest(text);
            let body_start = if attrs.is_empty() { 0 } else { text.len() - rest.len() };
            let context = ScopeContext { refs, module: symbol.module.as_str(), symbol: i, target: None };
            let scopes = scan_scopes(text, refs.container_name(i).is_some(), &context);
            let in_import =
                |range: &std::ops::Range<usize>| scopes.imports.iter().any(|import| import.start <= range.start && range.end <= import.end);
            mentions
                .into_iter()
                .filter(|mention| mention.range.start >= body_start)
                .filter_map(|mention| match mention.mention {
                    Mention::Path(segs) => {
                        let name = segs.first()?.clone();
                        if name == "_" || in_import(&mention.range) || shadowed(&scopes, &segs, mention.range.start) {
                            None
                        } else {
                            Some(name)
                        }
                    }
                    Mention::Macro(name) => Some(name),
                    Mention::Format(name) => {
                        if shadowed(&scopes, std::slice::from_ref(&name), mention.range.start) {
                            None
                        } else {
                            Some(name)
                        }
                    }
                    Mention::Method(_) | Mention::Field(_) => None,
                })
                .collect()
        })
        .collect()
}

pub(crate) struct References<'a> {
    refs: Refs<'a>,
    modules: &'a [ModuleRow],
    pub(crate) mentioned: Vec<HashSet<String>>,
}

pub(crate) fn references<'a>(syms: &'a [Sym], modules: &'a [ModuleRow]) -> References<'a> {
    let refs = Refs::new(syms, modules);
    let mentioned = mentioned_names_in(&refs, syms);
    References { refs, modules, mentioned }
}

pub(crate) fn references_external_imports(refs: &References, module: &str) -> Result<Vec<ImportSite>, Failure> {
    Ok(import_sites(refs.refs.syms, refs.modules, module, true)?
        .into_iter()
        .filter(|site| !local_path(refs.modules, &site.path) && block_use_targets(&refs.refs, module, None, &site.path).is_none())
        .collect())
}

/// The traits `module` imports by name, with the import that binds each.
pub(crate) fn imported_traits(refs: &References, module: &str) -> Result<Vec<(usize, ImportSite)>, Failure> {
    let mut out = vec![];
    for site in import_sites(refs.refs.syms, refs.modules, module, true)? {
        if site.glob {
            continue;
        }
        let Some((m, name)) = refs.refs.resolve_use(module, None, &site.path) else { continue };
        for i in refs.refs.symbols_named(&m, &name) {
            if refs.refs.syms[i].kind_text == "trait"
                && !out.iter().any(|(j, s): &(usize, ImportSite)| *j == i && s.path == site.path && s.local == site.local)
            {
                out.push((i, site.clone()));
            }
        }
    }
    Ok(out)
}

/// Whether `trait_i` declares any of `methods`.
pub(crate) fn trait_has_method(refs: &References, trait_i: usize, methods: &HashSet<String>) -> bool {
    refs.refs
        .children
        .get(refs.refs.syms[trait_i].id.as_str())
        .is_some_and(|kids| kids.iter().any(|&k| methods.contains(leaf(&refs.refs.syms[k].name))))
}

/// `syms[i]`'s text with unshadowed mentions of `from` renamed to `to`: a path's
/// first segment or a macro call. Imports inside the item keep their own names.
pub(crate) fn rename_name(refs: &References, i: usize, from: &str, to: &str) -> String {
    let names = HashMap::from([(from.to_string(), to.to_string())]);
    rename_names(refs, i, &refs.refs.syms[i].text, &names)
}

fn inline_path(module: &str, name: &str) -> String {
    format!("{module}::{name}")
}

fn name_scope<'a>(refs: &'a Refs<'_>, m: &'a str) -> (&'a str, Option<&'a str>) {
    refs.inline_modules.get(m).map_or((m, None), |&k| {
        let s = &refs.syms[k];
        (s.module.as_str(), Some(s.name.as_str()))
    })
}

fn scope_module(refs: &Refs, m: &str, scope: Option<usize>) -> String {
    scope.map_or_else(|| m.to_string(), |k| inline_path(m, &refs.syms[k].name))
}

fn super_module(refs: &Refs, m: &str, scope: usize) -> String {
    refs.syms[scope].name.rsplit_once("::").map_or_else(|| m.to_string(), |(parent, _)| inline_path(m, parent))
}

pub(crate) fn rewrite_sites_refs(refs: &References, target: usize) -> Vec<Site> {
    rewrite_sites_in(&refs.refs, refs.refs.syms, target)
}

fn rewrite_sites_in(refs: &Refs, syms: &[Sym], target: usize) -> Vec<Site> {
    let mut out = vec![];
    for (i, symbol) in syms.iter().enumerate() {
        if symbol.kind_text == "use" {
            continue;
        }
        let text = &symbol.text;
        let mentions = scanned(text);
        let context = ScopeContext { refs: &refs, module: symbol.module.as_str(), symbol: i, target: Some(target) };
        let (bound, sites) = bound(text, &mentions, refs.container_name(i).is_some(), &context);
        out.extend(sites);
        for mention in mentions {
            if bound.contains(&mention.range) {
                continue;
            }
            match &mention.mention {
                Mention::Path(segs) => out.extend(path_site(&refs, i, &mention.range, segs, target, false)),
                Mention::Format(name) => out.extend(format_site(&refs, i, &mention.range, name).filter(|site| site.target == target)),
                _ => {}
            }
        }
    }
    out
}

#[derive(Clone, Copy)]
pub(crate) enum CallKind {
    Method,
    Macro,
}

/// The method or macro names `text` calls, for finding the imports they need.
pub(crate) fn called_names(text: &str, kind: CallKind) -> HashSet<String> {
    scanned(text)
        .into_iter()
        .filter_map(|m| match (kind, m.mention) {
            (CallKind::Method, Mention::Method(name)) | (CallKind::Macro, Mention::Macro(name)) => Some(name),
            _ => None,
        })
        .collect()
}

fn generic_binding(trees: &[TokenTree], i: usize) -> bool {
    let after_open = previous_punct(trees, i, '<') || previous_punct(trees, i, ',');
    let before_eq = matches!(trees.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == '=' && p.spacing() != Spacing::Joint);
    after_open && before_eq
}

/// The methods a standard trait provides when the trait is not a store symbol.
pub(crate) fn std_trait_methods(path: &[String]) -> Option<&'static [&'static str]> {
    match path.join("::").as_str() {
        "std::io::Read" => Some(&["read", "read_to_string", "read_to_end", "read_exact"][..]),
        "std::io::BufRead" => Some(&["fill_buf", "consume", "read_until", "read_line"][..]),
        "std::io::Write" => Some(&["write", "write_all", "write_vectored", "write_fmt", "flush"][..]),
        "std::fmt::Write" => Some(&["write_str", "write_char", "write_fmt"][..]),
        "std::io::Seek" => Some(&["seek", "rewind", "stream_position", "stream_len"][..]),
        _ => None,
    }
}

/// A mention's exact targets and the targets only its possible reading names.
fn classify(refs: &Refs, m: &str, i: usize, mention: &Spanned, scopes: &Scopes) -> (Vec<usize>, Vec<usize>) {
    let (exact, mut possible) = match &mention.mention {
        Mention::Path(segs) => {
            if shadowed(scopes, segs, mention.range.start) {
                return (vec![], vec![]);
            }
            if segs.len() == 1 {
                let targets = bare_candidates(refs, i, m, &segs[0]);
                if targets.len() == 1 { (targets, vec![]) } else { (vec![], targets) }
            } else if let Some((target, _)) = resolve_exact(refs, m, i, segs) {
                (vec![target], vec![])
            } else if let Some(target) = prefix_exact(refs, m, i, segs) {
                (vec![target], refs.resolve(m, i, &mention.mention).into_iter().filter(|&t| t != target).collect())
            } else {
                (vec![], refs.resolve(m, i, &mention.mention))
            }
        }
        Mention::Format(name) => {
            if shadowed(scopes, std::slice::from_ref(name), mention.range.start) {
                return (vec![], vec![]);
            }
            let targets = bare_candidates(refs, i, m, name);
            if targets.len() == 1 { (targets, vec![]) } else { (vec![], targets) }
        }
        Mention::Macro(_) | Mention::Field(_) => (vec![], refs.resolve(m, i, &mention.mention)),
        Mention::Method(name) => (vec![], method_targets(refs, name)),
    };
    let mut expanded = exact.clone();
    expanded.extend(possible.iter().copied());
    refs.expand_types(&mut expanded);
    for target in expanded {
        if !exact.contains(&target) && !possible.contains(&target) {
            possible.push(target);
        }
    }
    (exact, possible)
}

/// A bare name's targets, with a test module's sibling hiding its parent's.
fn bare_candidates(refs: &Refs, i: usize, m: &str, name: &str) -> Vec<usize> {
    let siblings = refs.test_siblings(i, name);
    if siblings.is_empty() { bare_targets(refs, m, i, name) } else { item_targets(refs, siblings) }
}

/// The methods and associated functions a method call `.name()` may name.
fn method_targets(refs: &Refs, name: &str) -> Vec<usize> {
    refs.leaves
        .get(name)
        .map(|items| items.iter().copied().filter(|&i| refs.syms[i].kind_text == "fn" && refs.container_name(i).is_some()).collect())
        .unwrap_or_default()
}

/// The statics a `thread_local!` item declares, in order: its name, the byte
/// range of its `static` keyword, and whether it is already visible.
pub(crate) fn thread_local_names(text: &str) -> Vec<(String, std::ops::Range<usize>, bool)> {
    let Ok(item) = syn::parse_str::<syn::ItemMacro>(text) else { return vec![] };
    if !item.mac.path.segments.last().is_some_and(|s| s.ident == "thread_local") {
        return vec![];
    }
    let mut names = vec![];
    let mut decl = proc_macro2::TokenStream::new();
    for tree in item.mac.tokens {
        let end = matches!(&tree, TokenTree::Punct(p) if p.as_char() == ';');
        decl.extend(std::iter::once(tree));
        if end {
            if let Ok(stat) = syn::parse2::<syn::ItemStatic>(decl) {
                names.push((
                    stat.ident.to_string(),
                    syn::spanned::Spanned::span(&stat.static_token).byte_range(),
                    !matches!(stat.vis, syn::Visibility::Inherited),
                ));
            }
            decl = proc_macro2::TokenStream::new();
        }
    }
    names
}

/// `text` with unshadowed mentions of each name renamed to its replacement.
pub(crate) fn rename_names(refs: &References, i: usize, text: &str, names: &HashMap<String, String>) -> String {
    let symbol = &refs.refs.syms[i];
    let mentions = scanned(text);
    let context = ScopeContext { refs: &refs.refs, module: symbol.module.as_str(), symbol: i, target: None };
    let scopes = scan_scopes(text, refs.refs.container_name(i).is_some(), &context);
    let in_import =
        |range: &std::ops::Range<usize>| scopes.imports.iter().any(|import| import.start <= range.start && range.end <= import.end);
    let mut edits: Vec<(std::ops::Range<usize>, String)> = mentions
        .into_iter()
        .filter_map(|mention| match &mention.mention {
            Mention::Path(segs) if segs.first().is_some_and(|seg| names.contains_key(seg)) => {
                if in_import(&mention.range) || shadowed(&scopes, segs, mention.range.start) {
                    None
                } else {
                    let from = segs.first()?;
                    Some((mention.range.start..mention.range.start + from.len(), names[from].clone()))
                }
            }
            Mention::Macro(name) => names.get(name).map(|to| (mention.range.clone(), to.clone())),
            _ => None,
        })
        .collect();
    edits.sort_by(|a, b| b.0.start.cmp(&a.0.start));
    let mut out = text.to_string();
    for (range, to) in edits {
        out.replace_range(range, &to);
    }
    out
}

/// The module a `use` path names from `from`, when it names one in this crate.
pub(crate) fn import_module(refs: &References, from: &str, path: &[String]) -> Option<String> {
    refs.refs.resolve_use_module(from, None, path)
}

/// The argument a macro reads its format string from.
fn format_arg_index(name: &str) -> Option<usize> {
    match name {
        "format" | "format_args" | "print" | "println" | "eprint" | "eprintln" | "panic" | "unreachable" | "todo" | "unimplemented" => {
            Some(0)
        }
        "write" | "writeln" | "assert" | "debug_assert" => Some(1),
        "assert_eq" | "assert_ne" | "debug_assert_eq" | "debug_assert_ne" => Some(2),
        _ => None,
    }
}

/// One top-level argument of a macro's group.
fn group_arg(group: &proc_macro2::Group, index: usize) -> Option<TokenTree> {
    let trees: Vec<TokenTree> = group.stream().into_iter().collect();
    let mut arg = 0;
    let mut start = 0;
    for (i, tree) in trees.iter().enumerate() {
        if matches!(tree, TokenTree::Punct(p) if p.as_char() == ',') {
            if arg == index {
                return trees.get(start).cloned();
            }
            arg += 1;
            start = i + 1;
        }
    }
    if arg == index { trees.get(start).cloned() } else { None }
}

/// The names a format string captures, with their ranges inside its literal.
fn format_captures(tree: &TokenTree, text: &str, starts: &[usize], named: &[String], out: &mut Vec<Spanned>) {
    let TokenTree::Literal(_) = tree else { return };
    let range = token_range(tree, text, starts);
    let Some(source) = text.get(range.clone()) else { return };
    let Some(first) = source.as_bytes().first() else { return };
    if *first != b'"' && *first != b'r' {
        return;
    }
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' if *first == b'"' && follows_unicode_escape(bytes, i) => i += 1,
            b'{' if bytes.get(i + 1) == Some(&b'{') => i += 2,
            b'{' => {
                let Some(close) = source[i + 1..].find('}') else { break };
                let body = &source[i + 1..i + 1 + close];
                let end = body.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(body.len());
                let name = &body[..end];
                let next = body.as_bytes().get(end).copied();
                if !name.is_empty()
                    && !name.as_bytes()[0].is_ascii_digit()
                    && matches!(next, None | Some(b':') | Some(b'.') | Some(b'['))
                    && !named.iter().any(|n| n.as_str() == name)
                {
                    let start = range.start + i + 1;
                    let line = starts.iter().take_while(|&&s| s <= start).count().saturating_sub(1);
                    out.push(Spanned { mention: Mention::Format(name.to_string()), line, range: start..start + name.len() });
                }
                i += close + 2;
            }
            b'}' if bytes.get(i + 1) == Some(&b'}') => i += 2,
            _ => i += 1,
        }
    }
}

/// The items a module's named re-exports bind to `name`.
fn reexport_targets(refs: &Refs, module: &str, name: &str) -> Vec<usize> {
    let Some(imps) = imports_for(refs, module, None) else { return vec![] };
    let targets: Vec<usize> = imps
        .iter()
        .filter(|imp| !imp.glob && imp.local == name)
        .filter_map(|imp| refs.resolve_use(module, None, &imp.path))
        .flat_map(|(m, n)| refs.symbols_named(&m, &n))
        .collect();
    item_targets(refs, targets)
}

/// The item the longest resolving prefix of a path names.
fn prefix_exact(refs: &Refs, m: &str, i: usize, segs: &[String]) -> Option<usize> {
    (2..segs.len()).rev().find_map(|k| resolve_exact(refs, m, i, &segs[..k]).map(|(target, _)| target))
}

fn format_site(refs: &Refs, i: usize, range: &std::ops::Range<usize>, name: &str) -> Option<Site> {
    let segs = vec![name.to_string()];
    let (target, via) = resolve_exact(refs, &refs.syms[i].module, i, &segs)?;
    Some(Site { range: range.clone(), alias: None, segs, outside: 0, target, via, symbol: Some(i), format: true })
}

/// The names a format-like macro's named arguments supply: a slot with one of
/// these names does not capture an item from the enclosing scope.
fn named_format_args(group: &proc_macro2::Group, format_arg: usize) -> Vec<String> {
    let trees: Vec<TokenTree> = group.stream().into_iter().collect();
    let mut out = vec![];
    let mut arg = 0;
    for (i, tree) in trees.iter().enumerate() {
        match tree {
            TokenTree::Punct(p) if p.as_char() == ',' => arg += 1,
            TokenTree::Ident(id) if arg > format_arg => {
                if matches!(trees.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == '=' && p.spacing() != Spacing::Joint) {
                    out.push(id.to_string());
                }
            }
            _ => {}
        }
    }
    out
}

/// Whether the `{` at `at` opens a Unicode escape such as `\u{e9}` in a
/// normal string literal.
fn follows_unicode_escape(bytes: &[u8], at: usize) -> bool {
    if at < 2 || bytes[at - 1] != b'u' {
        return false;
    }
    let mut backslashes = 0;
    let mut i = at - 2;
    while bytes.get(i) == Some(&b'\\') {
        backslashes += 1;
        if i == 0 {
            break;
        }
        i -= 1;
    }
    backslashes % 2 == 1
}

/// The modules that hold test code (D-170): the `tests/` folder, and every
/// module declared under `#[cfg(test)]`, with everything below it.
fn test_modules(syms: &[Sym], modules: &[ModuleRow]) -> HashSet<String> {
    let attrs: Vec<Vec<String>> = syms.iter().map(|s| outer_attrs(&s.text)).collect();
    let mut declared: HashSet<(String, String)> = HashSet::new();
    for m in modules {
        header_cfg_mods(&m.header, &m.path, &mut declared);
    }
    for (i, s) in syms.iter().enumerate() {
        if s.kind_text == "mod" && s.text.trim_end().ends_with(';') && cfg_test_in(&attrs[i]) {
            declared.insert((s.module.clone(), leaf(&s.name).to_string()));
        }
    }
    let mut test_mods: HashSet<String> = modules.iter().filter(|m| m.dir == "tests").map(|m| m.path.clone()).collect();
    loop {
        let mut added = false;
        for m in modules {
            if test_mods.contains(&m.path) {
                continue;
            }
            let name = m.path.rsplit("::").next().unwrap_or(&m.path).to_string();
            let parent = m.path.rsplit_once("::").map_or("crate", |(p, _)| p).to_string();
            if declared.contains(&(parent.clone(), name)) || test_mods.contains(&parent) {
                test_mods.insert(m.path.clone());
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    test_mods
}

/// Whether an import path starts at the crate root, a parent module, or a
/// module this project holds, so it names no other crate.
fn local_path(modules: &[ModuleRow], path: &[String]) -> bool {
    path.first()
        .is_some_and(|first| matches!(first.as_str(), "crate" | "self" | "super") || modules.iter().any(|module| module.path == *first))
}

/// The bytes of a block comment's body, `after_open` being the text past its
/// `/*`: through the matching `*/`, nested comments included.
pub(crate) fn block_comment_len(after_open: &str) -> usize {
    let b = after_open.as_bytes();
    let (mut depth, mut i) = (1i32, 0);
    while i < b.len() && depth > 0 {
        if b[i..].starts_with(b"/*") {
            depth += 1;
            i += 2;
        } else if b[i..].starts_with(b"*/") {
            depth -= 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    i
}

impl ImportSite {
    /// An import the extract writes rather than one read from a source place.
    pub(crate) fn named(path: Vec<String>) -> ImportSite {
        let local = path.last().cloned().unwrap_or_default();
        ImportSite { local, path, range: 0..0, alias: None, outside: 0, glob: false, attributes: String::new() }
    }
}

/// Each `use` tree with its start and the outer attributes written before it.
fn with_attributes(source: &str, trees: Vec<(usize, String)>) -> Vec<(usize, String, String)> {
    let ranges = crate::language::rust::import_ranges(source).unwrap_or_default();
    trees
        .into_iter()
        .map(|(base, tree)| {
            let range = base..base + tree.len();
            let attributes = ranges
                .iter()
                .find(|(_, tree_range)| tree_range.start <= range.start && range.end <= tree_range.end)
                .map_or_else(String::new, |(item, tree_range)| attrs_and_rest(&source[item.start..tree_range.start]).0.join("\n"));
            (base, tree, attributes)
        })
        .collect()
}

/// Each `use` tree in `text`, with where it starts and the outer attributes
/// written before it.
fn container_use_trees_with_attributes(text: &str) -> Result<Vec<(usize, String, String)>, syn::Error> {
    let (closed, trees) = container_trees(text)?;
    Ok(with_attributes(closed.as_deref().unwrap_or(text), trees))
}

/// The `use` trees of `text` with where they start, and the closed text when
/// a parse needed its missing brace added.
fn container_trees(text: &str) -> Result<(Option<String>, Vec<(usize, String)>), syn::Error> {
    match use_trees(text) {
        Ok(trees) => Ok((None, use_tree_texts(trees))),
        Err(_) => {
            let closed = format!("{text}\n}}");
            let trees = use_tree_texts(use_trees(&closed)?);
            Ok((Some(closed), trees))
        }
    }
}

fn use_tree_texts(trees: Vec<(usize, &str)>) -> Vec<(usize, String)> {
    trees.into_iter().map(|(base, tree)| (base, tree.to_string())).collect()
}
