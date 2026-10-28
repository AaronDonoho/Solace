use std::collections::BTreeSet;

use super::{full_path, impl_owner_symbol, impl_self_range, leaf, mark, module_named_like};
use crate::{ Index};
use crate::failure::fail;
use crate::failure::Failure;



pub(super) fn check_direct(
    ix: &Index,
    ids: &std::collections::HashMap<&str, usize>,
    roots: &[usize],
    moved: &mut [bool],
) -> Result<(), Failure> {
    let children = children(ix);
    let mut chosen: BTreeSet<usize> = BTreeSet::new();
    for &root in roots {
        let sym = &ix.syms[root];
        if sym.kind_text == "impl" {
            impl_owner_symbol(ix, root)?;
            if sym.name.starts_with('<') && crate::language::rust_refs::impl_trait(&ix.syms, &ix.modules, root).is_none() {
                return fail(format!(
                    "E6013 {} is a trait impl whose trait path does not resolve exactly; make the path name one trait in this crate",
                    sym.qname()
                ));
            }
            chosen.insert(root);
            continue;
        }
        let Some(parent) = sym.parent.as_deref() else { continue };
        let Some(&at) = ids.get(parent) else { continue };
        if ix.syms[at].kind_text != "impl" {
            continue;
        }
        impl_owner_symbol(ix, at)?;
        if ix.syms[at].name.starts_with('<') {
            if !moved[at] {
                if crate::language::rust_refs::impl_trait(&ix.syms, &ix.modules, at).is_none() {
                    return fail(format!(
                        concat!(
                            "E6013 {} is a method of the trait impl {}, whose trait path does not resolve exactly; ",
                            "make the path name one trait in this crate"
                        ),
                        sym.qname(),
                        ix.syms[at].qname()
                    ));
                }
                let siblings = children.get(parent).map(Vec::as_slice).unwrap_or_default();
                if siblings.iter().any(|&j| ix.syms[j].kind_text != "use" && !moved[j]) {
                    return fail(format!(
                        "E6013 {} is a method of the trait impl {}; a trait impl can move only whole: select {} or every method in it",
                        sym.qname(),
                        ix.syms[at].qname(),
                        ix.syms[at].qname()
                    ));
                }
                mark(ix, moved, at);
            }
            chosen.insert(at);
        } else if !moved[at] {
            chosen.insert(at);
        }
    }
    if chosen.len() > 1 {
        let list: Vec<String> = chosen.iter().map(|&i| ix.syms[i].qname()).collect();
        return fail(format!("E6013 methods from more than one impl move at once: {}; select methods from one impl", list.join(", ")));
    }
    Ok(())
}

pub(super) fn named_impl(ix: &Index, input: &str) -> Result<Option<usize>, Failure> {
    let Some(open) = input.find('<') else { return Ok(None) };
    let Some(inner) = input[open + 1..].strip_suffix('>') else { return Ok(None) };
    if inner.is_empty() || inner.contains(" as ") {
        return Ok(None);
    }
    let prefix = input[..open].trim_end_matches(['.', ':']);
    let found: Vec<usize> = (0..ix.syms.len())
        .filter(|&i| {
            let sym = &ix.syms[i];
            sym.kind_text == "impl"
                && sym.parent.is_none()
                && (sym.name == inner || crate::store::without_generics(&sym.name) == inner)
                && (prefix.is_empty() || module_named_like(&sym.module, prefix))
        })
        .collect();
    match found.as_slice() {
        [] => Ok(None),
        [i] => Ok(Some(*i)),
        many => {
            let list: Vec<String> = many
                .iter()
                .map(|&i| {
                    let methods: Vec<&str> =
                        ix.syms.iter().filter(|c| c.parent.as_deref() == Some(ix.syms[i].id.as_str())).map(|c| c.name.as_str()).collect();
                    format!(
                        "  {} (impl; methods: {})",
                        ix.syms[i].qname(),
                        if methods.is_empty() { "none".to_string() } else { methods.join(", ") }
                    )
                })
                .collect();
            fail(format!("E1002 ambiguous name: {input} could be\n{}; select methods from the block you want", list.join("\n")))
        }
    }
}

pub(super) struct ImplTarget {
    pub(super) name: String,
    pub(super) existing: Option<usize>,
    pub(super) text: String,
    pub(super) tail: String,
}

pub(super) struct Placement {
    pub(super) roots: Vec<usize>,
    pub(super) target: ImplTarget,
}

pub(super) fn placements(
    ix: &Index,
    ids: &std::collections::HashMap<&str, usize>,
    target: &str,
    moved: &[bool],
    roots: &[usize],
) -> Result<Vec<Placement>, Failure> {
    let children = children(ix);
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
    for &root in roots {
        let sym = &ix.syms[root];
        if sym.kind_text == "impl" {
            continue;
        }
        let Some(parent) = sym.parent.as_deref() else { continue };
        let Some(&at) = ids.get(parent) else { continue };
        if ix.syms[at].kind_text != "impl" || moved[at] {
            continue;
        }
        groups.entry(at).or_default().push(root);
    }
    let mut out = vec![];
    for (source, members) in groups {
        let owner = impl_owner_symbol(ix, source)?;
        let existing = target_impl(ix, target, owner, moved)?;
        if let Some(at) = existing {
            for &root in &members {
                let name = leaf(&ix.syms[root].name);
                let duplicate = children
                    .get(ix.syms[at].id.as_str())
                    .and_then(|members| members.iter().copied().find(|&j| !moved[j] && leaf(&ix.syms[j].name) == name));
                if let Some(that) = duplicate {
                    return fail(format!("E1004 {} already exists; {} cannot move there", ix.syms[that].qname(), ix.syms[root].qname()));
                }
            }
        }
        let (text, tail) =
            if existing.is_some() { (String::new(), String::new()) } else { new_impl_parts(&ix.syms[source], &ix.syms[owner]) };
        out.push(Placement { roots: members, target: ImplTarget { name: ix.syms[source].name.clone(), existing, text, tail } });
    }
    Ok(out)
}

fn target_impl(ix: &Index, target: &str, owner: usize, moved: &[bool]) -> Result<Option<usize>, Failure> {
    let impls: Vec<usize> = (0..ix.syms.len())
        .filter(|&i| {
            let sym = &ix.syms[i];
            sym.kind_text == "impl" && sym.parent.is_none() && sym.module == target && !moved[i] && !sym.name.starts_with('<')
        })
        .collect();
    let owners = crate::language::rust_refs::impl_owner_batch(&ix.syms, &ix.modules, &impls);
    Ok(impls.into_iter().zip(owners).find_map(|(i, owner_here)| match owner_here {
        crate::language::rust_refs::ImplOwner::One(t) if t == owner => Some(i),
        _ => None,
    }))
}

fn new_impl_parts(source: &crate::Sym, owner: &crate::Sym) -> (String, String) {
    let Some(range) = impl_self_range(&source.text) else {
        return (format!("impl {} {{\n    ", full_path(&owner.module, &owner.name)), "\n}".to_string());
    };
    let current = &source.text[range.clone()];
    let generics = current.find('<').map_or(String::new(), |at| current[at..].to_string());
    let ty = format!("{}{generics}", full_path(&owner.module, &owner.name));
    let mut text = source.text.clone();
    text.replace_range(range, &ty);
    (text, "\n}".to_string())
}

pub(super) fn open_impl(text: &str) -> String {
    let trimmed = text.trim_end();
    let Some(open) = trimmed.strip_suffix('}') else { return trimmed.to_string() };
    format!("{}\n", open.trim_end())
}

pub(super) fn place_target_impls(
    s: &crate::Store,
    ix: &Index,
    target: &str,
    placements: &[Placement],
    ord: &mut i64,
) -> Result<std::collections::HashMap<usize, String>, Failure> {
    let mut created: Vec<Option<String>> = vec![None; placements.len()];
    let children = children(ix);
    for (k, p) in placements.iter().enumerate() {
        match p.target.existing {
            Some(i) => {
                let id = ix.syms[i].id.clone();
                if children.get(id.as_str()).map_or(true, |children| children.is_empty()) {
                    let text = open_impl(&ix.syms[i].text);
                    s.db.execute(
                        "UPDATE symbols SET text = ?2, tail = ?3, revision = ?4 WHERE id = ?1",
                        rusqlite::params![id, text, "\n}", crate::revision(&text)],
                    )?;
                    s.history(&id, "edit", &ix.syms[i].name)?;
                }
            }
            None => {
                *ord += 1;
                let id = crate::new_id(crate::language::rust::id_kind("impl"));
                let rev = crate::revision(&p.target.text);
                s.db.execute(
                    "INSERT INTO symbols(id, module, name, kind, text, ord, revision, parent, sep, tail) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9)",
                    rusqlite::params![id, target, p.target.name, "impl", p.target.text, *ord, rev, "\n", p.target.tail],
                )?;
                s.history(&id, "create", &p.target.name)?;
                created[k] = Some(id);
            }
        }
    }
    let mut parents: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
    for (k, p) in placements.iter().enumerate() {
        let id = p.target.existing.map_or_else(|| created[k].clone().unwrap_or_default(), |i| ix.syms[i].id.clone());
        for &root in &p.roots {
            parents.insert(root, id.clone());
        }
    }
    Ok(parents)
}

fn children(ix: &Index) -> std::collections::HashMap<&str, Vec<usize>> {
    let mut children: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
    for (i, sym) in ix.syms.iter().enumerate() {
        if let Some(parent) = sym.parent.as_deref() {
            children.entry(parent).or_default().push(i);
        }
    }
    children
}
