//! HTTP routing for the code graph: the read-only node and file views, review summaries and test runs.

use super::*;
#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;



/// `%xx` escapes in a URL path decoded; invalid UTF-8 replaced.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (vec![], 0);
    while i < b.len() {
        match (b[i], b.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) fn route(s: &Store, method: &str, parts: &[&str], query: &HashMap<String, String>, body: &Value) -> Result<Value, Failure> {
    let reply = route_request(s, method, parts, query, body);
    crate::language::rust::clear_spans();
    reply
}

fn route_request(s: &Store, method: &str, parts: &[&str], query: &HashMap<String, String>, body: &Value) -> Result<Value, Failure> {
    match (method, parts) {
        ("GET", ["graph", "review"]) => {
            let tid = query.get("task").ok_or_else(|| Failure("E7001 task required".into(), 1))?;
            let (_, t) = task_here(s, tid)?;
            Ok(json!({"summary": review(s, &t)?["summary"].clone()}))
        }
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
                Err(e) if cut.len() < bytes.len() && cut.len() - e.valid_up_to() < 4 => std::str::from_utf8(&cut[..e.valid_up_to()]).ok(),
                _ => None,
            };
            Ok(json!({
                "path": name,
                "category": category,
                "text": text,
                "size": bytes.len(),
                "truncated": text.is_some() && cut.len() < bytes.len(),
            }))
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
