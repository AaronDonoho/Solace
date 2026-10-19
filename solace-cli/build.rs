//! Embeds the frontend shell's bundle, `frontend/priv/app.js`, which `src/board.rs` serves at
//! `/app.js`. The bundle is built beforehand by the frontend's `[build]` step `release`
//! (`build plan` lists it before this crate and marks it when it is out of date), so a Rust check
//! never runs bun or gleam. The build stops when the bundle is missing and names the step to run.
//! Only the bundle is watched (D-166): its sources reach cargo through `build plan`, not here.

use std::path::Path;

fn main() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let bundle = crate_dir.join("frontend/priv/app.js");
    // Only the bundle is watched: the sources reach cargo through `build plan`, which records
    // what built the bundle (D-166), so a source added later can't go unwatched.
    println!("cargo:rerun-if-changed={}", bundle.display());
    if !bundle.is_file() {
        panic!("solace-cli/frontend/priv/app.js is missing: run the [build] step `release` in solace-cli/frontend (`build plan` lists it)");
    }
}
