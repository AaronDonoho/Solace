use super::*;
use std::collections::{BTreeMap, BTreeSet};

// ponytail: effects inside dependencies are seen only through this table; extend by settings later.
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
pub(super) fn direct_effects(ix: &Index, i: usize) -> BTreeMap<String, Vec<String>> {
    let s = &ix.syms[i];
    let imports = ix.modules.iter().find(|m| m.path == s.module).map(|m| gleam::imports(&m.header)).unwrap_or_default();
    let mut effects: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in gleam::references(&s.text) {
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
                return json!({"call":chars[..i].iter().collect::<String>().trim(),"expected":chars[i+2..].iter().collect::<String>().trim()});
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
fn words(t: &str) -> String {
    let t = t.trim();
    let args = |inner: &str| gleam::split_args(inner).into_iter().map(|(_, a)| words(&a)).collect::<Vec<_>>();
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
    let params = gleam::split_args(rest)
        .into_iter()
        .map(|(_, p)| match split_top(&p, ":") {
            Some((names, t)) => (names.split_whitespace().last().unwrap_or("").to_string(), t.trim().to_string()),
            None => (String::new(), p.trim().to_string()),
        })
        .collect::<Vec<_>>();
    let close = gleam::split_args(rest).last().map_or(0, |(at, p)| at + p.len());
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
        let (params, out) = parts(&gleam::head(&s.item()), &s.name);
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
        let cons = gleam::record_fields(&ix.syms[ty].text);
        if cons.iter().filter(|(_, fields)| !fields.is_empty()).count() >= 3 {
            return cons
                .into_iter()
                .map(|(name, labels)| json!({"name":spoken(&name),"fields":labels.into_iter().map(|l| l.map_or("a value".into(), |l| spaced(&l))).collect::<Vec<_>>()}))
                .collect();
        }
    }
    vec![]
}
fn graph_index(ix: &Index, files: &[(String, String)], changed: &HashSet<String>) -> Value {
    let mut nodes = vec![];
    let mut edges = vec![];
    let src = |i: usize| ix.modules.iter().any(|m| m.path == ix.syms[i].module && m.dir == "src");
    let function = |i: usize| ix.syms[i].kind == Kind::Fn;
    let tests = reach::tests(ix);
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
        let sig = gleam::head(&s.item());
        let summary = s.text.lines().filter_map(|l| l.strip_prefix("///")).map(str::trim).collect::<Vec<_>>().join(" ");
        let count = tests.iter().filter(|&&t| ix.uses[t].contains(&i)).count();
        let (params, out) = parts(&sig, &s.name);
        let public = s.text.contains(&format!("pub fn {}(", s.name));
        areas.insert(s.module.clone());
        nodes.push(json!({"id":s.id,"kind":if entry {"entry"} else {"logic"},"name":s.name,"module":s.module,"summary":summary,"signature":sig,"known":"derived","tests":count,"changed":changed.contains(&format!("symbol:{}",s.id)),
            "public":public,"takes":takes(&params),"gives":if out.is_empty() {String::new()} else {words(&out)},"requests":if entry {requests(ix, i, &params)} else {vec![]}}));
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
        nodes.push(json!({"id":format!("effect:{cat}"),"kind":"effect","name":cat,"module":"","summary":summary,"signature":"","known":"derived","tests":0,"changed":false,"calls":calls}));
    }
    for &i in &tests {
        let s = &ix.syms[i];
        nodes.push(json!({"id":s.id,"kind":"test","name":s.name,"module":s.module,"summary":"","signature":gleam::signature(&s.item()),"known":"derived","tests":0,"changed":changed.contains(&format!("symbol:{}",s.id)),"checks":checks(&s.text)}));
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
        nodes.push(json!({"id":format!("file:{name}"),"kind":"file","name":name,"module":"","summary":"","signature":"","known":"derived","tests":0,"changed":changed.contains(&format!("file:{name}")),"tier":tier,"sentence":sentence}));
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
/// A task as this project sees it (D-61): only the changes made here.
fn changed_here(s: &Store, mut t: Value) -> Value {
    let here = scope::dir(&scope::top(&s.root), &s.root);
    let mine: Vec<Value> = board::changes_of(s, &t).unwrap_or_default().into_iter().filter(|c| c["project"] == here).collect();
    t["changes"] = json!(mine);
    t
}
/// A task found from any project in its scope, as this project sees it, with its home.
fn task_here(s: &Store, tid: &str) -> Result<(PathBuf, Value), Failure> {
    let home = scope::home(&s.root, tid)?;
    let t = board::task(&home, tid)?;
    Ok((home.clone(), changed_here(s, t)))
}
pub fn graph(s: &Store) -> Result<Value, Failure> {
    let ix = s.index()?;
    let files = lockdown::tree_files(&s.root, &lockdown::settings(&s.root)?)?;
    let mut changed = HashSet::new();
    let w = board::flow(&s.root)?;
    let top = scope::top(&s.root);
    for (_, t) in scope::tasks_in(&top, &scope::dir(&top, &s.root))? {
        let t = changed_here(s, t);
        if w["columns"].as_array().unwrap().iter().any(|c| c["id"] == t["column"] && c["final"] == true) {
            continue;
        }
        for c in t["changes"].as_array().unwrap_or(&vec![]) {
            if let Some(e) = c["entity"].as_str() {
                changed.insert(e.to_string());
            }
        }
    }
    let mut graph = graph_index(&ix, &files, &changed);
    let name = Some(workspace::name(&s.root))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| s.root.file_name().unwrap_or_default().to_string_lossy().into_owned());
    graph["project"] = json!({"name": name, "fingerprint": board::fingerprint(s)?});
    Ok(graph)
}

#[derive(Clone, Copy)]
enum Version<'a> {
    Current,
    Before(&'a str),
}
fn version(s: &Store, v: Version<'_>) -> Result<Index, Failure> {
    let mut rows = board::snapshot(s)?;
    if let Version::Before(tid) = v {
        let mut st = s.db.prepare("SELECT entity,data FROM board_before WHERE task=?1")?;
        for row in st.query_map([tid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (key, data) = row?;
            rows[&key] = serde_json::from_str(&data).map_err(|e| Failure(e.to_string(), 1))?;
        }
    }
    let mut syms = vec![];
    let mut modules = vec![];
    for (key, row) in rows.as_object().unwrap() {
        if row.is_null() {
            continue;
        }
        if key.starts_with("symbol:") {
            syms.push(Sym {
                id: row[0].as_str().unwrap().into(),
                module: row[1].as_str().unwrap().into(),
                name: row[2].as_str().unwrap().into(),
                kind: Kind::parse(row[3].as_str().unwrap()),
                text: row[4].as_str().unwrap().into(),
            });
        } else if key.starts_with("module:") {
            modules.push(ModuleRow {
                path: row[0].as_str().unwrap().into(),
                dir: row[1].as_str().unwrap().into(),
                header: row[2].as_str().unwrap().into(),
            });
        }
    }
    syms.sort_by(|a, b| a.module.cmp(&b.module).then(a.name.cmp(&b.name)));
    Ok(Index::build(syms, modules))
}
/// What the task's changes reach (D-44): its tests (those it changed, and
/// those that reach a symbol it changed or read a field the change reaches),
/// and the uses no test checks, with declared suites and where data leaves.
fn task_reach(s: &Store, old: &Index, now: &Index, task: &Value) -> Result<Value, Failure> {
    let ids: Vec<String> = task["changes"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|c| c["entity"].as_str().and_then(|x| x.strip_prefix("symbol:")).map(String::from))
        .collect();
    let before: Before = ids.iter().map(|id| (id.clone(), old.syms.iter().find(|s| &s.id == id).map(|s| s.text.clone()))).collect();
    let impact = reach::flow(now, &reach::origins(now, &before));
    let changed: Vec<usize> = (0..now.syms.len()).filter(|&i| ids.contains(&now.syms[i].id)).collect();
    let readers = reach::readers(now, &impact.fields);
    let tests: Vec<&str> = reach::tests(now)
        .into_iter()
        .filter(|&t| {
            changed.contains(&t) || readers.contains(&t) || !reach::closure(now, t).is_disjoint(&changed.iter().copied().collect())
        })
        .map(|t| now.syms[t].id.as_str())
        .collect();
    let untested: Vec<Value> = reach::ranked(now, &impact)
        .into_iter()
        .filter(|(_, t)| t.is_empty())
        .map(|(u, _)| json!({"id":now.syms[u.sym].id,"name":reach::short(now,u.sym),"steps":u.steps,"via":u.via,"certain":u.certain}))
        .collect();
    let syms: Vec<usize> = impact.uses.iter().map(|u| u.sym).collect();
    let suites = reach::reached(&reach::suites(s, now)?, &syms);
    let leaves: Vec<String> = impact.leaves.iter().map(|(i, e)| format!("{} ({e})", reach::short(now, *i))).collect();
    Ok(json!({"tests":tests,"untested":untested,"suites":suites,"leaves":leaves}))
}
fn review_graphs(before: &Value, after: &Value, task: &Value, reach: &Value, settings: &toml::Value, root: &Path) -> Value {
    let nodes = |g: &Value| -> BTreeMap<String, Value> {
        g["nodes"].as_array().unwrap().iter().map(|n| (n["id"].as_str().unwrap().into(), n.clone())).collect()
    };
    let b = nodes(before);
    let a = nodes(after);
    let effects = |g: &Value, id: &str| -> BTreeSet<String> {
        g["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["from"] == id && e["kind"] == "uses")
            .filter_map(|e| e["to"].as_str().map(String::from))
            .collect()
    };
    let mut entries = vec![];
    let mut signatures = vec![];
    for (id, node) in &a {
        if node["kind"] != "entry" {
            continue;
        }
        let old = b.get(id);
        let added = effects(after, id);
        let removed = old.map(|_| effects(before, id)).unwrap_or_default();
        let add: Vec<_> = added.difference(&removed).cloned().collect();
        let del: Vec<_> = removed.difference(&added).cloned().collect();
        let sig_changed = old.is_some_and(|o| o["signature"] != node["signature"]);
        if old.is_none() || !add.is_empty() || !del.is_empty() || sig_changed {
            entries.push(json!({"id":id,"change":if old.is_none(){"added"}else{"changed"},"effects_added":add,"effects_removed":del}));
        }
    }
    for (id, node) in &b {
        if node["kind"] == "entry" && !a.contains_key(id) {
            entries.push(
                json!({"id":id,"change":"removed","effects_added":[],"effects_removed":effects(before,id),"removed":true,"node":node}),
            );
        }
    }
    for (id, node) in &a {
        if let Some(old) = b.get(id)
            && old["signature"] != node["signature"]
            && matches!(node["kind"].as_str(), Some("entry" | "logic"))
        {
            signatures.push(json!({"id":id,"before":old["signature"],"after":node["signature"]}));
        }
    }
    let before_effects: BTreeSet<_> = b.keys().filter(|k| k.starts_with("effect:")).cloned().collect();
    let after_effects: BTreeSet<_> = a.keys().filter(|k| k.starts_with("effect:")).cloned().collect();
    let effects_added: Vec<_> = after_effects.difference(&before_effects).cloned().collect();
    let effects_removed: Vec<_> = before_effects.difference(&after_effects).cloned().collect();
    let mut files = vec![];
    for change in task["changes"].as_array().unwrap_or(&vec![]) {
        if let Some(name) = change["entity"].as_str().filter(|k| board::reviewable(root, k)).and_then(|x| x.strip_prefix("file:")) {
            let path = root.join(name);
            let before_path = board::before_file(root, task["id"].as_str().unwrap(), name);
            let change = if !path.exists() {
                "removed"
            } else if before_path.with_extension("absent").exists() {
                "added"
            } else {
                "changed"
            };
            files.push(json!({"id":format!("file:{name}"),"tier":lockdown::tier(root,settings,name,None),"change":change}));
        }
    }
    let changed: HashSet<_> = task["changes"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|c| c["entity"].as_str().and_then(|x| x.strip_prefix("symbol:")).map(String::from))
        .collect();
    let tested: HashSet<_> = reach["tests"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|id| id.as_str().map(String::from))
        .chain(changed.iter().cloned())
        .collect();
    let stored = &task["review_tests"];
    let test_results = stored["results"].as_array().cloned().unwrap_or_default();
    let tests: Vec<_> = a
        .iter()
        .filter(|(id, n)| n["kind"] == "test" && tested.contains(*id))
        .map(|(id, n)| {
            let mut result = test_results
                .iter()
                .find(|r| r["id"] == *id)
                .cloned()
                .unwrap_or_else(|| json!({"id":id,"name":n["name"],"result":"not run yet","checks":n["checks"]}));
            // run_tests matches failures by module and name.
            result["module"] = n["module"].clone();
            if let Some(old) = b.get(id) {
                let mut old_checks = BTreeMap::new();
                for c in old["checks"].as_array().unwrap_or(&vec![]) {
                    if let Some(call) = c["call"].as_str() {
                        old_checks.insert(call.to_string(), c["expected"].clone());
                    }
                }
                for c in result["checks"].as_array_mut().unwrap() {
                    if let Some(call) = c["call"].as_str()
                        && let Some(was) = old_checks.get(call)
                        && *was != c["expected"]
                    {
                        c["was"] = was.clone();
                    }
                }
            }
            result
        })
        .collect();
    let unfinished = tests.iter().filter(|t| t["result"] == "unfinished").count();
    let added_count: usize = entries.iter().map(|e: &Value| e["effects_added"].as_array().map_or(0, Vec::len)).sum();
    let untested = reach["untested"].as_array().map_or(0, Vec::len);
    let summary = format!(
        "{} effects added, {} signature changed, {} test unfinished, {} untested uses{}",
        added_count,
        signatures.len(),
        unfinished,
        untested,
        if stored["state"].is_null() { ", tests not run yet" } else { "" }
    );
    json!({"entries":entries,"effects":{"added":effects_added,"removed":effects_removed},"signatures":signatures,"tests":tests,"files":files,
        "untested":reach["untested"],"suites":reach["suites"],"leaves":reach["leaves"],"summary":summary})
}
pub fn review(s: &Store, task: &Value) -> Result<Value, Failure> {
    let tid = task["id"].as_str().unwrap();
    let files = lockdown::tree_files(&s.root, &lockdown::settings(&s.root)?)?;
    let (old, now) = (version(s, Version::Before(tid))?, version(s, Version::Current)?);
    let before = graph_index(&old, &files, &HashSet::new());
    let after = graph_index(&now, &files, &HashSet::new());
    let reach = task_reach(s, &old, &now, task)?;
    let mut t = task.clone();
    let base = board::fingerprint(s)?;
    if t["review_tests"]["base"] != base {
        t["review_tests"] = Value::Null;
    }
    Ok(review_graphs(&before, &after, &t, &reach, &lockdown::settings(&s.root)?, &s.root))
}
fn copy_dir(from: &Path, to: &Path) -> Result<(), Failure> {
    if !from.exists() {
        return Ok(());
    }
    fs::create_dir_all(to)?;
    migrate::copy_tree(from, to, &|_| false)
}
/// A copy of the project as it is now, for running review tests away from the real one.
fn scratch(s: &Store, dir: &Path) -> Result<(), Failure> {
    s.materialize_to(&version(s, Version::Current)?, dir)?;
    for name in ["gleam.toml", "manifest.toml"] {
        if s.root.join(name).exists() {
            fs::copy(s.root.join(name), dir.join(name))?;
        }
    }
    // Tests may read data and fixtures, text or binary.
    for (name, tier) in lockdown::tree_files(&s.root, &lockdown::settings(&s.root)?)? {
        if tier == "watched" || tier == "unwatched (binary)" {
            let dest = dir.join(&name);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(s.root.join(&name), dest)?;
        }
    }
    copy_dir(&s.root.join("build/packages"), &dir.join("build/packages"))?;
    copy_dir(&s.root.join("build/dev"), &dir.join("build/dev"))?;
    Ok(())
}
pub fn run_tests(root: PathBuf, tid: String) {
    std::thread::spawn(move || {
        let result = (|| -> Result<(), Failure> {
            let dir = env::temp_dir().join(format!("solace-view-{}", unique_id()));
            // The task may be stored in another project of its scope (D-61).
            // ponytail: one review_tests per task; tests run from another project replace them.
            let home = scope::home(&root, &tid)?;
            let (base, tests) = {
                let _guard = scope::lock_all(&[root.clone(), home.clone()])?;
                let s = board::open(&root)?;
                let (_, here) = task_here(&s, &tid)?;
                let base = board::fingerprint(&s)?;
                scratch(&s, &dir)?;
                let r = review(&s, &here)?;
                let running = json!({"base":base,"state":"running","results":[]});
                tasks::update(&home, &tid, move |t| {
                    t["review_tests"] = running;
                    Ok(())
                })?;
                (base, r["tests"].as_array().cloned().unwrap_or_default())
            };
            let output = lockdown::command(&dir, &["gleam".into(), "test".into()], 300, 100_000);
            let _ = fs::remove_dir_all(&dir);
            let (state, results) = match output {
                Ok(log) => {
                    let failed = failures(&log);
                    let compile = log.contains("error:") && failed.is_empty();
                    let results = tests
                        .iter()
                        .map(|t| {
                            let id = t["id"].as_str().unwrap();
                            let name = t["name"].as_str().unwrap();
                            let module = t["module"].as_str().unwrap_or("");
                            let key = format!("{}.{}", module, name);
                            let (result, message) = failed.get(&key).cloned().unwrap_or((
                                if compile { "failed to compile" } else { "passing" }.into(),
                                if compile { log.clone() } else { String::new() },
                            ));
                            json!({"id":id,"name":name,"result":result,"message":message,"checks":t["checks"]})
                        })
                        .collect::<Vec<_>>();
                    (if compile { "failed to compile" } else { "done" }, results)
                }
                Err(e) => (
                    "failed to compile",
                    tests
                        .iter()
                        .map(|t| json!({"id":t["id"],"name":t["name"],"result":"failed to compile","message":e.0,"checks":t["checks"]}))
                        .collect(),
                ),
            };
            let done = json!({"base":base,"state":state,"results":results});
            tasks::update(&home, &tid, move |t| {
                t["review_tests"] = done;
                Ok(())
            })
        })();
        if let Err(e) = result {
            eprintln!("review tests: {}", e.0);
        }
    });
}

/// `%xx` escapes in a URL path decoded; invalid UTF-8 replaced.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (vec![], 0);
    while i < b.len() {
        match (b[i], b.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())) {
            (b'%', Some(byte)) => (out.push(byte), i += 3),
            (c, _) => (out.push(c), i += 1),
        };
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn api(s: &Store, method: &str, parts: &[&str], query: &HashMap<String, String>, body: &Value) -> Result<Value, Failure> {
    match (method, parts) {
        ("GET", ["graph"]) => {
            let mut g = graph(s)?;
            if let Some(tid) = query.get("task") {
                let (_, t) = task_here(s, tid)?;
                let files = lockdown::tree_files(&s.root, &lockdown::settings(&s.root)?)?;
                let before = graph_index(&version(s, Version::Before(tid))?, &files, &HashSet::new());
                let current: HashSet<_> =
                    g["nodes"].as_array().unwrap().iter().filter_map(|n| n["id"].as_str().map(String::from)).collect();
                let nodes = g["nodes"].as_array_mut().unwrap();
                for old in before["nodes"].as_array().unwrap() {
                    if !current.contains(old["id"].as_str().unwrap()) {
                        let mut removed = old.clone();
                        removed["removed"] = json!(true);
                        nodes.push(removed);
                    }
                }
                let changed: HashSet<_> =
                    t["changes"].as_array().unwrap_or(&vec![]).iter().filter_map(|c| c["entity"].as_str().map(String::from)).collect();
                for node in nodes {
                    if let Some(id) = node["id"].as_str() {
                        node["changed"] = json!(changed.contains(&format!("symbol:{id}")) || changed.contains(id));
                    }
                }
                g["review"] = review(s, &t)?;
            }
            Ok(g)
        }
        ("GET", ["nodes", id]) if query.get("source").is_some_and(|v| v == "1") => {
            // A symbol's text; with a task, also the line diff from before the task (removed symbols too).
            let text = |v| -> Result<Option<String>, Failure> {
                Ok(version(s, v)?.syms.into_iter().find(|sym| sym.id == *id).map(|sym| sym.text))
            };
            let now = text(Version::Current)?;
            let before = match query.get("task") {
                Some(tid) => text(Version::Before(tid))?,
                None => None,
            };
            if now.is_none() && before.is_none() {
                return board::err("E7001", "unknown node");
            }
            let mut out = json!({"source": now.clone().unwrap_or_default()});
            if query.contains_key("task") {
                let ops = board::diff_ops(before.as_deref().unwrap_or(""), now.as_deref().unwrap_or(""));
                out["diff"] = json!(ops.iter().map(|(op, line)| [op.to_string(), line.to_string()]).collect::<Vec<_>>());
            }
            Ok(out)
        }
        ("GET", ["files", rest @ ..]) => {
            // Read-only text of a file the files panel lists; never graph or internal files.
            let name = percent_decode(&rest.join("/"));
            let files = lockdown::tree_files(&s.root, &lockdown::settings(&s.root)?)?;
            let Some((_, category)) = files.iter().find(|(f, _)| *f == name && !f.ends_with('/')) else {
                return board::err("E7001", "unknown file");
            };
            let bytes = fs::read(lockdown::path(&s.root, &name)?)?;
            const SHOWN: usize = 1 << 20;
            let cut = &bytes[..bytes.len().min(SHOWN)];
            let text = match std::str::from_utf8(cut) {
                Ok(t) if !t.contains('\0') => Some(t),
                // A cut through a multi-byte character at the end isn't binary.
                Err(e) if cut.len() < bytes.len() && cut.len() - e.valid_up_to() < 4 => std::str::from_utf8(&cut[..e.valid_up_to()]).ok(),
                _ => None,
            };
            Ok(
                json!({"path": name, "category": category, "text": text, "size": bytes.len(), "truncated": text.is_some() && cut.len() < bytes.len()}),
            )
        }
        ("GET", ["nodes", id]) => {
            let g = graph(s)?;
            let mut node = g["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["id"] == *id)
                .cloned()
                .ok_or_else(|| Failure("E7001 unknown node".into(), 1))?;
            let edges = g["edges"].as_array().unwrap();
            for (field, kind, reverse) in
                [("callers", "calls", true), ("callees", "calls", false), ("effects", "uses", false), ("tests", "tests", true)]
            {
                node[field] = json!(
                    edges
                        .iter()
                        .filter(|e| e["kind"] == kind && if reverse { e["to"] == *id } else { e["from"] == *id })
                        .map(|e| if reverse { e["from"].clone() } else { e["to"].clone() })
                        .collect::<Vec<_>>()
                );
            }
            // Examples (D-55.5): the checks of the tests that test this function and call it.
            let call = format!("{}(", node["name"].as_str().unwrap_or(""));
            let examples: Vec<Value> = node["tests"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|t| g["nodes"].as_array().unwrap().iter().find(|n| n["id"] == *t))
                .flat_map(|t| t["checks"].as_array().cloned().unwrap_or_default())
                .filter(|c| c["call"].as_str().is_some_and(|c| c.contains(&call)))
                .take(6)
                .collect();
            node["examples"] = json!(examples);
            Ok(node)
        }
        ("POST", ["tests"]) => {
            let tid = body["task"].as_str().ok_or_else(|| Failure("E7001 task required".into(), 1))?;
            task_here(s, tid)?;
            run_tests(s.root.clone(), tid.into());
            Ok(json!({"state":"running"}))
        }
        _ => board::err("E7001", "unknown route"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::*;
    #[test]
    fn c1_graph_has_entries_effects_and_tests() {
        let ix = test_index(&[
            (
                "src",
                "app",
                r#"import gleam/io
@external(erlang, "file", "write_file")
fn write(path: String, text: String) -> Nil
@external(erlang, "init", "get_plain_arguments")
fn args() -> List(String)
pub fn main() {
  let xs = args()
  io.println("hello")
  write("out", "hi")
}"#,
            ),
            (
                "test",
                "app_test",
                r#"import app
pub fn main_test() {
  assert app.main() == Nil
}"#,
            ),
        ]);
        let g = graph_index(&ix, &[], &HashSet::new());
        assert!(g["nodes"].as_array().unwrap().iter().any(|n| n["id"] == "fn:app.main" && n["kind"] == "entry"));
        for cat in ["prints", "writes files", "environment"] {
            assert!(g["edges"].as_array().unwrap().iter().any(|e| e["from"] == "fn:app.main" && e["to"] == format!("effect:{cat}")));
        }
        assert!(g["edges"].as_array().unwrap().iter().any(|e| e["kind"] == "tests" && e["to"] == "fn:app.main"));
    }
    #[test]
    fn c2_pilot_date_check() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../bench/pilot-large");
        let mut sources = vec![];
        let mut owned = vec![];
        for dir in ["src", "test"] {
            let mut paths = vec![];
            collect_gleam_files(&root.join(dir), &mut paths).unwrap();
            for path in paths {
                let module = path.strip_prefix(root.join(dir)).unwrap().with_extension("").to_string_lossy().to_string();
                owned.push((dir.to_string(), module, fs::read_to_string(path).unwrap()));
            }
        }
        for (dir, path, text) in &owned {
            sources.push((dir.as_str(), path.as_str(), text.as_str()));
        }
        let g = graph_index(&test_index(&sources), &[], &HashSet::new());
        assert!(g["nodes"].as_array().unwrap().iter().any(|n| n["module"] == "pilot/api" && n["name"] == "handle" && n["kind"] == "entry"));
        assert!(g["nodes"].as_array().unwrap().iter().any(|n| {
            n["module"] == "pilot/date_test"
                && n["checks"].as_array().is_some_and(|cs| {
                    cs.iter().any(|c| c["call"] == "date.add_business_days(Date(2026, 3, 5), 2)" && c["expected"] == "Date(2026, 3, 9)")
                })
        }));
    }
    #[test]
    fn c4_files_match_cli_listing() {
        let p = Scratch::new()
            .with(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("solace.toml", "unwatched=[\"priv/gen/\"]\n"), ("priv/a", "hello")])
            .import();
        let root = &p.root;
        let settings = lockdown::settings(root).unwrap();
        let files = lockdown::tree_files(root, &settings).unwrap();
        let mut listing = String::new();
        lockdown::dispatch(&board::open(root).unwrap(), &settings, &["files"], &mut || Ok(String::new()), None, 1000)
            .unwrap()
            .map(|s| listing = s)
            .unwrap();
        for (name, tier) in files {
            assert!(listing.contains(&format!("{name} [{tier}]\n")));
        }
    }
    #[test]
    fn plain_words_requests_state_and_areas() {
        assert_eq!(words("Result(List(Order), Error)"), "a list of Order or an error");
        assert_eq!(words("#(Store, Response)"), "Store and Response");
        assert_eq!(words("option.Option(Int)"), "a number or nothing");
        assert_eq!(words("Result(Nil, String)"), "success or an error");
        assert_eq!(words("Dict(String, Int)"), "a number by text");
        let changer = |m: &str| {
            format!("import app/db.{{type Db}}\npub fn change(d: Db, order_id: Int) -> Db {{ d }}\npub fn read_{m}(d: Db) -> Int {{ 1 }}")
        };
        let (a, b) = (changer("a"), changer("b"));
        let ix = test_index(&[
            (
                "src",
                "app/db",
                "//// The program's data.\n////\n//// More detail.\npub type Db {\n  Db(n: Int)\n}\npub fn new() -> Db { Db(0) }",
            ),
            ("src", "app/a", &a),
            ("src", "app/b", &b),
            (
                "src",
                "app/api",
                "import app/db.{type Db}\npub type Colour {\n  Red\n  Green\n  Blue\n}\npub fn paint(c: Colour) -> Int { 1 }\npub type Request {\n  CreateOrder(customer_id: Int)\n  AddItem(order_id: Int, sku: String)\n  Cancel(Int)\n}\npub fn handle(d: Db, request: Request) -> #(Db, String) { #(d, \"\") }",
            ),
        ]);
        let g = graph_index(&ix, &[], &HashSet::new());
        let node = |module: &str, name: &str| {
            g["nodes"].as_array().unwrap().iter().find(|n| n["module"] == module && n["name"] == name).unwrap().clone()
        };
        let handle = node("app/api", "handle");
        assert_eq!(handle["takes"], json!(["Db", "Request"]));
        assert_eq!(handle["gives"], "Db and text");
        assert_eq!(
            handle["requests"],
            json!([{"name":"Create order","fields":["customer id"]},{"name":"Add item","fields":["order id","sku"]},{"name":"Cancel","fields":["a value"]}])
        );
        assert_eq!(node("app/api", "paint")["requests"], json!([]));
        assert_eq!(node("app/a", "change")["takes"], json!(["Db", "order id"]));
        // Db is taken and returned in three modules (a, b, api), so it is state: a change or a read.
        let effect = |from: &Value, to: &str| {
            g["edges"].as_array().unwrap().iter().any(|e| e["from"] == from["id"] && e["to"] == to && e["kind"] == "uses")
        };
        assert!(effect(&handle, "effect:changes Db"));
        assert!(effect(&node("app/a", "change"), "effect:changes Db"));
        assert!(effect(&node("app/b", "read_b"), "effect:reads Db"));
        assert!(!effect(&node("app/db", "new"), "effect:changes Db"));
        assert!(
            g["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["id"] == "effect:changes Db" && n["summary"].as_str().unwrap().contains("returns a changed one"))
        );
        assert_eq!(g["areas"][3], json!({"id":"app/db","name":"db","summary":"The program's data."}));
        // Two modules aren't enough to call a type state.
        let ix = test_index(&[("src", "app/db", "pub type Db {\n  Db(n: Int)\n}"), ("src", "app/a", &a), ("src", "app/b", &b)]);
        assert!(!graph_index(&ix, &[], &HashSet::new())["nodes"].as_array().unwrap().iter().any(|n| n["kind"] == "effect"));
    }
    #[test]
    fn graph_api_routes() {
        let p = Scratch::new()
            .with(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("gleam.toml", "name=\"fixture\"\nversion=\"1.0.0\"\n")])
            .import();
        let root = p.root.clone();
        let g = board::api(&root, "GET", "/graph", json!({})).unwrap();
        assert_eq!(g["project"]["name"], "fixture");
        assert_eq!(g["project"]["fingerprint"].as_str().unwrap().len(), 64);
        let expected = board::fingerprint(&board::open(&root).unwrap()).unwrap();
        assert_eq!(g["project"]["fingerprint"], expected);
        let id = g["nodes"].as_array().unwrap().iter().find(|n| n["kind"] == "entry").unwrap()["id"].as_str().unwrap();
        let node = board::api(&root, "GET", &format!("/nodes/{id}?source=1"), json!({})).unwrap();
        assert!(node["source"].as_str().unwrap().contains("pub fn main"));
        // With a task, the source comes with its line diff from before the task.
        let store = board::open(&root).unwrap();
        let mut old = board::snapshot(&store).unwrap()[format!("symbol:{id}")].clone();
        old[4] = json!("pub fn main() {\n  0\n}");
        store.db.execute("INSERT INTO board_before VALUES ('t1',?1,?2)", [format!("symbol:{id}"), old.to_string()]).unwrap();
        let diff = board::api(&root, "GET", &format!("/nodes/{id}?source=1&task=t1"), json!({})).unwrap();
        assert_eq!(diff["diff"], json!([["-", "pub fn main() {"], ["-", "  0"], ["-", "}"], ["+", "pub fn main() { 1 }"]]));
        assert!(board::api(&root, "GET", "/nodes/fn:app.nope?source=1&task=t1", json!({})).is_err());
        // Files: listed ones only, read-only text; graph, internal and unlisted paths are refused.
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("docs/read me.md"), "hello\n").unwrap();
        fs::write(root.join("docs/logo.bin"), b"\0\x01").unwrap();
        let f = board::api(&root, "GET", "/files/docs/read%20me.md", json!({})).unwrap();
        assert_eq!((f["text"].as_str(), f["category"].as_str()), (Some("hello\n"), Some("watched")));
        assert!(board::api(&root, "GET", "/files/docs/logo.bin", json!({})).unwrap()["text"].is_null());
        for bad in ["/files/src/app.gleam", "/files/.solace/store.db", "/files/../etc/passwd", "/files/docs/"] {
            assert!(board::api(&root, "GET", bad, json!({})).is_err(), "{bad}");
        }
        fs::write(root.join("gleam.toml"), "version=\"1.0.0\"\n").unwrap();
        let fallback = board::api(&root, "GET", "/graph", json!({})).unwrap();
        assert_eq!(fallback["project"]["name"], root.file_name().unwrap().to_str().unwrap());
        assert!(board::api(&root, "GET", "/graph?task=no-such-task", json!({})).unwrap_err().0.contains("E7001"));
    }
    #[test]
    fn c3_review_shows_a_new_reachable_write() {
        let before = graph_index(&test_index(&[("src", "app", "pub fn main() { work() }\nfn work() { 1 }")]), &[], &HashSet::new());
        let after = graph_index(
            &test_index(&[("src", "app", "import simplifile\npub fn main() { work() }\nfn work() { simplifile.write(\"a\", \"b\") }")]),
            &[],
            &HashSet::new(),
        );
        let task = json!({"id":"t","changes":[{"entity":"symbol:fn:app.work"}],"review_tests":null});
        let r = review_graphs(
            &before,
            &after,
            &task,
            &json!({"tests":[],"untested":[{"id":"fn:app.main"}]}),
            &"".parse().unwrap(),
            Path::new("/tmp"),
        );
        assert_eq!(r["effects"]["added"], json!(["effect:writes files"]));
        assert!(r["summary"].as_str().unwrap().contains("1 untested uses"), "{}", r["summary"]);
        assert!(
            r["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["id"] == "fn:app.main" && e["effects_added"] == json!(["effect:writes files"]))
        );
    }
    /// Real `gleam test` output, colors and all: a failed assert, a `todo`
    /// and a passing test.
    #[test]
    fn gleam_test_output_states() {
        let failed = failures(include_str!("../tests/golden/gleam-test-failures.txt"));
        assert_eq!(failed["app_test.wrong_test"].0, "wrong");
        assert!(failed["app_test.wrong_test"].1.contains("left: 2\nright: 3"), "{failed:?}");
        assert_eq!(failed["app_test.todo_test"].0, "unfinished");
        assert!(!failed.contains_key("app_test.passing_test"));
    }
    /// `%xx` decodes; a byte that isn't UTF-8 is replaced, and a non-hex
    /// escape stays as written.
    #[test]
    fn percent_decoding_replaces_invalid_utf8() {
        assert_eq!(percent_decode("%41"), "A");
        assert_eq!(percent_decode("a%41b"), "aAb");
        assert_eq!(percent_decode("%E9"), "\u{FFFD}");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    /// A file shown up to 1 MiB: a cut through a multi-byte character at the
    /// end is text, not binary.
    #[test]
    fn a_cut_through_a_multibyte_character_is_still_text() {
        let p = Scratch::new()
            .with(&[("src/app.gleam", "pub fn main() { 1 }\n"), ("solace.toml", "watch_limit = 1100000\nunwatched = []\n")])
            .import();
        let root = p.root.clone();
        fs::create_dir_all(root.join("docs")).unwrap();
        let mut bytes = vec![b'a'; (1 << 20) - 1];
        bytes.extend("é".bytes());
        fs::write(root.join("docs/big.txt"), &bytes).unwrap();
        let f = board::api(&root, "GET", "/files/docs/big.txt", json!({})).unwrap();
        assert_eq!(f["size"], json!((1 << 20) + 1));
        assert_eq!(f["truncated"], json!(true));
        assert_eq!(f["text"].as_str().unwrap().len(), (1 << 20) - 1, "the cut drops the partial character");
    }

    /// `/nodes/{id}` without `source`: the neighbours from the graph's edges,
    /// and at most six examples from the tests that call it.
    #[test]
    fn node_details_list_neighbours_and_examples() {
        let p = Scratch::new()
            .with(&[
                ("src/app.gleam", "pub fn main(n: Int) -> Int {\n  helper(n)\n}\n\nfn helper(n: Int) -> Int {\n  n\n}\n"),
                (
                    "test/app_test.gleam",
                    "import app\n\npub fn main_test() {\n  assert app.main(1) == 1\n  assert app.main(2) == 2\n  assert app.main(3) == 3\n  assert app.main(4) == 4\n  assert app.main(5) == 5\n  assert app.main(6) == 6\n  assert app.main(7) == 7\n}\n",
                ),
                ("gleam.toml", "name=\"fixture\"\nversion=\"1.0.0\"\n"),
            ])
            .import();
        let root = p.root.clone();
        let g = board::api(&root, "GET", "/graph", json!({})).unwrap();
        let id_of = |name: &str| g["nodes"].as_array().unwrap().iter().find(|n| n["name"] == name).unwrap()["id"].as_str().unwrap().to_string();
        let node = board::api(&root, "GET", &format!("/nodes/{}", id_of("main")), json!({})).unwrap();
        assert_eq!(node["callees"], json!([id_of("helper")]));
        assert_eq!(node["callers"], json!([]));
        assert_eq!(node["tests"], json!([id_of("main_test")]));
        let examples = node["examples"].as_array().unwrap();
        assert_eq!(examples.len(), 6, "seven checks, six shown: {examples:?}");
        assert_eq!(examples[0]["call"], json!("app.main(1)"));
        assert!(examples.iter().all(|c| c["call"].as_str().unwrap().contains("app.main(")));
    }

    /// A review of a signature change: the entry and the signature are listed,
    /// an unfinished test shows, and a check whose expected value moved carries
    /// `was`.
    #[test]
    fn review_graphs_show_signatures_unfinished_and_changed_checks() {
        let before = graph_index(
            &test_index(&[
                ("src", "app", "pub fn main(n: Int) -> Int {\n  n\n}"),
                ("test", "app_test", "import app\n\npub fn main_test() {\n  assert app.main(1) == 1\n}"),
            ]),
            &[],
            &HashSet::new(),
        );
        let after = graph_index(
            &test_index(&[
                ("src", "app", "pub fn main(n: Int, k: Int) -> Int {\n  n\n}"),
                ("test", "app_test", "import app\n\npub fn main_test() {\n  assert app.main(1) == 2\n}"),
            ]),
            &[],
            &HashSet::new(),
        );
        let task = json!({"id":"t","changes":[{"entity":"symbol:fn:app.main"}],
            "review_tests": {"base":"x","state":"done","results":[{"id":"fn:app_test.main_test","name":"main_test","result":"unfinished","checks":[{"call":"app.main(1)","expected":"2"}]}]}});
        let r = review_graphs(&before, &after, &task, &json!({"tests":["fn:app_test.main_test"],"untested":[],"suites":[],"leaves":[]}), &"".parse().unwrap(), Path::new("/tmp"));
        assert_eq!(
            r["signatures"],
            json!([{"id":"fn:app.main","before":"pub fn main(n: Int) -> Int","after":"pub fn main(n: Int, k: Int) -> Int"}])
        );
        assert_eq!(r["tests"].as_array().unwrap().len(), 1, "{:?}", r["tests"]);
        assert_eq!(r["tests"][0]["result"], json!("unfinished"));
        assert_eq!(r["tests"][0]["checks"][0]["was"], json!("1"), "the check's old expected value");
        assert!(r["summary"].as_str().unwrap().contains("1 signature changed, 1 test unfinished, 0 untested uses"), "{}", r["summary"]);
        assert!(!r["summary"].as_str().unwrap().contains("not run yet"), "{}", r["summary"]);
    }

    #[test]
    fn test_failure_blocks() {
        let f =
            failures("todo test/app_test.gleam:3\n test: app_test.todo_test\nassert test/app_test.gleam:6\n test: app_test.wrong_test\n");
        assert_eq!(f["app_test.todo_test"].0, "unfinished");
        assert_eq!(f["app_test.wrong_test"].0, "wrong");
        assert!(!f.contains_key("app_test.ok_test"));
        assert_eq!(f["app_test.wrong_test"].1, "assert test/app_test.gleam:6\ntest: app_test.wrong_test");
    }
}
