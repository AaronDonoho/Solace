//! What the app keeps between runs: the size board windows open at, and a
//! copy of the pages' settings. A page's own storage belongs to its address,
//! port included, so each board window is given the copy before its page runs.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{json, Value};



/// The size a board window opens at when none was kept.
pub(crate) const DEFAULT_SIZE: (f64, f64) = (1280.0, 820.0);

#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Preferences {
    /// The logical size of the last board window closed.
    pub(crate) size: Option<(f64, f64)>,
    /// Each setting's value, "" once cleared, so a cleared setting stays
    /// cleared at every address.
    pub(crate) settings: BTreeMap<String, String>,
}

/// The preferences and the file that keeps them.
pub(crate) struct Store {
    path: PathBuf,
    kept: Mutex<Preferences>,
}

fn usable(length: f64) -> bool {
    length.is_finite() && length >= 1.0
}

fn board_size(preferences: &Preferences) -> (f64, f64) {
    match preferences.size {
        Some((width, height)) if usable(width) && usable(height) => (width, height),
        _ => DEFAULT_SIZE,
    }
}

fn read(path: &PathBuf) -> Preferences {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .map(|value| from_json(&value))
        .unwrap_or_default()
}

fn from_json(value: &Value) -> Preferences {
    let size = value["size"]
        .as_array()
        .and_then(|size| match size.as_slice() {
            [width, height] => Some((width.as_f64()?, height.as_f64()?)),
            _ => None,
        });
    let settings = value["settings"]
        .as_object()
        .map(|settings| {
            settings
                .iter()
                .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Preferences { size, settings }
}

fn to_json(preferences: &Preferences) -> Value {
    json!({
        "size": preferences.size.map(|(width, height)| [width, height]),
        "settings": preferences.settings,
    })
}

fn write(path: &PathBuf, preferences: &Preferences) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(path, to_json(preferences).to_string()).map_err(|e| e.to_string())
}

/// Puts `settings` in the page's storage and tells the page's subscribers of
/// each one that changed (the settings store's `solace-setting` event).
pub(crate) fn settings_script(settings: &BTreeMap<String, String>) -> String {
    let settings = json!(settings);
    format!(
        "(() => {{ try {{ for (const [key, value] of Object.entries({settings})) {{ \
         if ((localStorage.getItem(key) ?? '') === value) continue; \
         value ? localStorage.setItem(key, value) : localStorage.removeItem(key); \
         document.dispatchEvent(\
         new CustomEvent('solace-setting', {{ detail: {{ key, value }} }})); \
         }} }} catch {{}} }})();"
    )
}

/// Runs before each page in a board window: the window's first page at an
/// address is drawn with `settings`, and a reload keeps what the page stored
/// since.
pub(crate) fn seed_script(settings: &BTreeMap<String, String>) -> String {
    format!(
        "(() => {{ try {{ if (sessionStorage.getItem('solace-seeded')) return; \
         sessionStorage.setItem('solace-seeded', '1'); }} catch {{ return; }} {} }})();",
        settings_script(settings)
    )
}

#[cfg(test)]
fn test_path(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("solace-preferences-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir.join("preferences.json")
}

#[test]
fn a_closed_board_windows_size_opens_the_next_one() {
    let path = test_path("size");
    let store = Store::open(path.clone());
    assert_eq!(store.size(), DEFAULT_SIZE);
    store.set_size(1500.0, 900.0).unwrap();
    store.set_size(0.0, 0.0).unwrap();
    assert_eq!(Store::open(path.clone()).size(), (1500.0, 900.0));
    fs::write(&path, r#"{"size":[0,820]}"#).unwrap();
    assert_eq!(Store::open(path).size(), DEFAULT_SIZE);
}

#[test]
fn settings_survive_a_restart_and_a_cleared_one_stays_cleared() {
    let path = test_path("settings");
    let store = Store::open(path.clone());
    store.set_setting("solace-font-size", "20").unwrap();
    store.set_setting("solace-theme", "dark").unwrap();
    store.set_setting("solace-theme", "").unwrap();
    let settings = Store::open(path).settings();
    let expected: BTreeMap<String, String> = [
        ("solace-font-size".to_string(), "20".to_string()),
        ("solace-theme".to_string(), String::new()),
    ]
    .into();
    assert_eq!(settings, expected);
    let script = seed_script(&settings);
    assert!(script.contains(r#"{"solace-font-size":"20","solace-theme":""}"#));
    assert!(script.contains("sessionStorage.getItem('solace-seeded')"));
}

impl Store {
    /// The preferences kept at `path`, empty when there are none yet.
    pub(crate) fn open(path: PathBuf) -> Store {
        let kept = Mutex::new(read(&path));
        Store { path, kept }
    }

    /// The size the next board window opens at.
    pub(crate) fn size(&self) -> (f64, f64) {
        board_size(&self.kept.lock().unwrap())
    }

    pub(crate) fn settings(&self) -> BTreeMap<String, String> {
        self.kept.lock().unwrap().settings.clone()
    }

    /// Keeps a closed board window's size; a minimized window's empty size
    /// is not kept.
    pub(crate) fn set_size(&self, width: f64, height: f64) -> Result<(), String> {
        if !usable(width) || !usable(height) {
            return Ok(());
        }
        let mut kept = self.kept.lock().unwrap();
        kept.size = Some((width, height));
        write(&self.path, &kept)
    }

    /// Keeps a page's setting, "" when it was cleared.
    pub(crate) fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        let mut kept = self.kept.lock().unwrap();
        kept.settings.insert(key.to_string(), value.to_string());
        write(&self.path, &kept)
    }
}
