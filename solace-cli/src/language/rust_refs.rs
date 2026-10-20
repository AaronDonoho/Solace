//! Rust references (D-170): the names an item's text mentions, resolved
//! against its module's `use` lines and the crate's item names, so a Rust
//! store carries the same kind of reference graph Gleam's does.
//!
//! Names are read from the text's tokens, not with a `syn` visitor: a call
//! inside `assert!(…)` or `format!(…)` is a token in a macro's arguments, so a
//! visitor over the syntax tree would miss the tests it sits in. Resolution
//! overestimates on purpose — a method name links to every item that has it,
//! an unresolved path start to every item with its last name — so test reach
//! never misses a test.

use crate::{ModuleRow, Sym};
use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};
use std::collections::{HashMap, HashSet};
use syn::Meta;

/// A Rust store's reference graph: what each symbol uses, its reverse, each
/// reference's line, and which symbols are test code.
pub struct Graph {
    pub uses: Vec<Vec<usize>>,
    pub used_by: Vec<Vec<usize>>,
    pub at: Vec<Vec<(usize, usize)>>,
    pub test: Vec<bool>,
}

/// Builds the graph of `syms`, named as `modules` names them.
pub fn graph(syms: &[Sym], modules: &[ModuleRow]) -> Graph {
    let refs = Refs::new(syms, modules);
    let mut at: Vec<Vec<(usize, usize)>> = vec![vec![]; syms.len()];
    let mut uses: Vec<Vec<usize>> = vec![vec![]; syms.len()];
    for i in 0..syms.len() {
        // A `use` statement's own text names what it imports: that is an
        // import, not a use of the item.
        if syms[i].kind_text == "use" {
            continue;
        }
        let m = syms[i].module.as_str();
        for (mention, line) in mentions(&syms[i].text) {
            for t in refs.resolve(m, i, &mention) {
                if t != i && !at[i].contains(&(t, line)) {
                    at[i].push((t, line));
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
    Graph { uses, used_by, at, test: test_flags(syms, modules) }
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
fn attrs_and_rest(text: &str) -> (Vec<&str>, &str) {
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
            let b = r.as_bytes();
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
            s = &r[i..];
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
enum Mention {
    /// `a::b::c`, and a bare name as a one-segment path.
    Path(Vec<String>),
    /// A method call `.f(`.
    Method(String),
    /// A macro call `name!`.
    Macro(String),
}

/// The names `text` mentions, each with its 0-based line in `text`.
fn mentions(text: &str) -> Vec<(Mention, usize)> {
    let Ok(stream) = text.parse::<TokenStream>() else { return vec![] };
    let mut out = vec![];
    scan(&stream, &mut out);
    out
}

fn scan(stream: &TokenStream, out: &mut Vec<(Mention, usize)>) {
    let trees: Vec<TokenTree> = stream.clone().into_iter().collect();
    let mut i = 0;
    let mut after_dot = false;
    while i < trees.len() {
        match &trees[i] {
            // A macro's arguments are a group: their calls are mentions too.
            TokenTree::Group(g) => {
                scan(&g.stream(), out);
                after_dot = false;
                i += 1;
            }
            TokenTree::Punct(p) => {
                after_dot = p.as_char() == '.' && p.spacing() != Spacing::Joint;
                i += 1;
            }
            TokenTree::Literal(_) => {
                after_dot = false;
                i += 1;
            }
            TokenTree::Ident(id) => {
                let line = id.span().start().line.saturating_sub(1);
                let dot = after_dot;
                let mut segs = vec![id.to_string()];
                let mut j = i + 1;
                while j + 2 < trees.len() && is_path_sep(&trees[j], &trees[j + 1]) {
                    if let TokenTree::Ident(n) = &trees[j + 2] {
                        segs.push(n.to_string());
                        j += 3;
                    } else {
                        break;
                    }
                }
                let call = matches!(trees.get(j), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis);
                let macro_call = segs.len() == 1
                    && matches!(trees.get(j), Some(TokenTree::Punct(p)) if p.as_char() == '!' && p.spacing() != Spacing::Joint);
                if macro_call {
                    out.push((Mention::Macro(segs[0].clone()), line));
                    j += 1;
                } else if segs.len() > 1 {
                    out.push((Mention::Path(segs.clone()), line));
                } else if dot && call {
                    out.push((Mention::Method(segs[0].clone()), line));
                } else {
                    out.push((Mention::Path(segs.clone()), line));
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
    names: HashMap<&'a str, HashMap<&'a str, Vec<usize>>>,
    heads: HashMap<&'a str, HashMap<&'a str, Vec<usize>>>,
    imports: HashMap<&'a str, Vec<Import>>,
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
            names: HashMap::new(),
            heads: HashMap::new(),
            imports: HashMap::new(),
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
        for m in modules {
            let mut imports = header_uses(&m.header);
            for s in syms.iter().filter(|s| s.module == m.path && s.kind_text == "use") {
                imports.extend(parse_use(&s.name));
            }
            r.imports.insert(m.path.as_str(), imports);
        }
        r
    }

    fn module_exists(&self, m: &str) -> bool {
        self.module_paths.contains(m)
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
        match name {
            "crate" => return Some("crate".to_string()),
            "self" => return Some(m.to_string()),
            "super" => return self.parent_module(m),
            _ => {}
        }
        // The crate root is named `crate` on its own: its children are top level.
        let child = if m == "crate" { name.to_string() } else { format!("{m}::{name}") };
        if self.module_exists(&child) {
            return Some(child);
        }
        if self.module_exists(name) {
            return Some(name.to_string());
        }
        let imps = self.imports.get(m)?;
        for imp in imps.iter().filter(|i| !i.glob && i.local == name) {
            if let Some(m2) = self.resolve_use_module(m, &imp.path) {
                return Some(m2);
            }
        }
        None
    }

    /// The module a `use` path names (`super::*` -> the parent module).
    fn resolve_use_module(&self, m: &str, path: &[String]) -> Option<String> {
        let (first, rest) = path.split_first()?;
        let mut cur = self.module_named(m, first)?;
        for seg in rest {
            cur = self.module_named(&cur, seg)?;
        }
        Some(cur)
    }

    /// The module and item name a `use` path names.
    fn resolve_use(&self, m: &str, path: &[String]) -> Option<(String, String)> {
        let (first, rest) = path.split_first()?;
        if rest.is_empty() {
            return None;
        }
        // An unknown first segment may be another crate (the library's name in
        // an integration test): resolve the rest from this crate's root.
        let mut cur = match first.as_str() {
            "crate" | "self" | "super" => self.module_named(m, first)?,
            other => self.module_named(m, other).unwrap_or_else(|| "crate".to_string()),
        };
        for seg in &rest[..rest.len() - 1] {
            cur = self.module_named(&cur, seg)?;
        }
        Some((cur, rest.last()?.clone()))
    }

    fn symbols_named(&self, module: &str, name: &str) -> Vec<usize> {
        let mut out = vec![];
        if let Some(v) = self.heads.get(module).and_then(|h| h.get(name)) {
            out.extend(v);
        }
        if let Some(v) = self.names.get(module).and_then(|h| h.get(name)) {
            out.extend(v);
        }
        out
    }

    fn locals(&self, m: &str, name: &str) -> Vec<usize> {
        self.heads.get(m).and_then(|h| h.get(name)).cloned().unwrap_or_default()
    }

    /// The symbols of `m` whose name is `joined` or ends with `::joined`.
    fn names_in(&self, m: &str, joined: &str) -> Vec<usize> {
        let Some(map) = self.names.get(m) else { return vec![] };
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
    fn import_visible(&self, m: &str, name: &str) -> Vec<usize> {
        let mut out = vec![];
        let mut seen = HashSet::new();
        self.import_visible_in(m, name, &mut seen, &mut out);
        out
    }

    fn import_visible_in(&self, m: &str, name: &str, seen: &mut HashSet<String>, out: &mut Vec<usize>) {
        if !seen.insert(m.to_string()) {
            return;
        }
        let Some(imps) = self.imports.get(m) else { return };
        for imp in imps {
            if imp.glob {
                if let Some(g) = self.resolve_use_module(m, &imp.path) {
                    out.extend(self.symbols_named(&g, name));
                    self.import_visible_in(&g, name, seen, out);
                }
            } else if imp.local == name
                && let Some((m2, n)) = self.resolve_use(m, &imp.path)
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
        let (start, rest): (String, Vec<String>) = match segs[0].as_str() {
            "crate" => ("crate".to_string(), segs[1..].to_vec()),
            "self" => (m.to_string(), segs[1..].to_vec()),
            "super" => match self.parent_module(m) {
                Some(p) => (p, segs[1..].to_vec()),
                None => return self.fallback(&segs),
            },
            _ => (m.to_string(), segs.clone()),
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
            for idx in self.locals(&cur, &head).into_iter().chain(self.import_visible(&cur, &head)) {
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
            Mention::Path(segs) if segs.len() == 1 => {
                let mut v = self.locals(m, &segs[0]);
                v.extend(self.import_visible(m, &segs[0]));
                v
            }
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
}

/// Whether each symbol is test code (D-170): the `tests/` folder, anything
/// under a `#[cfg(test)]` module, and `#[test]` functions.
fn test_flags(syms: &[Sym], modules: &[ModuleRow]) -> Vec<bool> {
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
