//! Rust behind the language seam (D-158): a file split into items with `syn`,
//! each item's text cut from the source by span, so comments and formatting
//! are kept and the pieces join back to the file exactly. `syn` is also the
//! syntax check, and the split lists the code outside the subset Solace holds.

use crate::Failure;
use crate::language::Kind;
use proc_macro2::Span;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::ops::Range;
use std::path::Path;
use syn::spanned::Spanned;
use syn::{Attribute, ForeignItem, ImplItem, Item, TraitItem};

/// A file: its header (inner docs, and the imports and `mod x;` declarations
/// before the first item), its items, and what Solace refuses in it.
pub struct File {
    pub header: String,
    /// Every top-level `mod x;`, in the header or an item, with its `#[path]` file.
    pub mods: Vec<(String, Option<String>)>,
    pub items: Vec<Piece>,
    pub refusals: Vec<Refusal>,
    /// Whether this file's `mod x;` children sit beside it rather than in a
    /// folder named after it: a crate root, `mod.rs`, a `tests/*.rs` crate root
    /// or a `#[path]` file. The store keeps what `walk` found, so a `submit`
    /// places a `#[path]` module's child where the importer read it (D-168).
    pub owns_dir: bool,
}

/// An item, from its leading comments to the next item's.
/// Names are unique except a container's: two `impl A` blocks are both `A`;
/// their items are the symbols.
pub struct Piece {
    pub kind: &'static str,
    /// Within the file: `open`, `Store::open`, `tests::opens`, `<Failure as fmt::Display>::fmt`.
    pub name: String,
    /// The item itself in the source: from its first attribute or doc to its end.
    pub item: Range<usize>,
    /// Where the item's own text starts, after its attributes, docs and comments.
    pub head: usize,
    /// The file a `#[path]` module names.
    pub file: Option<String>,
    /// A container's text up to its first child; any other item's whole text.
    pub text: String,
    pub children: Vec<Piece>,
    /// A container's text after its last child.
    pub tail: String,
}

/// Code outside the subset (D-158 point 2): the line, quoted, and the rewrite.
pub struct Refusal {
    pub line: usize,
    pub quote: String,
    pub message: String,
}

impl File {
    pub fn join(&self) -> String {
        let mut out = self.header.clone();
        self.items.iter().for_each(|p| p.join(&mut out));
        out
    }
}

impl Piece {
    fn join(&self, out: &mut String) {
        out.push_str(&self.text);
        self.children.iter().for_each(|p| p.join(out));
        out.push_str(&self.tail);
    }
}

/// The kind an item of `kind` takes its id prefix from.
pub fn id_kind(kind: &str) -> Kind {
    match kind {
        "struct" | "enum" | "union" | "type" | "trait" => Kind::Type,
        "const" | "static" => Kind::Const,
        _ => Kind::Fn,
    }
}

/// The items of `source`, the file of `module` (`board::agents`, `crate`).
pub fn split(source: &str, module: &str) -> Result<File, Failure> {
    let file = split_parsed(source, module);
    // Spans are kept per thread for every parse; `serve` parses for as long as it runs.
    proc_macro2::extra::invalidate_current_thread_spans();
    file
}

fn split_parsed(source: &str, module: &str) -> Result<File, Failure> {
    // `syn` drops a BOM and a `#!` line before parsing, so spans count from after
    // them; the parse starts there, and the header keeps them. The `#!` line's
    // newline stays in the body, so line numbers still match the file.
    let prefix = prefix_len(source);
    let body = &source[prefix..];
    let ast = syn::parse_file(body).map_err(|e| {
        let line = e.span().start().line;
        Failure(format!("line {line}: {e}: `{}`", quote(body, line)), 1)
    })?;
    let mut cx = Cx { source: body, refusals: vec![] };
    let nodes: Vec<Node> = ast.items.iter().map(|i| cx.node(i, "", module)).collect();
    cx.duplicates(&nodes, module, "");
    let mods = nodes.iter().filter(|n| n.kind == "mod" && n.children.is_none()).map(|n| (n.name.clone(), n.file.clone())).collect();
    let (first, mut items) = cut(body, &nodes, 0, body.len(), true);
    shift(&mut items, prefix);
    Ok(File { header: source[..prefix + first].to_string(), mods, items, refusals: cx.refusals, owns_dir: false })
}

/// The bytes of a leading BOM and `#!` line (not an inner `#![attribute]`).
fn prefix_len(source: &str) -> usize {
    let bom = if source.starts_with('\u{feff}') { 3 } else { 0 };
    let rest = &source[bom..];
    match rest.strip_prefix("#!") {
        Some(r) if !r.trim_start().starts_with('[') => bom + rest.find('\n').unwrap_or(rest.len()),
        _ => bom,
    }
}

fn shift(pieces: &mut [Piece], by: usize) {
    for p in pieces {
        p.item = p.item.start + by..p.item.end + by;
        p.head += by;
        shift(&mut p.children, by);
    }
}

fn quote(source: &str, line: usize) -> String {
    source.lines().nth(line.saturating_sub(1)).unwrap_or("").trim().to_string()
}

/// An item as parsed, before its text is cut.
struct Node {
    kind: &'static str,
    name: String,
    item: Range<usize>,
    head: usize,
    file: Option<String>,
    /// Where its name is, for a refusal's line.
    at: usize,
    cfgs: Vec<String>,
    children: Option<Vec<Node>>,
}

enum Kids<'a> {
    None,
    Impl(&'a [ImplItem]),
    Trait(&'a [TraitItem]),
    Foreign(&'a [ForeignItem]),
    Mod(&'a [Item]),
}

struct Cx<'a> {
    source: &'a str,
    refusals: Vec<Refusal>,
}

fn within(prefix: &str, name: &str) -> String {
    if prefix.is_empty() { name.to_string() } else { format!("{prefix}::{name}") }
}

impl Cx<'_> {
    /// A node's source text, its whitespace runs as single spaces.
    fn text(&self, t: &impl Spanned) -> String {
        self.source[t.span().byte_range()].split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn refuse(&mut self, at: usize, message: String) {
        let line = self.source[..at].matches('\n').count() + 1;
        self.refusals.push(Refusal { line, quote: quote(self.source, line), message });
    }

    fn leaf(&self, kind: &'static str, name: String, attrs: &[Attribute], at: Span, whole: &impl Spanned) -> Node {
        let cfgs = attrs.iter().filter(|a| a.path().is_ident("cfg")).map(|a| self.text(a)).collect();
        let item = whole.span().byte_range();
        let after = attrs.iter().map(|a| a.span().byte_range().end).fold(item.start, usize::max);
        Node { kind, name, head: past_comments(self.source, after), item, at: at.byte_range().start, cfgs, children: None, file: None }
    }

    fn node(&mut self, item: &Item, prefix: &str, module: &str) -> Node {
        let (kind, name, attrs, at, kids): (&'static str, String, &[Attribute], Span, Kids) = match item {
            Item::Const(i) => ("const", i.ident.to_string(), &i.attrs, i.ident.span(), Kids::None),
            Item::Enum(i) => ("enum", i.ident.to_string(), &i.attrs, i.ident.span(), Kids::None),
            Item::ExternCrate(i) => ("extern crate", i.ident.to_string(), &i.attrs, i.ident.span(), Kids::None),
            Item::Fn(i) => ("fn", i.sig.ident.to_string(), &i.attrs, i.sig.ident.span(), Kids::None),
            Item::ForeignMod(i) => ("extern", self.text(&i.abi), &i.attrs, i.abi.span(), Kids::Foreign(&i.items)),
            Item::Impl(i) => {
                let ty = self.text(&*i.self_ty);
                let name = match &i.trait_ {
                    Some((path, _)) => {
                        let not = if self.source[..path.span().byte_range().start].trim_end().ends_with('!') { "!" } else { "" };
                        format!("<{ty} as {not}{}>", self.text(path))
                    }
                    None => ty,
                };
                ("impl", name, &i.attrs, i.self_ty.span(), Kids::Impl(&i.items))
            }
            Item::Macro(i) => {
                let path = self.text(&i.mac.path);
                let at = i.mac.path.span();
                match (i.mac.path.segments.last().map(|s| s.ident.to_string()).as_deref(), &i.ident) {
                    (Some("macro_rules"), Some(id)) => ("macro_rules!", id.to_string(), &i.attrs, id.span(), Kids::None),
                    (Some("thread_local"), _) => {
                        let tokens = self.text(&i.mac.tokens);
                        let mut words = tokens.split_whitespace().skip_while(|w| *w != "static").skip(1);
                        let first: String = words.next().unwrap_or("").chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                        ("thread_local!", first, &i.attrs, at, Kids::None)
                    }
                    (Some("include"), _) => {
                        let args = self.text(&i.mac.tokens);
                        self.refuse(
                            at.byte_range().start,
                            format!("Not saved: `include!({args})` pastes in code Solace doesn't hold. Make it a module (`mod x;`); for data, use `include_str!(\"x.txt\")`."),
                        );
                        ("macro", path, &i.attrs, at, Kids::None)
                    }
                    _ => {
                        self.refuse(
                            at.byte_range().start,
                            format!("Not saved: `{path}! {{ … }}` at module level makes items Solace can't hold as symbols. Write them out: `static X: LazyLock<T> = LazyLock::new(|| …);`."),
                        );
                        ("macro", path, &i.attrs, at, Kids::None)
                    }
                }
            }
            Item::Mod(i) => {
                for (at, v) in mod_paths(&i.attrs) {
                    if v.contains(['/', '\\']) || !v.ends_with(".rs") {
                        self.refuse(
                            at,
                            format!("Not saved: `#[path = \"{v}\"]` is outside its parent's folder. A `#[path]` module is a `.rs` file beside its parent: `#[path = \"board_tests.rs\"] mod tests;`."),
                        );
                    }
                }
                let kids = i.content.as_ref().map_or(Kids::None, |(_, items)| Kids::Mod(items));
                ("mod", i.ident.to_string(), &i.attrs, i.ident.span(), kids)
            }
            Item::Static(i) => ("static", i.ident.to_string(), &i.attrs, i.ident.span(), Kids::None),
            Item::Struct(i) => ("struct", i.ident.to_string(), &i.attrs, i.ident.span(), Kids::None),
            Item::Trait(i) => ("trait", i.ident.to_string(), &i.attrs, i.ident.span(), Kids::Trait(&i.items)),
            Item::TraitAlias(i) => ("trait", i.ident.to_string(), &i.attrs, i.ident.span(), Kids::None),
            Item::Type(i) => ("type", i.ident.to_string(), &i.attrs, i.ident.span(), Kids::None),
            Item::Union(i) => ("union", i.ident.to_string(), &i.attrs, i.ident.span(), Kids::None),
            Item::Use(i) => ("use", self.text(&i.tree), &i.attrs, i.tree.span(), Kids::None),
            other => ("item", self.text(other), &[], other.span(), Kids::None),
        };
        let full = within(prefix, &name);
        // An `extern` block's items are the module's own.
        let inner = if kind == "extern" { prefix.to_string() } else { full.clone() };
        let mut node = self.leaf(kind, full, attrs, at, item);
        if let Item::Mod(i) = item {
            node.file = mod_paths(&i.attrs).into_iter().next().map(|(_, v)| v);
        }
        node.children = match kids {
            Kids::None => None,
            Kids::Impl(items) => Some(items.iter().map(|c| self.impl_item(c, &inner)).collect()),
            Kids::Trait(items) => Some(items.iter().map(|c| self.trait_item(c, &inner)).collect()),
            Kids::Foreign(items) => Some(items.iter().map(|c| self.foreign_item(c, &inner)).collect()),
            Kids::Mod(items) => {
                let module = format!("{module}::{name}");
                let kids: Vec<Node> = items.iter().map(|c| self.node(c, &inner, &module)).collect();
                self.duplicates(&kids, &module, &inner);
                Some(kids)
            }
        };
        node
    }

    fn impl_item(&self, item: &ImplItem, prefix: &str) -> Node {
        match item {
            ImplItem::Fn(f) => self.leaf("fn", within(prefix, &f.sig.ident.to_string()), &f.attrs, f.sig.ident.span(), item),
            ImplItem::Const(c) => self.leaf("const", within(prefix, &c.ident.to_string()), &c.attrs, c.ident.span(), item),
            ImplItem::Type(t) => self.leaf("type", within(prefix, &t.ident.to_string()), &t.attrs, t.ident.span(), item),
            ImplItem::Macro(m) => self.leaf("macro", within(prefix, &self.text(&m.mac.path)), &m.attrs, m.mac.path.span(), item),
            other => self.leaf("item", within(prefix, &self.text(other)), &[], other.span(), item),
        }
    }

    fn trait_item(&self, item: &TraitItem, prefix: &str) -> Node {
        match item {
            TraitItem::Fn(f) => self.leaf("fn", within(prefix, &f.sig.ident.to_string()), &f.attrs, f.sig.ident.span(), item),
            TraitItem::Const(c) => self.leaf("const", within(prefix, &c.ident.to_string()), &c.attrs, c.ident.span(), item),
            TraitItem::Type(t) => self.leaf("type", within(prefix, &t.ident.to_string()), &t.attrs, t.ident.span(), item),
            TraitItem::Macro(m) => self.leaf("macro", within(prefix, &self.text(&m.mac.path)), &m.attrs, m.mac.path.span(), item),
            other => self.leaf("item", within(prefix, &self.text(other)), &[], other.span(), item),
        }
    }

    fn foreign_item(&self, item: &ForeignItem, prefix: &str) -> Node {
        match item {
            ForeignItem::Fn(f) => self.leaf("fn", within(prefix, &f.sig.ident.to_string()), &f.attrs, f.sig.ident.span(), item),
            ForeignItem::Static(s) => self.leaf("static", within(prefix, &s.ident.to_string()), &s.attrs, s.ident.span(), item),
            ForeignItem::Type(t) => self.leaf("type", within(prefix, &t.ident.to_string()), &t.attrs, t.ident.span(), item),
            ForeignItem::Macro(m) => self.leaf("macro", within(prefix, &self.text(&m.mac.path)), &m.attrs, m.mac.path.span(), item),
            other => self.leaf("item", within(prefix, &self.text(other)), &[], other.span(), item),
        }
    }

    /// Refuses a name defined again in one module: its items, and those of its
    /// `impl`, `trait` and `extern` blocks. `prefix` is the module's part of the names.
    fn duplicates(&mut self, nodes: &[Node], module: &str, prefix: &str) {
        let mut defs: Vec<&Node> = vec![];
        for n in nodes {
            if n.kind != "impl" && n.kind != "extern" {
                defs.push(n);
            }
            if matches!(n.kind, "impl" | "extern" | "trait") {
                defs.extend(n.children.iter().flatten());
            }
        }
        let local = |n: &str| n.strip_prefix(&format!("{prefix}::")).unwrap_or(n).to_string();
        let mut first: HashMap<&str, &Node> = HashMap::new();
        let mut found = vec![];
        for d in defs.into_iter().filter(|d| !matches!(d.kind, "use" | "macro" | "item") && d.name != "_" && !d.name.ends_with("::_")) {
            match first.get(d.name.as_str()) {
                Some(f) => {
                    let cfgs: Vec<String> = f.cfgs.iter().chain(&d.cfgs).map(|c| format!("`{c}`")).collect();
                    let cfgs = if cfgs.is_empty() { String::new() } else { format!(" ({})", cfgs.join(", ")) };
                    let name = local(&d.name);
                    found.push((
                        d.at,
                        format!("Not saved: `{name}` is defined twice in `{module}`{cfgs}. Keep one `{name}` and put the platform code inside it: `#[cfg(unix)] {{ … }}` or `if cfg!(unix) {{ … }}`."),
                    ));
                }
                None => {
                    first.insert(&d.name, d);
                }
            }
        }
        found.into_iter().for_each(|(at, m)| self.refuse(at, m));
    }
}

/// The first byte from `at` that is neither whitespace nor a comment.
fn past_comments(source: &str, mut at: usize) -> usize {
    loop {
        let rest = &source[at..];
        let t = rest.trim_start();
        at += rest.len() - t.len();
        if t.starts_with("//") {
            at += t.find('\n').map_or(t.len(), |n| n + 1);
        } else if t.starts_with("/*") {
            at += block_comment(t);
        } else {
            return at;
        }
    }
}

/// The length of the block comment `t` starts with, the comments nested in it
/// (`/* /* */ */`) included.
fn block_comment(t: &str) -> usize {
    let b = t.as_bytes();
    let (mut depth, mut i) = (0, 0);
    while i < b.len() {
        if b[i..].starts_with(b"/*") {
            depth += 1;
            i += 2;
        } else if b[i..].starts_with(b"*/") {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    b.len()
}

/// Where an item's text starts: at its line's start when only indentation is
/// before it, taking the `//` comment lines right above it, never before `floor`.
fn lead(source: &str, at: usize, floor: usize) -> usize {
    let line_start = |i: usize| source[..i].rfind('\n').map_or(0, |n| n + 1);
    let mut start = line_start(at);
    if start < floor || !source[start..at].trim().is_empty() {
        return at;
    }
    while start > floor {
        let above = line_start(start - 1);
        let t = source[above..start].trim();
        if above < floor || !t.starts_with("//") || t.starts_with("//!") {
            break;
        }
        start = above;
    }
    start
}

/// The pieces of `nodes`, each up to the next one's start and the last up to
/// `to`, with where the first starts; imports before the first item are left
/// to the text before it, and so, in a file (`top`), are its `mod x;` declarations.
fn cut(source: &str, nodes: &[Node], floor: usize, to: usize, top: bool) -> (usize, Vec<Piece>) {
    let nodes: Vec<&Node> =
        nodes.iter().skip_while(|n| matches!(n.kind, "use" | "extern crate") || (top && n.kind == "mod" && n.children.is_none())).collect();
    let mut starts = vec![];
    let mut prev = floor;
    for n in &nodes {
        starts.push(lead(source, n.item.start, prev));
        prev = n.item.end;
    }
    let mut pieces = vec![];
    for (k, n) in nodes.iter().enumerate() {
        let (start, end) = (starts[k], starts.get(k + 1).copied().unwrap_or(to));
        let leaf = |text: String| Piece {
            kind: n.kind,
            name: n.name.clone(),
            item: n.item.clone(),
            head: n.head,
            file: n.file.clone(),
            text,
            children: vec![],
            tail: String::new(),
        };
        let piece = match n.children.as_deref() {
            Some(kids @ [.., last]) => {
                let (first, children) = cut(source, kids, n.item.start, last.item.end, false);
                if children.is_empty() {
                    leaf(source[start..end].to_string())
                } else {
                    let tail = source[last.item.end..end].to_string();
                    Piece { text: source[start..first].to_string(), children, tail, ..leaf(String::new()) }
                }
            }
            _ => leaf(source[start..end].to_string()),
        };
        pieces.push(piece);
    }
    (starts.first().copied().unwrap_or(to), pieces)
}

/// The files a module's attributes name, each with its attribute's start:
/// `#[path = "x"]`, and each `path = "x"` after a `#[cfg_attr(…)]`'s predicate,
/// nested ones included.
fn mod_paths(attrs: &[Attribute]) -> Vec<(usize, String)> {
    let mut out = vec![];
    for a in attrs {
        let at = a.span().byte_range().start;
        out.extend(paths_in(&a.meta).into_iter().map(|v| (at, v)));
    }
    out
}

/// The `path = "x"` values of one attribute: its own, or those after a
/// `cfg_attr`'s predicate, at any depth.
fn paths_in(meta: &syn::Meta) -> Vec<String> {
    match meta {
        syn::Meta::NameValue(nv) if nv.path.is_ident("path") => match &nv.value {
            syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => vec![s.value()],
            _ => vec![],
        },
        syn::Meta::List(l) if l.path.is_ident("cfg_attr") => l
            .parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
            .map(|metas| metas.iter().skip(1).flat_map(paths_in).collect())
            .unwrap_or_default(),
        _ => vec![],
    }
}

/// A Cargo package of one crate (D-158 point 1) as (file from `root`, module
/// path, split file): its crate's files from its root, `src/main.rs`,
/// `src/lib.rs` or one program in `src/bin/` (module `crate`), then each
/// `tests/*.rs` and the files it reaches.
pub fn package(root: &Path) -> Result<Vec<(String, String, File)>, Failure> {
    let text = fs::read_to_string(root.join("Cargo.toml")).map_err(|e| Failure(format!("Cargo.toml: {e}"), 1))?;
    let manifest: toml::Table = text.parse().map_err(|e| Failure(format!("Cargo.toml: {e}"), 1))?;
    let others = members(root, manifest.get("workspace"));
    let Some(package) = manifest.get("package") else {
        if others.is_empty() {
            return Err(Failure("Cargo.toml has no `[package]`; Solace holds a package of one crate.".into(), 1));
        }
        return Err(Failure(format!("This is a workspace of {} crates; add each member as its own project.", others.len()), 1));
    };
    if !others.is_empty() {
        return Err(Failure(format!("This is a workspace of {} crates; add each member as its own project.", others.len() + 1), 1));
    }
    let name = package.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let lib = manifest.get("lib");
    // Cargo accepts both spellings.
    if ["proc-macro", "proc_macro"].iter().any(|k| lib.and_then(|l| l.get(*k)).and_then(|v| v.as_bool()) == Some(true)) {
        return Err(Failure(format!("`{name}` is a proc-macro crate; Solace doesn't hold proc-macro crates yet."), 1));
    }
    let moved = moved_targets(&manifest, package, name);
    if !moved.is_empty() {
        return Err(Failure(format!("Cargo.toml sets {}; Solace reads a package's files from Cargo's default places for now.", moved.join(", ")), 1));
    }
    let mut crates: Vec<String> = ["src/lib.rs", "src/main.rs"].into_iter().filter(|f| root.join(f).is_file()).map(String::from).collect();
    for n in entries(root, "src/bin")? {
        let file = if n.ends_with(".rs") { format!("src/bin/{n}") } else { format!("src/bin/{n}/main.rs") };
        if root.join(&file).is_file() {
            crates.push(file);
        }
    }
    let main = match crates.as_slice() {
        [one] => one.clone(),
        [] => return Err(Failure("This package has no crate: no `src/main.rs`, `src/lib.rs` or program in `src/bin/`.".into(), 1)),
        _ => return Err(Failure(format!("This package has {} crates ({}); Solace holds a package of one crate.", crates.len(), crates.join(", ")), 1)),
    };
    let mut out = vec![];
    let mut refused = vec![];
    let mut seen = HashSet::new();
    walk(root, main, "crate".into(), true, &mut seen, &mut out, &mut refused)?;
    for name in entries(root, "tests")?.into_iter().filter(|n| n.ends_with(".rs") && root.join("tests").join(n).is_file()) {
        // A test crate's modules take the `integration::` prefix (D-169), so a helper shared by
        // two test crates (`tests/common/mod.rs`) holds one name and never clashes with a library
        // module of the same name, nor with the crate's own unit-test module `tests`.
        let module = format!("integration::{}", name.trim_end_matches(".rs"));
        walk(root, format!("tests/{name}"), module, true, &mut seen, &mut out, &mut refused)?;
    }
    if !refused.is_empty() {
        return Err(Failure(refused.join("\n"), 1));
    }
    let mut modules = HashMap::new();
    for (path, module, _) in &out {
        if let Some(first) = modules.insert(module.as_str(), path.as_str()) {
            return Err(Failure(format!("Two modules are named `{module}` ({first}, {path}); Solace holds one module of each name for now."), 1));
        }
    }
    Ok(out)
}

/// The member folders of a `[workspace]` other than the package's own, its
/// globs expanded (`crates/*`), the `exclude`d ones left out.
fn members(root: &Path, workspace: Option<&toml::Value>) -> Vec<String> {
    let folders = |key: &str| -> Vec<String> {
        let list = workspace.and_then(|w| w.get(key)).and_then(|v| v.as_array());
        let paths = list.into_iter().flatten().filter_map(|v| v.as_str());
        paths.map(|p| p.split('/').filter(|s| !s.is_empty() && *s != ".").collect::<Vec<_>>().join("/")).collect()
    };
    let exclude = folders("exclude");
    let mut found = vec![];
    for member in folders("members") {
        // A named member counts whether or not its folder is there yet; Cargo needs it either way.
        if !member.contains(['*', '?', '[', '{']) {
            found.extend(Some(member).filter(|m| !m.is_empty()));
            continue;
        }
        let mut dirs = vec![String::new()];
        for part in member.split('/').filter(|s| !s.is_empty()) {
            let glob = globset::Glob::new(part).ok().filter(|_| part.contains(['*', '?', '[', '{'])).map(|g| g.compile_matcher());
            dirs = dirs
                .into_iter()
                .flat_map(|d| {
                    let names: Vec<String> = match &glob {
                        Some(m) => entries(root, &d).unwrap_or_default().into_iter().filter(|n| m.is_match(n)).collect(),
                        None => vec![part.to_string()],
                    };
                    names.into_iter().map(move |n| if d.is_empty() { n } else { format!("{d}/{n}") })
                })
                .filter(|p| root.join(p).is_dir())
                .collect();
        }
        found.extend(dirs.into_iter().filter(|d| !d.is_empty() && root.join(d).join("Cargo.toml").is_file()));
    }
    found.retain(|d| !exclude.contains(d));
    found.sort();
    found.dedup();
    found
}

/// The targets `manifest` moves from Cargo's default places: a `[lib]`,
/// `[[bin]]` or `[[test]]` path elsewhere, or `autolib`, `autobins` or `autotests` off.
fn moved_targets(manifest: &toml::Table, package: &toml::Value, name: &str) -> Vec<String> {
    let path = |t: &toml::Value| t.get("path").and_then(|p| p.as_str()).map(String::from);
    let mut moved = vec![];
    if let Some(p) = manifest.get("lib").and_then(path).filter(|p| p != "src/lib.rs") {
        moved.push(format!("`[lib] path = \"{p}\"`"));
    }
    let targets = |key: &str| manifest.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    for (key, dir) in [("bin", "src/bin"), ("test", "tests")] {
        for t in targets(key) {
            let n = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let mut defaults = vec![format!("{dir}/{n}.rs"), format!("{dir}/{n}/main.rs")];
            if key == "bin" && n == name {
                defaults.push("src/main.rs".into());
            }
            if let Some(p) = path(&t).filter(|p| !defaults.contains(p)) {
                moved.push(format!("`[[{key}]] path = \"{p}\"`"));
            }
        }
    }
    for key in ["autolib", "autobins", "autotests"] {
        if package.get(key).and_then(|v| v.as_bool()) == Some(false) {
            moved.push(format!("`{key} = false`"));
        }
    }
    moved
}

/// The names in folder `dir` of `root`, sorted; none when it doesn't exist.
fn entries(root: &Path, dir: &str) -> Result<Vec<String>, Failure> {
    let Ok(read) = fs::read_dir(root.join(dir)) else { return Ok(vec![]) };
    let mut names = vec![];
    for e in read {
        names.push(e?.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}

/// Splits file `rel` as `module`, then the files its `mod x;` declarations name.
/// `owns_dir`: a crate root, `mod.rs` or `#[path]` file, whose modules are beside
/// it rather than in a folder named after it.
fn walk(
    root: &Path,
    rel: String,
    module: String,
    owns_dir: bool,
    seen: &mut HashSet<String>,
    out: &mut Vec<(String, String, File)>,
    refused: &mut Vec<String>,
) -> Result<(), Failure> {
    if let Some((_, first, _)) = out.iter().find(|(p, ..)| *p == rel) {
        // Test crates share a helper (`tests/common/mod.rs`); it is held once, under the
        // name `integration::common` every test crate's `mod common;` gives it.
        if rel.starts_with("tests/") && first == &module {
            return Ok(());
        }
        return Err(Failure(format!("{rel} is two modules (`{first}`, `{module}`); Solace holds a file as one module for now."), 1));
    }
    // A refused file never reaches `out`; `seen` still keeps a shared test helper from being read again.
    if !seen.insert(rel.clone()) {
        return Ok(());
    }
    let text = fs::read_to_string(root.join(&rel)).map_err(|e| Failure(format!("{rel}: {e}"), 1))?;
    let mut file = split(&text, &module).map_err(|e| Failure(format!("{rel}: {}", e.0), 1))?;
    let ok = file.refusals.is_empty();
    if ok {
        if let Some(name) = inline_file_mod(&text, &file.items) {
            return Err(Failure(format!("{rel}: `{name}` is a `mod x;` inside an inline `mod {{ … }}`; Solace doesn't hold one there yet."), 1));
        }
    } else {
        refused.extend(file.refusals.iter().map(|r| format!("{rel}: line {} (`{}`): {}", r.line, r.quote, r.message)));
    }
    let path = Path::new(&rel);
    let dir = path.parent().unwrap().to_string_lossy().replace('\\', "/");
    let base = child_dir(&rel, owns_dir);
    let mods = file.mods.clone();
    if ok {
        file.owns_dir = owns_dir;
        out.push((rel.clone(), module.clone(), file));
    }
    // A test crate's own `mod x;` is `integration::x` (D-169), so every test crate that names
    // `tests/common/mod.rs` holds the same module name, and it never clashes with the
    // library module `common`. A library crate's own `mod x;` takes the bare name `x`.
    for (name, named) in mods {
        let child = child_module(&module, &rel, owns_dir, &name);
        if let Some(p) = named {
            walk(root, format!("{dir}/{p}"), child, true, seen, out, refused)?;
            continue;
        }
        let (flat, nested) = (child_file(&rel, owns_dir, &name), format!("{base}{name}/mod.rs"));
        if root.join(&flat).is_file() {
            walk(root, flat, child, false, seen, out, refused)?;
        } else if root.join(&nested).is_file() {
            walk(root, nested, child, true, seen, out, refused)?;
        } else {
            return Err(Failure(format!("{rel}: `mod {name};` has no file: neither {flat} nor {nested} exists."), 1));
        }
    }
    Ok(())
}

/// The first `mod x;` inside an inline module of `pieces`.
fn inline_file_mod(text: &str, pieces: &[Piece]) -> Option<String> {
    pieces.iter().filter(|p| p.kind == "mod").flat_map(|p| &p.children).find_map(|c| {
        if c.kind == "mod" && c.children.is_empty() && text[c.item.clone()].ends_with(';') {
            Some(c.name.clone())
        } else {
            inline_file_mod(text, std::slice::from_ref(c))
        }
    })
}

/// The byte ranges of the string literals in `source`, their contents and quotes
/// included: a nested item's reindent leaves a multi-line literal as written
/// (D-168). Tokenizing finds them, so numbers and `'a'`/`b'a'` don't count.
pub fn string_literals(source: &str) -> Vec<Range<usize>> {
    let Ok(tokens) = source.parse::<proc_macro2::TokenStream>() else { return vec![] };
    let mut out = vec![];
    let mut groups = vec![tokens];
    while let Some(stream) = groups.pop() {
        for tree in stream {
            match tree {
                proc_macro2::TokenTree::Literal(l) if is_string(&l) => out.push(l.span().byte_range()),
                proc_macro2::TokenTree::Group(g) => groups.push(g.stream()),
                _ => {}
            }
        }
    }
    out
}

/// Whether a literal token is a string (`"x"`, `r"x"`, `r#"x"#`, `b"x"`, `c"x"`),
/// not a number or a character.
fn is_string(l: &proc_macro2::Literal) -> bool {
    let text = l.to_string();
    let rest = text.strip_prefix('b').or_else(|| text.strip_prefix('c')).unwrap_or(&text);
    rest.starts_with('"') || rest.starts_with("r\"") || rest.starts_with("r#")
}

/// Whether `source` holds a module-level `use`: parsed, so `pub use` and
/// `pub(crate) use` count, and an inline `mod`'s imports — which a nested save
/// would drop as a child row — are found (D-168).
pub fn imports(source: &str) -> Result<bool, syn::Error> {
    Ok(!import_texts(source)?.is_empty())
}

/// Whether an item is a `use`, an inline `mod`'s items included (D-168).
fn item_use(item: &Item) -> bool {
    match item {
        Item::Use(_) => true,
        Item::Mod(m) => m.content.as_ref().is_some_and(|(_, items)| items.iter().any(item_use)),
        _ => false,
    }
}

/// The folder a new module of `parent_file` goes in, from the project root
/// (D-168): beside `parent_file` when its parent owns its folder, and in a
/// folder named after the parent file otherwise.
fn child_dir(parent_file: &str, owns_dir: bool) -> String {
    let (dir, file) = parent_file.rsplit_once('/').map_or(("", parent_file), |(d, f)| (d, f));
    let folder = if owns_dir { String::new() } else { format!("{}/", file.strip_suffix(".rs").unwrap_or(file)) };
    if dir.is_empty() { folder } else { format!("{dir}/{folder}") }
}

/// The file a new module `name` takes, from the project root (D-168): a crate
/// root, `tests/x.rs`, `mod.rs` and a `#[path]` file place children beside
/// themselves; any other file places them in a folder named after it. So
/// `src/board/agents.rs` gives `src/board/agents/cache.rs`, and a child of
/// `tests/x.rs` gives `tests/y.rs`.
pub fn child_file(parent_file: &str, owns_dir: bool, name: &str) -> String {
    format!("{}{name}.rs", child_dir(parent_file, owns_dir))
}

/// Whether a module's file places its `mod x;` children beside itself (D-168):
/// a crate root, a `tests/*.rs` crate root, `mod.rs`, or a `#[path]` file,
/// whose name is not the module's own last segment.
pub fn owns_dir(path: &str, file: &str) -> bool {
    let (dir, name) = file.rsplit_once('/').map_or(("", file), |(d, f)| (d, f));
    let stem = name.strip_suffix(".rs").unwrap_or(name);
    matches!(stem, "mod" | "lib" | "main") || dir == "tests" || Some(stem) != path.rsplit("::").next()
}

/// The module a child `name` of module `path` (file `file`) takes (D-168): a
/// test crate's children share its `integration::` prefix (D-169), and the
/// crate root's take the bare name. `owns_dir` is the caller's, which only
/// `walk` knows when a `#[path]` file named the module.
pub fn child_module(path: &str, file: &str, owns_dir: bool, name: &str) -> String {
    if owns_dir && file.rsplit_once('/').is_some_and(|(dir, _)| dir == "tests") {
        format!("integration::{name}")
    } else if path == "crate" {
        name.to_string()
    } else {
        format!("{path}::{name}")
    }
}

#[cfg(test)]
#[path = "rust_tests.rs"]
mod tests;

/// The source text of every module-level `use`, an inline `mod`'s included.
pub fn import_texts(source: &str) -> Result<Vec<String>, syn::Error> {
    Ok(import_ranges(source)?.into_iter().map(|(item, _)| source[item].trim().to_string()).collect())
}

/// The item and tree byte ranges of every module-level `use`, an inline
/// `mod`'s included.
pub(crate) fn import_ranges(source: &str) -> Result<Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>, syn::Error> {
    fn collect(items: &[Item], out: &mut Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>) {
        for item in items {
            if !item_use(item) {
                continue;
            }
            match item {
                Item::Use(i) => out.push((i.span().byte_range(), i.tree.span().byte_range())),
                Item::Mod(m) => {
                    if let Some((_, items)) = &m.content {
                        collect(items, out);
                    }
                }
                _ => {}
            }
        }
    }
    let file = syn::parse_file(source)?;
    let mut out = vec![];
    collect(&file.items, &mut out);
    Ok(out)
}

/// The byte range of every module-level `use` tree, an inline `mod`'s included.
pub(crate) fn import_tree_ranges(source: &str) -> Result<Vec<std::ops::Range<usize>>, syn::Error> {
    Ok(import_ranges(source)?.into_iter().map(|(_, tree)| tree).collect())
}

