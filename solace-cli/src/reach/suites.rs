//! Suite selection and report formatting: which declared suites see a change, and how a report ranks its uses.

use super::*;
use crate::Index;
use crate::count;
use crate::reach::test_symbols;
use crate::reach::covered;
use crate::reach::flow::Use;
use crate::reach::flow::Impact;
#[cfg(test)]
#[path = "suites_tests.rs"]
mod tests;

/// The suites whose reach holds one of `syms`, as `e2e (`run e2e`)`.
pub fn reached(suites: &[Suite], syms: &[usize]) -> Vec<String> {
    suites
        .iter()
        .filter(|(_, r)| !syms.is_empty() && r.as_ref().is_none_or(|r| syms.iter().any(|j| r.contains(j))))
        .map(|(n, _)| format!("{n} (`run {n}`)"))
        .collect()
}

/// Each source symbol's tests, by D-36.3 coverage.
pub(crate) fn own_tests(ix: &Index) -> HashMap<usize, Vec<usize>> {
    let mut out: HashMap<usize, Vec<usize>> = HashMap::new();
    for t in test_symbols(ix) {
        for u in covered(ix, t) {
            out.entry(u).or_default().push(t);
        }
    }
    out
}

/// "N possible uses: readers of fields another type also declares (…)", or nothing.
pub(crate) fn possible_text(uses: &[(&Use, Vec<usize>)]) -> Option<String> {
    let mut fields: Vec<&str> = vec![];
    for (u, _) in uses.iter().filter(|(u, _)| !u.certain) {
        let f = u.via.strip_prefix("reads ").unwrap_or(&u.via);
        if !fields.contains(&f) {
            fields.push(f);
        }
    }
    let n = uses.iter().filter(|(u, _)| !u.certain).count();
    let more = if fields.len() > 4 { format!(" and {} more", fields.len() - 4) } else { String::new() };
    (n > 0).then(|| {
        format!(
            "{}: readers of a field another type also declares ({}{more})",
            count(n, "possible use"),
            fields[..fields.len().min(4)].join(", ")
        )
    })
}

pub(crate) fn fields_text(ix: &Index, impact: &Impact) -> String {
    let (mut out, mut last) = (vec![], usize::MAX);
    for (ty, f) in impact.fields.iter().take(8) {
        out.push(if *ty == last { format!(".{f}") } else { format!("{}.{f}", ix.syms[*ty].name) });
        last = *ty;
    }
    if impact.fields.len() > 8 {
        out.push(format!("and {} more", impact.fields.len() - 8));
    }
    out.join(", ")
}

/// `pricing.quote` for `pilot/pricing.quote`.
pub fn short(ix: &Index, i: usize) -> String {
    let s = &ix.syms[i];
    format!("{}.{}", s.module.rsplit('/').next().unwrap_or(&s.module), s.name)
}

pub fn suites(s: &Store, ix: &Index) -> Result<Vec<Suite>, Failure> {
    let settings = lockdown::settings(&s.root)?;
    let mut out = vec![];
    for (name, c) in settings.get("commands").and_then(|c| c.as_table()).into_iter().flatten() {
        let Some(checks) = c.get("checks").and_then(|x| x.as_array()) else { continue };
        let mut entries: Vec<String> = checks.iter().filter_map(|x| x.as_str().map(String::from)).collect();
        let run: Vec<&str> = c["run"].as_array().into_iter().flatten().filter_map(|x| x.as_str()).collect();
        if let ["gleam", "run", "-m", m, ..] = run.as_slice() {
            entries.push(format!("{m}.main"));
        }
        if entries.iter().any(|e| e == "*") {
            out.push((name.clone(), None));
            continue;
        }
        // entry points that don't resolve are skipped silently.
        let reach = entries.iter().filter_map(|e| s.resolve(ix, e).ok()).flat_map(|i| closure(ix, i)).collect();
        out.push((name.clone(), Some(reach)));
    }
    Ok(out)
}

/// Every entry point the declared suites name, resolved or not, as (suite,
/// entry): `extract` lists those that stop resolving (`suites` skips them).
pub fn entries(s: &Store) -> Result<Vec<(String, String)>, Failure> {
    let settings = lockdown::settings(&s.root)?;
    let mut out = vec![];
    for (name, c) in settings.get("commands").and_then(|c| c.as_table()).into_iter().flatten() {
        let Some(checks) = c.get("checks").and_then(|x| x.as_array()) else { continue };
        let run: Vec<&str> = c["run"].as_array().into_iter().flatten().filter_map(|x| x.as_str()).collect();
        let mut named: Vec<String> = checks.iter().filter_map(|x| x.as_str().map(String::from)).filter(|e| e != "*").collect();
        if let ["gleam", "run", "-m", m, ..] = run.as_slice() {
            named.push(format!("{m}.main"));
        }
        out.extend(named.into_iter().map(|e| (name.clone(), e)));
    }
    Ok(out)
}

/// The uses (not the changed symbols): certain ones nearest first, those
/// without a test of their own first within a step, then the possible ones
/// (run 20: these crowded out the callers whose tests reach the change):
/// (use, its tests).
pub fn ranked<'a>(ix: &Index, impact: &'a Impact) -> Vec<(&'a Use, Vec<usize>)> {
    let own = own_tests(ix);
    let mut out: Vec<(&Use, Vec<usize>)> =
        impact.uses.iter().filter(|u| u.steps > 0).map(|u| (u, own.get(&u.sym).cloned().unwrap_or_default())).collect();
    out.sort_by_key(|(u, t)| (!u.certain, u.steps, !t.is_empty()));
    out
}

/// The submit reply's note: at most three lines (D-44).
pub fn note(ix: &Index, impact: &Impact, suites: &[String]) -> Vec<String> {
    let uses = ranked(ix, impact);
    if uses.is_empty() {
        return vec![];
    }
    let origin: Vec<String> = impact.uses.iter().filter(|u| u.steps == 0).map(|u| short(ix, u.sym)).collect();
    let callers = uses.iter().any(|(u, _)| u.via.starts_with("calls"));
    let kinds = match (callers, impact.fields.is_empty()) {
        (true, true) => "callers".to_string(),
        (true, false) => format!("callers, and readers of {}", fields_text(ix, impact)),
        (false, _) => format!("readers of {}", fields_text(ix, impact)),
    };
    let sure = uses.iter().filter(|(u, _)| u.certain).count();
    let maybe = if sure < uses.len() { format!(" (and {} possible)", uses.len() - sure) } else { String::new() };
    let mut out = vec![format!("Reaches {}{maybe}: {kinds}.", count(sure, "use"))];
    let untested: Vec<String> = uses
        .iter()
        .filter(|(u, t)| u.certain && t.is_empty())
        .map(|(u, _)| {
            let via = u.via.strip_prefix("reads ").map_or(String::new(), |f| format!(", reads {}", f.split_once('.').map_or(f, |p| p.1)));
            format!("{} ({}{via})", short(ix, u.sym), count(u.steps, "step"))
        })
        .collect();
    let all = format!("`impact {}` lists all.", origin.join(" "));
    if untested.is_empty() {
        out[0] += &format!(" Each has a test of its own; {all}");
    } else {
        let more = if untested.len() > 3 { format!(" and {} more", untested.len() - 3) } else { String::new() };
        out.push(format!("No test of their own: {}{more} — {all}", untested[..untested.len().min(3)].join(", ")));
    }
    let mut last = vec![];
    if !impact.leaves.is_empty() {
        let leaves: Vec<String> = impact.leaves.iter().map(|(i, e)| format!("{} ({e})", short(ix, *i))).collect();
        last.push(format!("Leaves the program: {}.", leaves.join(", ")));
    }
    if !suites.is_empty() {
        last.push(format!("Also reached: {}.", suites.join(", ")));
    }
    if !last.is_empty() {
        out.push(last.join(" "));
    }
    out
}

/// `impact`: every use, nearest first, within `budget`.
pub fn report(ix: &Index, impact: &Impact, suites: &[String], budget: usize) -> String {
    let origin: Vec<String> = impact.uses.iter().filter(|u| u.steps == 0).map(|u| short(ix, u.sym)).collect();
    let uses = ranked(ix, impact);
    let sure = uses.iter().filter(|(u, _)| u.certain).count();
    let mut out =
        format!("{} reach{} {}, nearest first:\n", origin.join(", "), if origin.len() == 1 { "es" } else { "" }, count(sure, "use"));
    let mut tail = possible_text(&uses).map_or(String::new(), |p| format!("And {p}; `refs <Type>.<field>` lists them\n"));
    if !impact.fields.is_empty() {
        let fields: Vec<String> = impact.fields.iter().map(|(t, f)| format!("{}.{f}", ix.syms[*t].name)).collect();
        tail += &format!("Fields: {}\n", fields.join(", "));
    }
    if !impact.leaves.is_empty() {
        let leaves: Vec<String> = impact.leaves.iter().map(|(i, e)| format!("{} ({e})", short(ix, *i))).collect();
        tail += &format!("Leaves the program: {}\n", leaves.join(", "));
    }
    if !suites.is_empty() {
        tail += &format!("Also reached: {}\n", suites.join(", "));
    }
    for (k, (u, t)) in uses.iter().take(sure).enumerate() {
        let tests = match t.as_slice() {
            [] => "no test of its own".to_string(),
            [one] => format!("tested by {}", short(ix, *one)),
            [one, rest @ ..] => format!("tested by {} and {} more", short(ix, *one), rest.len()),
        };
        let line = format!("  {} — {}, {}; {tests}\n", short(ix, u.sym), count(u.steps, "step"), u.via);
        if out.len() + line.len() + tail.len() + 60 > budget {
            out += &format!("  … {} more not shown, to fit the size limit\n", sure - k);
            break;
        }
        out += &line;
    }
    out + &tail
}
