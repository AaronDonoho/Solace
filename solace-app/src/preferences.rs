//! What the app keeps between runs: the size board windows open at, and a
//! copy of the pages' settings. A page's own storage belongs to its address,
//! port included, so each board window is given the copy before its page runs.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{json, Value};
#[cfg(test)]
#[path = "preferences_tests.rs"]
mod tests;



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
    // A crash mid-write leaves the old file whole: only a finished file is renamed into place.
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    fs::write(&tmp, to_json(preferences).to_string()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
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
