use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;

use super::{Rewrite, add_import_line, is_known_name, leaf, names_need_glob, staying_in};
use crate::{Failure, Index};
fn parent_module(module: &str) -> &str {
    module.rsplit_once("::").map_or("crate", |(parent, _)| parent)
}

pub(crate) fn reaching_modules<'a>(reaching: &'a HashMap<String, HashSet<String>>, module: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    let known = reaching.get(module);
    let fallback = (!reaching.contains_key(module)).then_some(module);
    known.into_iter().flat_map(|modules| modules.iter().map(String::as_str)).chain(fallback)
}

pub(crate) fn descendants_need_glob(cx: &Rewrite, source: &str) -> Result<bool, Failure> {
    let source_named = named_imports(cx, source)?;
    for module in reaching_modules(cx.reaching, source) {
        if module != source && !glob_names_in(cx, source, &source_named, &module).is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn descendant_mentions(cx: &Rewrite, source: &str, name: &str) -> Vec<String> {
    let mut out = vec![];
    for module in reaching_modules(cx.reaching, source) {
        if module == source || module_has_name(cx.ix, source, module, name) {
            continue;
        }
        let mentioned = staying_in(cx.ix, cx.moved, cx.newmod, module).any(|i| cx.mentioned[i].contains(name));
        if mentioned {
            out.push(module.to_string());
        }
    }
    out
}

fn module_has_name(ix: &Index, source: &str, module: &str, name: &str) -> bool {
    let mut at = module;
    loop {
        if is_known_name(ix, at, name) {
            return true;
        }
        if at == source || at == "crate" {
            return false;
        }
        at = parent_module(at);
    }
}

fn import_line(path: &[String], local: &str) -> String {
    let path = path.join("::");
    if leaf(&path) == local { format!("use {path};") } else { format!("use {path} as {local};") }
}

pub(crate) fn add_module_import(
    module: &str,
    path: &[String],
    local: &str,
    newheaders: &mut HashMap<String, String>,
) -> Result<(), Failure> {
    let header = newheaders.get(module).cloned().unwrap_or_default();
    let line = import_line(path, local);
    let exists = header.lines().any(|existing| existing.trim() == line);
    if !exists {
        newheaders.insert(module.to_string(), add_import_line(&header, &line));
    }
    Ok(())
}

pub(crate) fn add_descendant_import(
    cx: &Rewrite,
    source: &str,
    name: &str,
    path: &[String],
    local: &str,
    newheaders: &mut HashMap<String, String>,
) -> Result<(), Failure> {
    for module in descendant_mentions(cx, source, name) {
        add_module_import(&module, path, local, newheaders)?;
    }
    Ok(())
}

pub(crate) fn add_glob_descendant_imports(
    cx: &Rewrite,
    source: &str,
    site: &crate::language::rust_refs::ImportSite,
    newheaders: &mut HashMap<String, String>,
) -> Result<(), Failure> {
    let source_named = named_imports(cx, source)?;
    for module in reaching_modules(cx.reaching, source) {
        if module == source {
            continue;
        }
        for name in glob_names_in(cx, source, &source_named, &module) {
            let mut path = site.path.clone();
            path.push(name.clone());
            add_module_import(&module, &path, &name, newheaders)?;
        }
    }
    Ok(())
}

pub(crate) fn named_imports(cx: &Rewrite, module: &str) -> Result<HashSet<String>, Failure> {
    Ok(crate::language::rust_refs::import_sites(&cx.ix.syms, &cx.ix.modules, module, true)?
        .into_iter()
        .filter(|site| !site.glob)
        .map(|site| site.local)
        .collect())
}

fn glob_names_in(cx: &Rewrite, source: &str, source_named: &HashSet<String>, module: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for i in staying_in(cx.ix, cx.moved, cx.newmod, module) {
        for name in &cx.mentioned[i] {
            let one: HashSet<String> = [name.clone()].into();
            if source_named.contains(name)
                || module_has_name(cx.ix, source, module, name)
                || !names_need_glob(cx.ix, source, &one, source_named)
            {
                continue;
            }
            out.insert(name.clone());
        }
    }
    out
}

fn module_paths(ix: &Index) -> BTreeSet<String> {
    ix.modules.iter().map(|m| m.path.clone()).chain(ix.syms.iter().map(|s| s.module.clone())).collect()
}

fn super_glob_modules(ix: &Index, modules: &BTreeSet<String>) -> Result<HashSet<String>, Failure> {
    let headers: HashMap<&str, &str> = ix.modules.iter().map(|m| (m.path.as_str(), m.header.as_str())).collect();
    let mut uses: HashMap<&str, Vec<&str>> = HashMap::new();
    for symbol in ix.syms.iter().filter(|s| s.kind_text == "use" && s.parent.is_none()) {
        uses.entry(symbol.module.as_str()).or_default().push(&symbol.text);
    }
    let mut out = HashSet::new();
    for module in modules {
        let mut texts = vec![];
        if let Some(header) = headers.get(module.as_str()) {
            texts.push(*header);
        }
        if let Some(lines) = uses.get(module.as_str()) {
            texts.extend(lines.iter().copied());
        }
        for text in texts {
            let sites = crate::language::rust_refs::import_sites_in(text)
                .map_err(|e| Failure(format!("imports in {module} don't parse: {e}"), 1))?;
            if sites.iter().any(|site| site.glob && site.path.len() == 1 && site.path[0] == "super") {
                out.insert(module.clone());
                break;
            }
        }
    }
    Ok(out)
}

pub(crate) fn reach_map(ix: &Index, sources: &BTreeSet<String>) -> Result<HashMap<String, HashSet<String>>, Failure> {
    let modules = module_paths(ix);
    let super_glob = super_glob_modules(ix, &modules)?;
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for module in &modules {
        if super_glob.contains(module) {
            children.entry(parent_module(module).to_string()).or_default().push(module.clone());
        }
    }
    let mut out = HashMap::new();
    for source in sources {
        let mut reaching = HashSet::from([source.clone()]);
        let mut at = vec![source.clone()];
        while let Some(module) = at.pop() {
            if let Some(kids) = children.get(&module) {
                for kid in kids {
                    if reaching.insert(kid.clone()) {
                        at.push(kid.clone());
                    }
                }
            }
        }
        out.insert(source.clone(), reaching);
    }
    Ok(out)
}

pub(crate) fn reaches(reaching: &HashMap<String, HashSet<String>>, module: &str, other: &str) -> bool {
    reaching.get(module).map_or(other == module, |modules| modules.contains(other))
}
