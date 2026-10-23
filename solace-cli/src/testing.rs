//! Fixtures for every module's tests.
use super::*;

/// A folder for one test, deleted when the test ends, passed or failed.
pub(crate) struct Scratch(pub PathBuf);
impl Scratch {
    pub(crate) fn new() -> Scratch {
        let dir = env::temp_dir().join(format!("solace-test-{}", unique_id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
    /// Writes `files` (path, text) into the folder.
    pub(crate) fn with(self, files: &[(&str, &str)]) -> Scratch {
        for (path, text) in files {
            let p = self.0.join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        self
    }
    /// Gleam's packages and compiled dependencies, so `gleam test` runs
    /// without downloading anything: the bench pilot's.
    pub(crate) fn with_packages(self) -> Scratch {
        let pilot = Path::new(env!("CARGO_MANIFEST_DIR")).join("../bench/pilot-large");
        for f in ["manifest.toml", "test/pilot_test.gleam"] {
            fs::create_dir_all(self.0.join(f).parent().unwrap()).unwrap();
            fs::copy(pilot.join(f), self.0.join(f)).unwrap();
        }
        let name = fs::read_to_string(self.0.join("gleam.toml")).map(|_| workspace::name(&self.0)).unwrap_or_else(|_| "app".into());
        fs::write(self.0.join("gleam.toml"), fs::read_to_string(pilot.join("gleam.toml")).unwrap().replace("pilot_large", &name)).unwrap();
        migrate::copy_tree(&pilot.join("build"), &self.0.join("build"), &|_| false).unwrap();
        self
    }
    /// The folder's Gleam project, read into a new store.
    pub(crate) fn import(self) -> Project {
        if !self.0.join("gleam.toml").exists() {
            fs::write(self.0.join("gleam.toml"), "name = \"app\"\nversion = \"1.0.0\"\n").unwrap();
        }
        import(&self.0).unwrap();
        Project { store: Store::at(&self.0).unwrap(), _dir: self }
    }
}
impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A store and its folder, deleted together.
pub(crate) struct Project {
    pub(crate) store: Store,
    _dir: Scratch,
}
impl std::ops::Deref for Project {
    type Target = Store;
    fn deref(&self) -> &Store {
        &self.store
    }
}

/// A Gleam project of `files` (path, text), named `app`, in a store.
pub(crate) fn project(files: &[(&str, &str)]) -> Project {
    Scratch::new().with(files).import()
}

/// A Cargo package's manifest, for a test project.
pub(crate) const CARGO: &str = "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";

/// A Rust project whose `src/lib.rs` is `lib`, read into a new store.
pub(crate) fn rust_project(lib: &str) -> (Scratch, Store) {
    let root = Scratch::new().with(&[("Cargo.toml", CARGO), ("src/lib.rs", lib)]);
    import(&root).unwrap();
    let store = Store::at(&root).unwrap();
    (root, store)
}

/// Leaves message `m` in the root's task inbox under `id`, as a sender would.
pub(crate) fn inbox_file(root: &Path, id: &str, mut m: Value) {
    let dir = board::store_dir(root).join("inbox");
    fs::create_dir_all(&dir).unwrap();
    m["id"] = json!(id);
    // written aside and renamed, as senders do, so a running server never reads half of it
    let part = dir.join(format!("{id}.json.tmp"));
    fs::write(&part, m.to_string()).unwrap();
    fs::rename(&part, dir.join(format!("{id}.json"))).unwrap();
}

/// The repair copy a reply says it kept, as a path under the project root.
pub(crate) fn kept_copy(root: &std::path::Path, reply: &str) -> std::path::PathBuf {
    let kept = reply
        .split("kept at ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no kept copy in reply: {reply}"));
    root.join(kept.trim_end_matches(|c: char| c == '.' || c == ','))
}
