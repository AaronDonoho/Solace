//! Index construction and signatures: the live symbols, modules, and their
//! resolved references.

use std::collections::HashMap;

use crate::language::{Import, Item, Kind, Language, Ref};

/// All live symbols and modules, with the reference graph resolved.
pub(crate) struct Index {
    pub(crate) lang: &'static dyn Language,
    /// Whether this is a Rust index: signatures come from the file's text.
    pub(crate) rust: bool,
    pub(crate) syms: Vec<Sym>,
    pub(crate) modules: Vec<ModuleRow>,
    pub(crate) uses: Vec<Vec<usize>>,
    /// The reverse of `uses`: the symbols that use each one, in order.
    pub(crate) used_by: Vec<Vec<usize>>,
    /// Every resolved reference: (symbol, line in the user's text).
    pub(crate) at: Vec<Vec<(usize, usize)>>,
    /// References that need type inference: (target, line), as in `at`.
    pub(crate) possible: Vec<Vec<(usize, usize)>>,
    /// Whether each symbol is test code (D-170).
    pub(crate) test: Vec<bool>,
    /// The modules that hold tests (D-170): a `tests/` folder, or a Rust
    /// module declared under `#[cfg(test)]`, and everything below it.
    pub(crate) test_modules: std::collections::HashSet<String>,
}

#[derive(Clone)]
pub(crate) struct Sym {
    pub(crate) id: String,
    pub(crate) module: String,
    pub(crate) name: String,
    pub(crate) kind: Kind,
    /// The kind as the source writes it; `Kind` keeps only Gleam's three (D-158).
    pub(crate) kind_text: String,
    pub(crate) text: String,
    /// The enclosing `impl`, `trait`, `extern` or inline `mod`, by id (Rust).
    pub(crate) parent: Option<String>,
    /// What follows the item in its file; `None`: Gleam's blank line.
    pub(crate) sep: Option<String>,
    /// A container's text after its last child.
    pub(crate) tail: String,
}

impl Sym {
    pub(crate) fn qname(&self) -> String {
        format!("{}.{}", self.module, self.name)
    }

    pub(crate) fn item(&self) -> Item {
        Item { kind: self.kind, name: self.name.clone(), text: self.text.clone() }
    }
}

pub(crate) struct ModuleRow {
    pub(crate) path: String,
    pub(crate) dir: String,
    pub(crate) header: String,
    /// Its file from the project root, written exactly (Rust); `None` for Gleam.
    pub(crate) file: Option<String>,
    /// Whether its `mod x;` children sit beside its file (Rust); `None` for
    /// Gleam or a store written before the column (D-168).
    pub(crate) owns_dir: Option<bool>,
}

impl ModuleRow {
    /// Its file from the project root.
    pub(crate) fn rel(&self) -> String {
        self.file.clone().unwrap_or_else(|| crate::outside::file_of(&self.dir, &self.path))
    }
}

pub(crate) fn build(lang: &'static dyn Language, syms: Vec<Sym>, modules: Vec<ModuleRow>, rust: bool) -> Index {
    // A Rust store's graph comes from the names its text mentions, resolved
    // against the module's `use` lines and the crate's item names (D-170).
    if rust {
        let g = crate::language::rust_refs::graph(&syms, &modules);
        return Index {
            lang,
            rust,
            syms,
            modules,
            uses: g.uses,
            used_by: g.used_by,
            at: g.at,
            possible: g.possible,
            test: g.test,
            test_modules: g.test_modules,
        };
    }
    // (module, name) -> symbol, for names and for type constructors
    let mut names: HashMap<(&str, &str), usize> = HashMap::new();
    for (i, s) in syms.iter().enumerate() {
        names.insert((&s.module, &s.name), i);
    }
    let mut ctors: Vec<(String, String, usize)> = vec![];
    for (i, s) in syms.iter().enumerate() {
        if s.kind == Kind::Type {
            for c in lang.constructors(&s.text) {
                ctors.push((s.module.clone(), c, i));
            }
        }
    }
    for (m, c, i) in &ctors {
        names.entry((m, c)).or_insert(*i);
    }
    let imports: HashMap<&str, Vec<Import>> = modules.iter().map(|m| (m.path.as_str(), lang.imports(&m.header))).collect();
    let at: Vec<Vec<(usize, usize)>> = syms
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let imps = &imports[s.module.as_str()];
            let mut found = vec![];
            for (r, line) in lang.references_at(&s.text) {
                let target = match &r {
                    Ref::Qualified(alias, member) => {
                        imps.iter().find(|imp| &imp.alias == alias).and_then(|imp| names.get(&(imp.module.as_str(), member.as_str())))
                    }
                    Ref::Bare(name) => names.get(&(s.module.as_str(), name.as_str())).or_else(|| {
                        imps.iter().find_map(|imp| {
                            imp.unqualified
                                .iter()
                                .find(|(_, local)| local == name)
                                .and_then(|(orig, _)| names.get(&(imp.module.as_str(), orig.as_str())))
                        })
                    }),
                };
                if let Some(&t) = target
                    && t != i
                {
                    found.push((t, line));
                }
            }
            found
        })
        .collect();
    let uses: Vec<Vec<usize>> = at
        .iter()
        .map(|found| {
            let mut uses: Vec<usize> = vec![];
            for &(t, _) in found {
                if !uses.contains(&t) {
                    uses.push(t);
                }
            }
            uses
        })
        .collect();
    let mut used_by = vec![vec![]; syms.len()];
    for (j, us) in uses.iter().enumerate() {
        for &u in us {
            used_by[u].push(j);
        }
    }
    let test: Vec<bool> = syms.iter().map(|s| modules.iter().any(|m| m.path == s.module && crate::store::is_test(m))).collect();
    let test_modules = modules.iter().filter(|m| crate::store::is_test(m)).map(|m| m.path.clone()).collect();
    let possible = vec![vec![]; syms.len()];
    Index { lang, rust, syms, modules, uses, used_by, at, possible, test, test_modules }
}

pub(crate) fn signature(ix: &Index, s: &Sym) -> String {
    if ix.rust { crate::orientation::rust_signature(&s.text, &s.kind_text) } else { ix.lang.signature(&s.item()) }
}

/// An index built from (dir, module, source) triples, for tests.
#[cfg(test)]
pub(crate) fn test_index(sources: &[(&str, &str, &str)]) -> Index {
    let (mut syms, mut modules) = (vec![], vec![]);
    for (dir, path, text) in sources {
        let m = crate::language::gleam::split(text);
        modules.push(ModuleRow { path: (*path).into(), dir: (*dir).into(), header: m.header, file: None, owns_dir: None });
        for item in m.items {
            syms.push(Sym {
                id: format!("fn:{path}.{}", item.name),
                module: (*path).into(),
                name: item.name,
                kind: item.kind,
                kind_text: item.kind.as_str().to_string(),
                text: item.text,
                parent: None,
                sep: None,
                tail: String::new(),
            });
        }
    }
    build(&crate::language::gleam::Gleam, syms, modules, false)
}

/// The exact users of `i`, then the ones only a possible reference names.
pub(crate) fn users_split(ix: &Index, i: usize) -> (Vec<usize>, Vec<usize>) {
    let mut exact = vec![];
    let mut possible = vec![];
    for &j in &ix.used_by[i] {
        let only_possible = ix.possible[j].iter().any(|&(t, _)| t == i)
            && !ix.at[j].iter().any(|&(t, line)| t == i && !ix.possible[j].contains(&(t, line)));
        if only_possible { possible.push(j) } else { exact.push(j) }
    }
    (exact, possible)
}