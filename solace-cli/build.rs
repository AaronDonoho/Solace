//! Embeds the frontend shell's bundle, `frontend/priv/app.js`, which `src/board.rs` serves at
//! `/app.js`. The bundle is built beforehand by the frontend's `[build]` step `release`
//! (`build plan` lists it before this crate and marks it when it is out of date), so a Rust check
//! never runs bun or gleam. The build stops when the bundle is missing and names the step to run.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// What each project contributes to the bundle: its sources, and the manifests whose contents
/// change what is bundled (npm dependencies reach the bundle through `bun build`).
const PROJECT_PARTS: &[&str] = &["src", "gleam.toml", "package.json", "bun.lock"];

/// A dependency path from `frontend/gleam.toml`, read from the frontend it is written in:
/// `../board` is `solace-cli/board`, and `..` goes up one folder, as a file manager does.
fn resolve(frontend: &Path, path: &str) -> PathBuf {
    let mut resolved = frontend.to_path_buf();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                resolved.pop();
            }
            part => resolved.push(part),
        }
    }
    resolved
}

/// The projects the bundle is bundled from: the frontend, and each path dependency of
/// `frontend/gleam.toml` — read there, so a dependency added there is watched without a change
/// here (D-159).
fn bundle_projects(frontend: &Path) -> Vec<PathBuf> {
    let manifest = frontend.join("gleam.toml");
    let text = fs::read_to_string(&manifest).unwrap_or_else(|e| panic!("reading {}: {e}", manifest.display()));
    let mut projects = vec![frontend.to_path_buf()];
    let mut in_dependencies = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_dependencies = line == "[dependencies]";
        } else if in_dependencies && line.contains("path = \"") {
            let path = line.split("path = \"").nth(1).unwrap_or_default();
            projects.push(resolve(frontend, path.split('"').next().unwrap_or_default()));
        }
    }
    projects
}

fn main() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let frontend = crate_dir.join("frontend");
    let bundle = frontend.join("priv/app.js");
    let parts: Vec<PathBuf> = bundle_projects(&frontend)
        .iter()
        .flat_map(|project| PROJECT_PARTS.iter().map(|part| project.join(part)))
        .collect();
    // A part that isn't there yet, a project's `package.json` for example, is watched through the
    // files beside it: naming a path that doesn't exist makes cargo rerun this script on every
    // build, and naming the folder would rerun it whenever the project's `build/` changes.
    let watched: BTreeSet<PathBuf> = parts
        .iter()
        .flat_map(|part| {
            if part.exists() {
                vec![part.clone()]
            } else {
                let beside = fs::read_dir(part.parent().unwrap()).into_iter().flatten().flatten();
                beside.map(|e| e.path()).filter(|p| p.is_file()).collect()
            }
        })
        .chain([bundle.clone()])
        .collect();
    for path in &watched {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    if !bundle.is_file() {
        panic!("solace-cli/frontend/priv/app.js is missing: run the [build] step `release` in solace-cli/frontend (`build plan` lists it)");
    }
}
