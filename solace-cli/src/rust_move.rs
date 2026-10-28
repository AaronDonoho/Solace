//! `move` of a Rust module's file (D-209): the module keeps its path and every
//! symbol ID, and its parent's `mod` line names the new file with `#[path]`.
use std::fs;
use std::path::Path;

use rusqlite::params;

use crate::rust_check::post_move_check;
use crate::lockdown::error;
use crate::rust_submit::{is_mod_line, mod_name, path_attr};
use crate::{ Index, ModuleRow,  Store, file_name, language, revision};
use crate::failure::Out;
use crate::failure::Failure;
#[cfg(test)]
#[path = "rust_move_tests.rs"]
mod tests;



/// Moves the file of the Rust module stored at `from` to `to`, or `None` when
/// no module's file is at `from` or it is a crate root's, which stays where
/// cargo finds it.
pub(crate) fn move_module_file(s: &Store, from: &str, to: &str) -> Option<Out> {
    let from = file_name(Path::new(from));
    let module: String = s.db.query_row("SELECT path FROM modules WHERE file = ?1", [&from], |r| r.get(0)).ok()?;
    let ix = match s.index() {
        Ok(ix) => ix,
        Err(e) => return Some(Err(e)),
    };
    let Some(parent) = parent_row(&ix, &module) else {
        return if module == "crate" {
            None
        } else {
            Some(error(
                "E6013",
                &module,
                format!("{module} has no stored parent module, so its file cannot move (integration test modules aren't supported)"),
            ))
        };
    };
    Some(move_module(s, &ix, &module, parent, &from, &file_name(Path::new(to))))
}

fn move_module(s: &Store, ix: &Index, module: &str, parent: &ModuleRow, from: &str, to: &str) -> Out {
    check_destination(s, ix, module, parent, to)?;
    let own = module.rsplit("::").next().unwrap_or(module);
    let declaration = declaration(ix, parent, own, path_value(parent, own, to).as_deref())?;
    let before = s.broken()?;
    s.transaction(&[s.root.join(to)], || {
        let dir = to.rsplit_once('/').map_or("", |(d, _)| d);
        s.db.execute(
            "UPDATE modules SET file = ?2, dir = ?3, owns_dir = ?4 WHERE path = ?1",
            params![module, to, dir, language::rust::owns_dir(module, to)],
        )?;
        match &declaration {
            Declaration::Header(header) => s.set_header(&parent.path, header)?,
            Declaration::Item(i, text) => {
                let x = &ix.syms[*i];
                s.db.execute("UPDATE symbols SET text = ?2, revision = ?3 WHERE id = ?1", params![x.id, text, revision(text)])?;
                s.history(&x.id, "edit", &x.name)?;
            }
        }
        post_move_check(s, &before)?;
        Ok(format!("Moved {from} to {to} (module {module}, its symbols kept)\n"))
    })
}

/// The module whose `mod` line declares `module`; a crate root has none.
fn parent_row<'a>(ix: &'a Index, module: &str) -> Option<&'a ModuleRow> {
    if module == "crate" {
        return None;
    }
    let parent = module.rsplit_once("::").map_or("crate", |(p, _)| p);
    ix.modules.iter().find(|m| m.path == parent)
}

/// The folder of `parent`'s file, where a `#[path]` it declares is read from.
fn parent_folder(parent: &ModuleRow) -> String {
    parent.file.as_deref().and_then(|f| f.rsplit_once('/')).map_or(String::new(), |(dir, _)| format!("{dir}/"))
}

fn check_destination(s: &Store, ix: &Index, module: &str, parent: &ModuleRow, to: &str) -> Result<(), Failure> {
    let folder = parent_folder(parent);
    if !to.ends_with(".rs") || !to.starts_with(&folder) || to.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return error("E6002", to, format!("{module}'s file goes in its parent's folder, {folder}, as a .rs file"));
    }
    if fs::symlink_metadata(s.root.join(to)).is_ok() || ix.modules.iter().any(|m| m.file.as_deref() == Some(to)) {
        return error("E6002", to, "something already exists there; a move never replaces a file, folder or module");
    }
    if ix.modules.iter().any(|m| m.path.starts_with(&format!("{module}::"))) {
        return error("E6013", module, format!("{module} declares modules of its own, whose files would have to move too"));
    }
    Ok(())
}

/// The `#[path]` the parent's `mod` line needs for `to`: none at the file Rust
/// finds by itself, else `to` from the parent's folder.
fn path_value(parent: &ModuleRow, own: &str, to: &str) -> Option<String> {
    let file = parent.file.as_deref()?;
    let owns = parent.owns_dir.unwrap_or_else(|| language::rust::owns_dir(&parent.path, file));
    let flat = language::rust::child_file(file, owns, own);
    if to == flat || to == format!("{}/mod.rs", flat.trim_end_matches(".rs")) {
        return None;
    }
    Some(to.strip_prefix(&parent_folder(parent)).unwrap_or(to).to_string())
}

/// Where the parent's rewritten `mod` line goes: its header, or a `mod` item.
enum Declaration {
    Header(String),
    Item(usize, String),
}

fn declaration(ix: &Index, parent: &ModuleRow, own: &str, path: Option<&str>) -> Result<Declaration, Failure> {
    if let Some(header) = with_path(&parent.header, own, path)? {
        return Ok(Declaration::Header(header));
    }
    let items = ix.syms.iter().enumerate().filter(|(_, x)| x.module == parent.path && x.parent.is_none() && x.kind_text == "mod");
    for (i, x) in items {
        if let Some(text) = with_path(&x.text, own, path)? {
            return Ok(Declaration::Item(i, text));
        }
    }
    error("E6013", &parent.path, format!("{} has no `mod {own};` line to name the new file", parent.path))
}

/// `text` with its `mod <own>;` line naming `path`: the line's `#[path]`
/// replaced, added, or dropped for `None`; `None` when `text` has no such line.
fn with_path(text: &str, own: &str, path: Option<&str>) -> Result<Option<String>, Failure> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let Some(at) = lines.iter().position(|l| is_mod_line(l) && mod_name(l) == Some(own)) else {
        return Ok(None);
    };
    let start = (0..at).rev().take_while(|&i| lines[i].trim_start().starts_with("#[")).last().unwrap_or(at);
    if lines[start..=at].iter().any(|l| path_attr(l).is_some() && l.contains("cfg_attr")) {
        return error("E6013", own, format!("`mod {own};` takes its path from a `cfg_attr`, which differs by platform"));
    }
    let indent = &lines[at][..lines[at].len() - lines[at].trim_start().len()];
    let mut out: String = lines[..start].concat();
    out.extend(lines[start..at].iter().filter(|l| path_attr(l).is_none()).copied());
    if let Some(p) = path {
        out += &format!("{indent}#[path = \"{p}\"]\n");
    }
    out += indent;
    out += without_path(lines[at].trim_start()).as_str();
    out += &lines[at + 1..].concat();
    Ok(Some(out))
}

/// A `mod` line with every `#[path = "..."]` attribute removed.
fn without_path(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(start) = rest.find("#[") {
        let Some(end) = attribute_end(rest, start) else { break };
        let attr = &rest[start..end];
        out.push_str(&rest[..start]);
        if path_attr(attr).is_none() {
            out.push_str(attr);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.trim_start().to_string()
}

/// The end of the attribute starting at `start`, matching its brackets outside
/// string literals.
fn attribute_end(text: &str, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = start;
    while i < text.len() {
        if let Some(end) = crate::rust_submit::string_end(text, i) {
            i = end;
            continue;
        }
        match text.as_bytes()[i] {
            b'[' => depth += 1,
            b']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}
