//! Builds each frontend project's release output, `priv/app.js`, which `src/board.rs`
//! embeds: the board's bundle, and the frontend shell's bundle served at `/app.js`.
//! Each project owns the recipe: its `solace.toml` declares the `build-release`
//! command, and this runs it in that project.

use std::path::Path;
use std::process::Command;

fn build_release(project: &Path, watched: &[&str]) {
    for dir in watched {
        println!("cargo:rerun-if-changed={}", project.join(dir).display());
    }
    let settings = project.join("solace.toml");
    let text = std::fs::read_to_string(&settings).unwrap_or_else(|e| panic!("reading {}: {e}", settings.display()));
    let value: toml::Value = text.parse().unwrap_or_else(|e| panic!("parsing {}: {e}", settings.display()));
    let run: Vec<&str> = value
        .get("commands")
        .and_then(|c| c.get("build-release"))
        .and_then(|c| c.get("run"))
        .and_then(|r| r.as_array())
        .map(|r| r.iter().filter_map(|a| a.as_str()).collect())
        .unwrap_or_default();
    let Some((program, args)) = run.split_first() else {
        panic!("{} declares no [commands] build-release run", settings.display());
    };
    let status = Command::new(program)
        .args(args)
        .current_dir(project)
        .status()
        .unwrap_or_else(|e| panic!("running {}'s build-release ({program}): {e}", project.display()));
    if !status.success() {
        panic!("{}'s build-release failed ({status}); its priv/app.js was not rebuilt", project.display());
    }
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let own = ["src", "solace.toml", "gleam.toml", "package.json"];
    // The frontend bundles the board, the contract view and migration, so build the
    // board first, and watch the frontend's path dependencies.
    build_release(&root.join("board"), &own);
    build_release(&root.join("frontend"), &own);
    for dep in ["board", "contracts", "migration"] {
        for watched in ["src", "gleam.toml"] {
            println!("cargo:rerun-if-changed={}", root.join(dep).join(watched).display());
        }
    }
}