use crate::{ Index};
use crate::failure::Failure;



pub(crate) struct ImplTarget {
    pub(crate)  name: String,
    pub(crate)  existing: Option<usize>,
    pub(crate)  text: String,
    pub(crate)  tail: String,
}

pub(crate) struct Placement {
    pub(crate)  roots: Vec<usize>,
    pub(crate)  target: ImplTarget,
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

pub(crate) fn children(ix: &Index) -> std::collections::HashMap<&str, Vec<usize>> {
    let mut children: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
    for (i, sym) in ix.syms.iter().enumerate() {
        if let Some(parent) = sym.parent.as_deref() {
            children.entry(parent).or_default().push(i);
        }
    }
    children
}
