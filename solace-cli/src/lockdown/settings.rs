use std::fs;
use std::path::Path;
use crate::failure::Failure;
use crate::lockdown::error;
use crate::lockdown::invalid_settings;
use crate::lockdown::paths::is_gitignore;
use crate::lockdown::commands::command;
use crate::lockdown::commands::passed;
use crate::recipe;
#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
use crate::drafts;
use crate::workspace;
pub fn settings(root: &Path) -> Result<toml::Value, Failure> {
    match fs::read_to_string(root.join("solace.toml")) {
        // A folder of projects' solace.toml only lists them (D-57); `workspace` checks it.
        Ok(s) if crate::language::detect(root).is_none() && !root.join(".solace/store.db").exists() => s.parse().map_err(invalid_settings),
        Ok(s) => {
            let v = parse_settings(&s)?;
            // A skills folder that is a nested project (D-57), or a folder
            // inside one, collides with its store's internal tier (D-27):
            // every folder from the root down to it is checked, where the
            // root is known.
            let dir = skills_dir(&v);
            let nested = root.join(&dir).strip_prefix(root).map(|rel| {
                let mut at = root.to_path_buf();
                rel.components().any(|c| {
                    at = at.join(c.as_os_str());
                    at.join(".solace/store.db").exists()
                })
            });
            if nested.unwrap_or(true) {
                return Err(invalid_settings("skills names a nested project; point it at a folder of house rules"));
            }
            Ok(v)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Value::Table(Default::default())),
        Err(e) => Err(e.into()),
    }
}

/// Checks settings text without reading or writing the file.
pub fn parse_settings(s: &str) -> Result<toml::Value, Failure> {
    let v: toml::Value = s.parse().map_err(invalid_settings)?;
    let invalid = invalid_settings;
    for (old, fix) in [
        ("resources", "`resources` was removed: every text file is watched now; delete it"),
        ("ignore", "`ignore` is now `unwatched`; rename it"),
        ("resource_limit", "`resource_limit` is now `watch_limit`; rename it"),
    ] {
        if v.get(old).is_some() {
            return Err(invalid(fix));
        }
    }
    // `projects`: the folders below this root project (D-57); `workspace` checks them.
    if v.get("projects").is_some_and(|p| p.as_array().is_none_or(|a| a.iter().any(|d| d.as_str().is_none()))) {
        return Err(invalid("projects"));
    }
    for key in ["unwatched"] {
        if let Some(a) = v.get(key) {
            for p in a.as_array().ok_or_else(|| invalid(key))? {
                let p = p.as_str().ok_or_else(|| invalid(key))?;
                globset::Glob::new(p).map_err(|_| invalid(p))?;
            }
        }
    }
    if v.get("watch_limit").is_some_and(|x| x.as_integer().is_none_or(|n| n < 0)) {
        return Err(invalid("watch_limit"));
    }
    // `skills`: the skills folder (D-27), a folder inside the project, and
    // none of its own code: `src` or `test` (or a folder inside them) would
    // make watched source protected, and `.solace` is Solace's own. A nested
    // project is refused by `settings`, which knows the root. `./x` names the
    // same folder as `x`, so the parts decide, not the raw string.
    if v.get("skills").is_some_and(|s| {
        s.as_str().is_none_or(|d| {
            let d = d.trim_end_matches('/');
            let parts: Vec<&str> = d.split('/').filter(|c| !c.is_empty() && *c != ".").collect();
            d.is_empty()
                || parts.is_empty()
                || Path::new(d).is_absolute()
                || parts.iter().any(|c| *c == "..")
                || matches!(parts.first().copied().unwrap_or(""), "src" | "test" | ".solace")
        })
    }) {
        return Err(invalid("skills"));
    }
    for key in ["commands", "validators"] {
        if let Some(t) = v.get(key) {
            for (name, c) in t.as_table().ok_or_else(|| invalid(key))? {
                let argv = if key == "commands" { c.get("run") } else { Some(c) };
                if argv.and_then(|a| a.as_array()).is_none_or(|a| a.is_empty() || a.iter().any(|x| x.as_str().is_none())) {
                    return Err(invalid(name));
                }
                if key == "validators" {
                    globset::Glob::new(name).map_err(|_| invalid(name))?;
                }
                // `checks`: the entry points a test suite drives (D-44), `"*"` for all
                if c.get("timeout").is_some_and(|x| x.as_integer().is_none_or(|n| n <= 0))
                    || c.get("args").is_some_and(|x| x.as_bool().is_none())
                    || c.get("allowed_in_reviews").is_some_and(|x| x.as_bool().is_none())
                    || c.get("test").is_some_and(|x| x.as_bool().is_none())
                    || c.get("checks").is_some_and(|x| x.as_array().is_none_or(|a| a.iter().any(|e| e.as_str().is_none())))
                {
                    return Err(invalid(name));
                }
            }
        }
    }
    // `[sandbox] read`: folders outside the project a sandboxed command may
    // read (D-158 point 4).
    if let Some(t) = v.get("sandbox") {
        let t = t.as_table().ok_or_else(|| invalid("sandbox"))?;
        for (key, value) in t {
            if key != "read" {
                return Err(invalid(key));
            }
            let paths = value.as_array().ok_or_else(|| invalid(key))?;
            if paths.iter().any(|p| p.as_str().is_none_or(|p| p.is_empty())) {
                return Err(invalid(key));
            }
        }
    }
    // `[build]`: the project's build recipe (D-131).
    recipe::check(&v)?;
    Ok(v)
}

pub(crate) fn limit(v: &toml::Value) -> usize {
    v.get("watch_limit").and_then(|x| x.as_integer()).unwrap_or(WATCH_LIMIT as i64) as usize
}

/// Dependency folders are never watched, whatever the `.gitignore` says or when it was written.
pub(crate) fn dependencies(name: &str) -> bool {
    name.split('/').any(|c| c == "node_modules")
}

/// Whether `name` is a Cargo or rustup setting the human approves (D-158
/// point 4, D-174): each one can run a program or pick a toolchain. Cargo and
/// rustup read both the older name and the newer one. A crate below the root
/// (a workspace member or a path dependency) has the same settings: cargo runs
/// its `build.rs` and builds with its `Cargo.toml` (D-175).
pub(crate) fn rust_setting(name: &str) -> bool {
    if matches!(name, "build.rs" | ".cargo/config.toml" | ".cargo/config" | "rust-toolchain.toml" | "rust-toolchain") {
        return true;
    }
    let unwatched = ["target/", ".git/", ".solace/"].iter().any(|p| name.starts_with(p));
    !unwatched && (name.ends_with("/build.rs") || name.ends_with("/Cargo.toml"))
}

/// Whether a run's `write` of `name` is a settings proposal (D-38, D-45,
/// D-158 point 4): the human-owned files, and a Rust project's Cargo and
/// rustup settings, its workspace crates' included (D-175).
pub fn settings_file(root: &Path, name: &str) -> bool {
    if crate::lockdown::paths::design_path(root, name) {
        return false;
    }
    name == "solace.toml" || name == "AGENTS.md" || is_gitignore(name) || (crate::language::is_rust(root) && rust_setting(name))
}

/// Why a Cargo.toml change that touches a protected key is refused (D-158 point 4).
pub(crate) fn cargo_setting_note() -> String {
    format!(
        "dependency, feature, workspace and build changes to Cargo.toml change only through \
         a human-approved proposal: `write Cargo.toml` alone, with the exact new text in `text` and \
         the why in `reason`. {}",
        drafts::WHOLE
    )
}

/// The dependency, feature and build keys of a Cargo.toml the human approves
/// (D-158 point 4, D-177): the dependency tables, `[features]`, each
/// `[target.*]` table's dependencies, `[patch]`, `[replace]`, `package.build`,
/// `package.links` and `lib.proc-macro`, and a workspace's `members`,
/// `default-members`, `exclude` and `dependencies` (D-175). Text that doesn't
/// parse counts as changed.
pub fn cargo_settings_changed(before: &[u8], after: &str) -> bool {
    let parse = |text: &str| text.parse::<toml::Value>().ok();
    let (Some(before), Some(after)) = (std::str::from_utf8(before).ok().and_then(parse), parse(after)) else { return true };
    cargo_settings(&before) != cargo_settings(&after)
}

/// The part of a parsed Cargo.toml that is a setting the human approves, so
/// the rest of the file compares equal (D-158 point 4).
fn cargo_settings(v: &toml::Value) -> toml::Value {
    let deps = ["dependencies", "dev-dependencies", "build-dependencies"];
    let mut out = toml::map::Map::new();
    for key in ["dependencies", "dev-dependencies", "build-dependencies", "features", "patch", "replace"] {
        if let Some(x) = v.get(key) {
            out.insert(key.to_string(), x.clone());
        }
    }
    if let Some(targets) = v.get("target").and_then(|t| t.as_table()) {
        let mut kept = toml::map::Map::new();
        for (name, table) in targets {
            let mut found = toml::map::Map::new();
            for key in deps {
                if let Some(x) = table.get(key) {
                    found.insert(key.to_string(), x.clone());
                }
            }
            if !found.is_empty() {
                kept.insert(name.clone(), toml::Value::Table(found));
            }
        }
        if !kept.is_empty() {
            out.insert("target".to_string(), toml::Value::Table(kept));
        }
    }
    let mut package = toml::map::Map::new();
    for key in ["build", "links"] {
        if let Some(x) = v.get("package").and_then(|p| p.get(key)) {
            package.insert(key.to_string(), x.clone());
        }
    }
    if !package.is_empty() {
        out.insert("package".to_string(), toml::Value::Table(package));
    }
    if let Some(x) = v.get("lib").and_then(|l| l.get("proc-macro")) {
        let mut lib = toml::map::Map::new();
        lib.insert("proc-macro".to_string(), x.clone());
        out.insert("lib".to_string(), toml::Value::Table(lib));
    }
    if let Some(workspace) = v.get("workspace").and_then(|w| w.as_table()) {
        let mut kept = toml::map::Map::new();
        for key in ["members", "default-members", "exclude", "dependencies"] {
            if let Some(x) = workspace.get(key) {
                kept.insert(key.to_string(), x.clone());
            }
        }
        if !kept.is_empty() {
            out.insert("workspace".to_string(), toml::Value::Table(kept));
        }
    }
    toml::Value::Table(out)
}

/// The project's skills folder (D-27): the `skills` key of solace.toml, and
/// `skills` when the key is missing. Humans own it: agents read its skills and
/// change none of them.
pub const DEFAULT_SKILLS: &str = "skills";

pub fn skills_dir(v: &toml::Value) -> String {
    v.get("skills").and_then(|s| s.as_str()).unwrap_or(DEFAULT_SKILLS).trim_end_matches('/').to_string()
}

/// Whether `name` is the skills folder or a file in it (D-27).
pub fn in_skills(v: &toml::Value, name: &str) -> bool {
    crate::lockdown::paths::matches(&format!("{}/", skills_dir(v)), name)
}

/// Checks a file written at `p` with `text`: a manifest needs a name and a
/// version, and each validator whose pattern matches must pass (E6003).
pub(crate) fn validate(root: &Path, v: &toml::Value, name: &str, p: &Path, text: &str) -> Result<(), Failure> {
    if name == "gleam.toml" {
        let manifest = text.parse::<toml::Value>().map_err(|e| Failure(format!("E6003 invalid manifest\n  {e}"), 1))?;
        if manifest.get("name").and_then(|x| x.as_str()).is_none_or(|s| !workspace::package_name(s))
            || manifest.get("version").and_then(|x| x.as_str()).is_none_or(|v| {
                let core = v.split(['-', '+']).next().unwrap_or("");
                let parts: Vec<_> = core.split('.').collect();
                parts.len() != 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
            })
        {
            return error("E6003", name, "manifest requires name and version");
        }
    }
    if let Some(validators) = v.get("validators").and_then(|x| x.as_table()) {
        for (pattern, argv) in validators {
            if crate::lockdown::paths::matches(pattern, name) {
                let argv: Vec<String> =
                    argv.as_array().unwrap().iter().map(|x| x.as_str().unwrap().replace("{file}", &p.to_string_lossy())).collect();
                let out = command(root, &argv, 120, crate::cli::BUDGET, false, false)
                    .map_err(|e| Failure(format!("E6003 validator failed\n  {}", e.0), 1))?;
                if !passed(&out) {
                    return error("E6003", name, out);
                }
            }
        }
    }
    Ok(())
}

/// The default `watch_limit`: text files up to this many bytes are watched.
pub const WATCH_LIMIT: usize = 262_144;
