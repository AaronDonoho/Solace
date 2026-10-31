//! Graph construction: the code map, the effects and state each symbol carries, and its requests.

use super::*;
use crate::Index;
#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;

pub(crate) fn direct_effects(ix: &Index, i: usize) -> BTreeMap<String, Vec<String>> {
    let s = &ix.syms[i];
    let imports = ix.modules.iter().find(|m| m.path == s.module).map(|m| ix.lang.imports(&m.header)).unwrap_or_default();
    let mut effects: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in ix.lang.references(&s.text) {
        let resolved = match r {
            Ref::Qualified(alias, name) => imports.iter().find(|imp| imp.alias == alias).map(|imp| (imp.module.clone(), name)),
            Ref::Bare(name) => imports.iter().find_map(|imp| {
                imp.unqualified.iter().find(|(_, local)| *local == name).map(|(orig, _)| (imp.module.clone(), orig.clone()))
            }),
        };
        if let Some((module, name)) = resolved
            && let Some(c) = category(&module, &name)
        {
            let call = format!("{}.{}", module.rsplit('/').next().unwrap_or(&module), name);
            let calls = effects.entry(c.into()).or_default();
            if !calls.contains(&call) {
                calls.push(call);
            }
        }
    }
    for line in s.text.lines().take_while(|l| !l.contains(" fn ")) {
        if let Some(rest) = line.trim().strip_prefix("@external(erlang,") {
            let fields: Vec<_> = rest.split('"').collect();
            if fields.len() >= 4 {
                let c = category(fields[1], fields[3]).unwrap_or("external code");
                effects.entry(c.into()).or_default().push(format!("{}:{}", fields[1], fields[3]));
            }
        } else if line.trim().starts_with("@external(") {
            effects.entry("external code".into()).or_default().push(line.trim().to_string());
        }
    }
    effects
}

// effects inside dependencies are seen only through this table; extend by settings later.
const EFFECTS: &[(&str, &str, &str)] = &[
    ("gleam/io", "*", "prints"),
    ("simplifile", "read*", "reads files"),
    ("simplifile", "write*", "writes files"),
    ("simplifile", "append*", "writes files"),
    ("simplifile", "delete*", "writes files"),
    ("simplifile", "create*", "writes files"),
    ("gleam/httpc", "*", "network"),
    ("gleam/fetch", "*", "network"),
    ("envoy", "*", "environment"),
    ("argv", "load", "environment"),
    ("gleam/time/timestamp", "system_time", "clock"),
    ("gleam/int", "random", "randomness"),
    ("gleam/float", "random", "randomness"),
    ("shellout", "*", "processes"),
    ("sqlight", "*", "database"),
    ("pog", "*", "database"),
    ("file", "write_file", "writes files"),
    ("file", "read_file", "reads files"),
    ("init", "get_plain_arguments", "environment"),
    ("os", "cmd", "processes"),
];

fn category(module: &str, name: &str) -> Option<&'static str> {
    EFFECTS
        .iter()
        .find(|(m, f, _)| *m == module && (f == &"*" || *f == name || f.strip_suffix('*').is_some_and(|p| name.starts_with(p))))
        .map(|(_, _, c)| *c)
}

fn check(statement: &str) -> Value {
    let chars: Vec<char> = statement.chars().collect();
    let mut depth = 0i32;
    let mut quoted = false;
    let mut escape = false;
    for (i, c) in chars.iter().enumerate() {
        if quoted {
            if escape {
                escape = false;
            } else if *c == '\\' {
                escape = true;
            } else if *c == '"' {
                quoted = false;
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '=' if depth == 0 && chars.get(i + 1) == Some(&'=') => {
                return json!({
                    "call": chars[..i].iter().collect::<String>().trim(),
                    "expected": chars[i + 2..].iter().collect::<String>().trim(),
                });
            }
            _ => {}
        }
    }
    json!({"text":statement.trim()})
}

fn checks(text: &str) -> Vec<Value> {
    let lines: Vec<_> = text.lines().collect();
    let mut out = vec![];
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let Some(first) = trimmed.strip_prefix("assert ") else {
            continue;
        };
        let indent = line.len() - trimmed.len();
        let mut statement = first.to_string();
        for next in lines.iter().skip(i + 1) {
            let n = next.trim_start();
            if n.is_empty() {
                continue;
            }
            if next.len() - n.len() <= indent {
                break;
            }
            statement.push(' ');
            statement.push_str(n);
        }
        out.push(check(&statement));
    }
    out
}

/// Splits at the first top-level `sep` (outside brackets).
fn split_top<'a>(s: &'a str, sep: &str) -> Option<(&'a str, &'a str)> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ if depth == 0 && s[i..].starts_with(sep) => return Some((&s[..i], &s[i + sep.len()..])),
            _ => {}
        }
    }
    None
}

fn spaced(name: &str) -> String {
    name.replace('_', " ")
}

/// `AddItem` as "Add item".
fn spoken(constructor: &str) -> String {
    let mut out = String::new();
    for (i, c) in constructor.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A type's name without its module or arguments: `option.Option(Int)` is `Option`.
fn base(t: &str) -> &str {
    let t = t.trim();
    let t = t.split('(').next().unwrap_or(t);
    t.rsplit('.').next().unwrap_or(t).trim()
}

/// A type in words, for people who don't read code (D-55).
pub(crate) fn words(t: &str) -> String {
    let t = t.trim();
    let args = |inner: &str| language::split_args(inner).into_iter().map(|(_, a)| words(&a)).collect::<Vec<_>>();
    let and = |parts: Vec<String>| match parts.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => "nothing".into(),
    };
    if let Some(inner) = t.strip_prefix("#(") {
        return and(args(inner));
    }
    if t.starts_with("fn(") {
        return "a function".into();
    }
    let inner = t.split_once('(').map(|(_, rest)| args(rest)).unwrap_or_default();
    match (base(t), inner.as_slice()) {
        ("Nil", _) => "nothing".into(),
        ("Int" | "Float", _) => "a number".into(),
        ("String", _) => "text".into(),
        ("Bool", _) => "yes or no".into(),
        ("List", [x]) => format!("a list of {x}"),
        ("Set", [x]) => format!("a set of {x}"),
        ("Option", [x]) => format!("{x} or nothing"),
        ("Result", [ok, _]) if ok == "nothing" => "success or an error".into(),
        ("Result", [ok, _]) => format!("{ok} or an error"),
        ("Dict", [k, v]) => format!("{v} by {k}"),
        (name, _) => name.into(),
    }
}

/// A signature's parameters as (name, type) and its return type.
fn parts(sig: &str, name: &str) -> (Vec<(String, String)>, String) {
    let Some((_, rest)) = sig.split_once(&format!("fn {name}(")) else { return (vec![], String::new()) };
    let params = language::split_args(rest)
        .into_iter()
        .map(|(_, p)| match split_top(&p, ":") {
            Some((names, t)) => (names.split_whitespace().last().unwrap_or("").to_string(), t.trim().to_string()),
            None => (String::new(), p.trim().to_string()),
        })
        .collect::<Vec<_>>();
    let close = language::split_args(rest).last().map_or(0, |(at, p)| at + p.len());
    let out = rest[close..].trim_start_matches(|c: char| c == ')' || c.is_whitespace());
    (params, out.strip_prefix("->").unwrap_or("").trim().to_string())
}

/// What a function takes, in words: a project type by its name, anything else by its parameter's name.
fn takes(params: &[(String, String)]) -> Vec<String> {
    params
        .iter()
        .map(|(name, t)| match base(t) {
            "Int" | "Float" | "String" | "Bool" if !name.is_empty() => spaced(name.trim_start_matches('_')),
            _ => words(t),
        })
        .collect()
}

/// Project types that functions in at least three modules both take and
/// return: the program's state (D-55.3).
fn state_types(ix: &Index, fns: &[usize]) -> BTreeSet<String> {
    let project: HashSet<&str> = ix.syms.iter().filter(|s| s.kind == Kind::Type).map(|s| s.name.as_str()).collect();
    let mut modules: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for &i in fns {
        let s = &ix.syms[i];
        let (params, out) = parts(&ix.lang.head(&s.item()), &s.name);
        for (_, t) in params {
            let b = base(&t);
            if project.contains(b) && out.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == b) {
                modules.entry(b.to_string()).or_default().insert(&s.module);
            }
        }
    }
    modules.into_iter().filter(|(_, m)| m.len() >= 3).map(|(t, _)| t).collect()
}

/// The requests an entry handles: the constructors of a project type it takes
/// that has at least three carrying data (D-55.2), each with its fields in words.
/// A plain enumeration such as a product category isn't requests.
fn requests(ix: &Index, i: usize, params: &[(String, String)]) -> Vec<Value> {
    for (_, t) in params {
        let Some(&ty) = ix.uses[i].iter().find(|&&j| ix.syms[j].kind == Kind::Type && ix.syms[j].name == base(t)) else { continue };
        let cons = ix.lang.record_fields(&ix.syms[ty].text);
        if cons.iter().filter(|(_, fields)| !fields.is_empty()).count() >= 3 {
            return cons
                .into_iter()
                .map(|(name, labels)| {
                    json!({
                        "name": spoken(&name),
                        "fields": labels
                            .into_iter()
                            .map(|l| l.map_or("a value".into(), |l| spaced(&l)))
                            .collect::<Vec<_>>(),
                    })
                })
                .collect();
        }
    }
    vec![]
}

pub(crate) fn graph_index(ix: &Index, files: &[(String, String)], changed: &HashSet<String>) -> Value {
    let mut nodes = vec![];
    let mut edges = vec![];
    let src = |i: usize| ix.modules.iter().any(|m| m.path == ix.syms[i].module && m.dir == "src");
    let function = |i: usize| ix.syms[i].kind == Kind::Fn;
    let tests = reach::test_symbols(ix);
    let mut visible = HashSet::new();
    let mut stack: Vec<_> = (0..ix.syms.len()).filter(|&i| src(i) && function(i) && ix.syms[i].text.contains("pub fn ")).collect();
    while let Some(i) = stack.pop() {
        if visible.insert(i) {
            stack.extend(ix.uses[i].iter().copied().filter(|&j| src(j) && function(j)));
        }
    }
    let direct: Vec<_> = (0..ix.syms.len()).map(|i| direct_effects(ix, i)).collect();
    let mut used_effects: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut ordered: Vec<usize> = visible.iter().copied().collect();
    ordered.sort();
    let state = state_types(ix, &ordered);
    let mut areas = BTreeSet::new();
    for &i in &ordered {
        let s = &ix.syms[i];
        let entry = s.name == "main" || (s.text.contains("pub fn ") && !ix.used_by[i].iter().any(|&j| src(j)));
        let sig = ix.lang.head(&s.item());
        let summary = s.text.lines().filter_map(|l| l.strip_prefix("///")).map(str::trim).collect::<Vec<_>>().join(" ");
        let count = tests.iter().filter(|&&t| ix.uses[t].contains(&i)).count();
        let (params, out) = parts(&sig, &s.name);
        let public = s.text.contains(&format!("pub fn {}(", s.name));
        areas.insert(s.module.clone());
        nodes.push(json!({
            "id": s.id,
            "kind": if entry { "entry" } else { "logic" },
            "name": s.name,
            "module": s.module,
            "summary": summary,
            "signature": sig,
            "known": "derived",
            "tests": count,
            "changed": changed.contains(&format!("symbol:{}", s.id)),
            "public": public,
            "takes": takes(&params),
            "gives": if out.is_empty() { String::new() } else { words(&out) },
            "requests": if entry { requests(ix, i, &params) } else { vec![] },
        }));
        // State (D-55.3): taken and returned is a change, taken alone a read; only the function's own signature counts.
        for t in params.iter().map(|(_, t)| base(t)).filter(|b| state.contains(*b)).collect::<BTreeSet<_>>() {
            let returned = out.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == t);
            let cat = format!("{} {t}", if returned { "changes" } else { "reads" });
            edges.push(json!({"from":s.id,"to":format!("effect:{cat}"),"kind":"uses","known":"derived","direct":true}));
            used_effects.entry(cat).or_default();
        }
        for &j in &ix.uses[i] {
            if visible.contains(&j) {
                edges.push(json!({"from":s.id,"to":ix.syms[j].id,"kind":"calls","known":"derived","direct":true}));
            }
        }
        let mut reachable = BTreeSet::new();
        let mut queue = vec![i];
        while let Some(j) = queue.pop() {
            if reachable.insert(j) {
                queue.extend(ix.uses[j].iter().copied().filter(|&k| visible.contains(&k)));
            }
        }
        let mut effects = BTreeMap::<String, bool>::new();
        for j in reachable {
            for (cat, calls) in &direct[j] {
                effects.entry(cat.clone()).and_modify(|v| *v |= i == j).or_insert(i == j);
                used_effects.entry(cat.clone()).or_default().extend(calls.iter().cloned());
            }
        }
        for (cat, is_direct) in effects {
            edges.push(json!({"from":s.id,"to":format!("effect:{cat}"),"kind":"uses","known":"derived","direct":is_direct}));
        }
    }
    for (cat, calls) in used_effects {
        let summary = match cat.split_once(' ') {
            Some(("changes", t)) if state.contains(t) => format!("Takes the program's {t} and returns a changed one."),
            Some(("reads", t)) if state.contains(t) => format!("Takes the program's {t} and doesn't return it."),
            _ => String::new(),
        };
        nodes.push(json!({
            "id": format!("effect:{cat}"),
            "kind": "effect",
            "name": cat,
            "module": "",
            "summary": summary,
            "signature": "",
            "known": "derived",
            "tests": 0,
            "changed": false,
            "calls": calls,
        }));
    }
    for &i in &tests {
        let s = &ix.syms[i];
        nodes.push(json!({
            "id": s.id,
            "kind": "test",
            "name": s.name,
            "module": s.module,
            "summary": "",
            "signature": crate::index::signature(ix, s),
            "known": "derived",
            "tests": 0,
            "changed": changed.contains(&format!("symbol:{}", s.id)),
            "checks": checks(&s.text),
        }));
        for &j in &ix.uses[i] {
            if visible.contains(&j) {
                edges.push(json!({"from":s.id,"to":ix.syms[j].id,"kind":"tests","known":"derived","direct":true}));
            }
        }
    }
    for (name, tier) in files {
        let sentence = if name == "gleam.toml" {
            "Watched: the project manifest. Agents can change it if it still parses; changes are undone with the task."
        } else if tier == "watched" {
            "Watched: agents can read and write it; changes are undone with the task."
        } else if tier == "protected" {
            "Protected: agents can read it; their changes wait for your approval."
        } else {
            "Unwatched: Solace doesn't search it as code. Agents can read and write it; their direct edits are undone with the task."
        };
        nodes.push(json!({
            "id": format!("file:{name}"),
            "kind": "file",
            "name": name,
            "module": "",
            "summary": "",
            "signature": "",
            "known": "derived",
            "tests": 0,
            "changed": changed.contains(&format!("file:{name}")),
            "tier": tier,
            "sentence": sentence,
        }));
    }
    nodes.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    // Areas (D-55.1): each source module with a function in the graph, its doc comment as the summary.
    let areas: Vec<Value> = areas
        .into_iter()
        .map(|m| {
            let header = ix.modules.iter().find(|r| r.path == m).map_or("", |r| r.header.as_str());
            let doc: Vec<&str> = header.lines().filter_map(|l| l.strip_prefix("////")).map(str::trim).collect();
            let summary = doc.split(|l| l.is_empty()).next().unwrap_or(&[]).join(" ");
            json!({"id":m,"name":spaced(m.rsplit('/').next().unwrap_or(&m)),"summary":summary})
        })
        .collect();
    json!({"nodes":nodes,"edges":edges,"areas":areas})
}
