//! Builds the board's release output, `board/priv/app.js`, which `src/board.rs`
//! embeds. The board owns the recipe: its `solace.toml` declares the
//! `build-release` command, and this runs it in `board/`.

use std::path::Path;
use std::process::Command;

fn main() {
    let board = Path::new(env!("CARGO_MANIFEST_DIR")).join("board");
    for watched in ["src", "solace.toml", "gleam.toml", "package.json"] {
        println!("cargo:rerun-if-changed={}", board.join(watched).display());
    }
    let settings = board.join("solace.toml");
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
        .current_dir(&board)
        .status()
        .unwrap_or_else(|e| panic!("running the board's build-release ({program}): {e}"));
    if !status.success() {
        panic!("the board's build-release failed ({status}); board/priv/app.js was not rebuilt");
    }
}
