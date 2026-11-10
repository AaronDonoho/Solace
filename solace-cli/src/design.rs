//! Designs: the JSON format of a design, the rules that make one valid, and
//! reading, saving and versioning designs.
//!
//! A design is data, not code. It stays out of the symbol store and the
//! compiler's type checks.

use crate::failure::Failure;
use serde_json::{Map, Value};
use std::collections::HashSet;
#[cfg(test)]
#[path = "design_tests.rs"]
mod tests;
/// The only schema version this build reads.
pub(crate) const SCHEMA_VERSION: u64 = 1;

/// The tabs a design belongs to.
pub(crate) const TABS: &[&str] = &["architecture", "ui"];

/// The kinds of value a property can hold. Later tasks add more kinds here.
pub(crate) enum ValueKind {
    /// One of a fixed, closed list of text values.
    OneOf(&'static [&'static str]),
    /// A spacing token name, or a non-negative number.
    Spacing,
    /// `fill`, `hug`, or a fixed positive number.
    Size,
    /// A color token name, or a `#rgb` or `#rrggbb` hex color.
    Color,
    /// A text-size token name, or a positive number.
    TextSize,
    /// Any JSON text.
    Text,
    /// A heading level: the whole number 1, 2 or 3.
    Level,
    /// The id of a top-level screen in the same design.
    Screen,
}

/// An element type and the properties it accepts.
pub(crate) struct ElementType {
    /// Whether the type may hold children.
    pub(crate) children: bool,
    /// The properties it accepts, with each property's value kind.
    pub(crate) properties: &'static [(&'static str, ValueKind)],
    /// The properties every element of this type must state.
    pub(crate) required: &'static [&'static str],
}

/// Every element type a design may use. Later tasks extend this table.
pub(crate) const ELEMENT_TYPES: &[(&str, ElementType)] = &[
    ("screen", SCREEN),
    ("row", CONTAINER),
    ("column", CONTAINER),
    ("text", TEXT),
    ("heading", HEADING),
    ("button", BUTTON),
    ("text_input", TEXT_INPUT),
    ("image_placeholder", IMAGE_PLACEHOLDER),
];

const DESIGN_KEYS: &[&str] = &["schemaVersion", "id", "title", "tab", "elements"];

const ELEMENT_KEYS: &[&str] = &["id", "type", "properties", "style", "children"];

/// Checks that `design` is a valid design (D-230).
pub(crate) fn validate(design: &Value) -> Result<(), Failure> {
    let object = design.as_object().ok_or_else(|| invalid("a design must be a JSON object"))?;
    if object.get("schemaVersion").and_then(Value::as_u64) != Some(SCHEMA_VERSION) {
        return Err(invalid(format!("unknown schema version; expected {SCHEMA_VERSION}")));
    }
    unknown_keys(object, DESIGN_KEYS, "design")?;
    let id = text(object, "id", "design")?;
    if !valid_id(id) {
        return Err(invalid(format!("design id {id:?} is not 16 lowercase hex digits")));
    }
    text(object, "title", "design")?;
    let tab = text(object, "tab", "design")?;
    if !TABS.contains(&tab) {
        return Err(invalid(format!("unknown tab {tab:?}; expected one of {}", TABS.join(", "))));
    }
    let elements = object.get("elements").and_then(Value::as_array).ok_or_else(|| invalid("design has no list of elements"))?;
    if tab == "ui" && elements.is_empty() {
        return Err(invalid("a ui design needs at least one screen"));
    }
    let screens: HashSet<String> = elements
        .iter()
        .filter(|element| element.get("type").and_then(Value::as_str) == Some("screen"))
        .filter_map(|element| element.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect();
    let mut ids = HashSet::new();
    for element in elements {
        let (id, ty) = validate_element(element, &mut ids, &screens, true)?;
        if tab == "ui" && ty != "screen" {
            return Err(invalid(format!("element {id:?} at the top level of a ui design must be a screen")));
        }
    }
    Ok(())
}

fn validate_element<'a>(
    value: &'a Value,
    ids: &mut HashSet<String>,
    screens: &HashSet<String>,
    top_level: bool,
) -> Result<(&'a str, &'a str), Failure> {
    let object = value.as_object().ok_or_else(|| invalid("an element must be a JSON object"))?;
    let id = text(object, "id", "an element")?;
    let at = format!("element {id:?}");
    unknown_keys(object, ELEMENT_KEYS, &at)?;
    if !ids.insert(id.to_string()) {
        return Err(invalid(format!("{at} has a repeated id")));
    }
    let ty = text(object, "type", &at)?;
    let Some(kind) = ELEMENT_TYPES.iter().find_map(|(name, kind)| (*name == ty).then_some(kind)) else {
        return Err(invalid(format!("{at} has unknown type {ty:?}")));
    };
    if ty == "screen" && !top_level {
        return Err(invalid(format!("{at} is a screen below the top level")));
    }
    let properties =
        object.get("properties").and_then(Value::as_object).ok_or_else(|| invalid(format!("{at} properties must be an object")))?;
    for (name, value) in properties {
        let Some((_, values)) = kind.properties.iter().find(|(property, _)| *property == name) else {
            return Err(invalid(format!("{at} has unknown property {name:?}")));
        };
        values.check(value).map_err(|why| invalid(format!("{at} property {name:?} {why}")))?;
        if let ValueKind::Screen = values {
            let target = value.as_str().unwrap_or_default();
            if !screens.contains(target) {
                return Err(invalid(format!("{at} property {name:?} names no screen {target:?}")));
            }
        }
    }
    for name in kind.required {
        if !properties.contains_key(*name) {
            return Err(invalid(format!("{at} needs property {name:?}")));
        }
    }
    if let Some(style) = object.get("style") {
        let style = style.as_object().ok_or_else(|| invalid(format!("{at} style must be an object")))?;
        for (name, value) in style {
            let Some((_, kind)) = STYLE_PROPERTIES.iter().find(|(property, _)| *property == name) else {
                return Err(invalid(format!("{at} has unknown style {name:?}")));
            };
            kind.check(value).map_err(|why| invalid(format!("{at} style {name:?} {why}")))?;
        }
    }
    if let Some(children) = object.get("children") {
        if !kind.children {
            return Err(invalid(format!("{at} of type {ty:?} can't hold children")));
        }
        let children = children.as_array().ok_or_else(|| invalid(format!("{at} children must be a list")))?;
        for child in children {
            validate_element(child, ids, screens, false)?;
        }
    }
    Ok((id, ty))
}

impl ValueKind {
    fn check(&self, value: &Value) -> Result<(), String> {
        match self {
            ValueKind::OneOf(values) => match value.as_str() {
                Some(text) if values.contains(&text) => Ok(()),
                Some(text) => Err(format!("has value {text:?}; expected one of {}", values.join(", "))),
                None => Err(format!("must be one of {}", values.join(", "))),
            },
            ValueKind::Spacing => match value.as_str() {
                Some(text) if SPACING_TOKENS.contains(&text) => Ok(()),
                Some(text) => Err(format!("has unknown spacing token {text:?}")),
                None => match value.as_f64() {
                    Some(number) if number >= 0.0 => Ok(()),
                    Some(_) => Err("must not be negative".to_owned()),
                    None => Err(format!("must be a spacing token ({}) or a non-negative number", SPACING_TOKENS.join(", "))),
                },
            },
            ValueKind::Size => check_size(value),
            ValueKind::Color => check_color(value),
            ValueKind::TextSize => check_text_size(value),
            ValueKind::Text => match value.as_str() {
                Some(_) => Ok(()),
                None => Err("must be text".to_owned()),
            },
            ValueKind::Level => match value.as_u64() {
                Some(1..=3) => Ok(()),
                Some(_) => Err("must be 1, 2 or 3".to_owned()),
                None => Err("must be a whole number from 1 to 3".to_owned()),
            },
            ValueKind::Screen => match value.as_str() {
                Some(_) => Ok(()),
                None => Err("must name a screen".to_owned()),
            },
        }
    }
}

fn text<'a>(object: &'a Map<String, Value>, key: &str, at: &str) -> Result<&'a str, Failure> {
    object.get(key).and_then(Value::as_str).filter(|text| !text.is_empty()).ok_or_else(|| invalid(format!("{at} has no text {key:?}")))
}

fn unknown_keys(object: &Map<String, Value>, allowed: &[&str], at: &str) -> Result<(), Failure> {
    match object.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => Err(invalid(format!("{at} has unknown key {key:?}"))),
        None => Ok(()),
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == 16 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn invalid(why: impl std::fmt::Display) -> Failure {
    Failure(format!("E7033 invalid design: {why}"), 1)
}

pub(crate) const DESIGNS_DIR: &str = "designs";

/// The workspace's valid designs, sorted by id, and the files that could not be
/// read as one.
pub(crate) fn list(top: &std::path::Path) -> Result<Value, Failure> {
    let dir = top.join(DESIGNS_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => {
            return Ok(serde_json::json!({"designs": [], "invalid": []}));
        }
        Err(e) => return Err(e.into()),
    };
    let mut designs = Vec::new();
    let mut invalid = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let Some(name) = path.file_name().map(|name| name.to_string_lossy().into_owned()) else { continue };
        if !path.is_file() || path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        match read_file(&path, &name) {
            Ok(design) => {
                designs.push(serde_json::json!({"id": design["id"], "title": design["title"], "tab": design["tab"]}));
            }
            Err(e) => invalid.push(serde_json::json!({"file": name, "reason": e.0})),
        }
    }
    designs.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    invalid.sort_by(|a, b| a["file"].as_str().cmp(&b["file"].as_str()));
    Ok(serde_json::json!({"designs": designs, "invalid": invalid}))
}

/// One design, whole. A missing or malformed id is `E7023`; a file that is not
/// a valid design is `E7033`.
pub(crate) fn show(top: &std::path::Path, id: &str) -> Result<Value, Failure> {
    if !valid_id(id) {
        return Err(unknown(id));
    }
    open(top, id).map_err(|(_, failure)| failure)
}

fn read_file(path: &std::path::Path, name: &str) -> Result<Value, Failure> {
    let design = crate::jsonx::read(path).map_err(|e| invalid(format!("{name}: {}", e.0)))?;
    validate(&design)?;
    let stem = name.strip_suffix(".json").unwrap_or(name);
    let id = design["id"].as_str().unwrap_or("");
    if stem != id {
        return Err(invalid(format!("file name {stem:?} does not match design id {id:?}")));
    }
    Ok(design)
}

fn unknown(id: &str) -> Failure {
    Failure(format!("{UNKNOWN} {id}"), 1)
}

/// Where a design's versions live, beneath its own data.
const VERSIONS_DIR: &str = "versions";

/// Who saved a design version.
pub(crate) enum Source<'a> {
    /// A person, through the Design page.
    Human,
    /// A design run, named by its id.
    DesignRun(&'a str),
}

/// Saves `design` as its workspace's current design, appending a version when
/// its content differs from the saved one. A differing current design, and the
/// text of one that can't be parsed, are first kept as a `repo` version.
pub(crate) fn save(top: &std::path::Path, design: &Value, source: Source<'_>) -> Result<(), Failure> {
    validate(design)?;
    let id = design["id"].as_str().unwrap_or_default();
    let data = top.join(DATA_DIR);
    std::fs::create_dir_all(&data)?;
    let _lock = crate::fsx::lock_file(&data.join("lock"))?;
    let dir = top.join(DESIGNS_DIR);
    let current = read_current(&dir, id)?;
    if current.design.as_ref() == Some(design) {
        return Ok(());
    }
    let versions = versions_dir(top, id);
    let first = next_version(&versions)?;
    let latest = if first > 1 { crate::jsonx::read(&versions.join(format!("{}.json", first - 1))).ok() } else { None };
    let latest_design = latest.as_ref().and_then(|version| version.get("design"));
    let design_is_latest = latest_design == Some(design);
    let latest_raw = if current.raw.is_some() { most_recent_raw(&versions, first)? } else { None };
    let number = keep_current(&versions, first, &current, latest_design, latest_raw.as_deref())?;
    let source = match source {
        Source::Human => serde_json::json!({"kind": "human"}),
        Source::DesignRun(id) => serde_json::json!({"kind": "run", "id": id}),
    };
    if number != first || !design_is_latest {
        append_design(&versions, number, &source, design)?;
    }
    crate::jsonx::replace_pretty(&dir, &format!("{id}.json"), design)
}

fn next_version(dir: &std::path::Path) -> Result<u64, Failure> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(1),
        Err(e) => return Err(e.into()),
    };
    let mut highest = 0;
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "json")
            && let Some(number) = path.file_stem().and_then(|stem| stem.to_str()).and_then(|stem| stem.parse::<u64>().ok())
        {
            highest = highest.max(number);
        }
    }
    Ok(highest + 1)
}

const ALIGN: &[&str] = &["start", "center", "end", "stretch"];

const JUSTIFY: &[&str] = &["start", "center", "end", "between", "around"];

const TEXT_ALIGN: &[&str] = &["left", "center", "right"];

const CONTAINER: ElementType = ElementType {
    children: true,
    properties: &[
        ("width", ValueKind::Size),
        ("height", ValueKind::Size),
        ("gap", ValueKind::Spacing),
        ("padding", ValueKind::Spacing),
        ("align", ValueKind::OneOf(ALIGN)),
        ("justify", ValueKind::OneOf(JUSTIFY)),
        ("variant", ValueKind::OneOf(CONTAINER_VARIANTS)),
        ("action", ValueKind::Text),
        ("navigates_to", ValueKind::Screen),
    ],
    required: &["width", "height"],
};

const TEXT: ElementType = ElementType {
    children: false,
    properties: &[
        ("width", ValueKind::Size),
        ("height", ValueKind::Size),
        ("align", ValueKind::OneOf(TEXT_ALIGN)),
        ("text", ValueKind::Text),
        ("variant", ValueKind::OneOf(TEXT_VARIANTS)),
        ("action", ValueKind::Text),
        ("navigates_to", ValueKind::Screen),
    ],
    required: &["width", "height"],
};

fn check_size(value: &Value) -> Result<(), String> {
    match value.as_str() {
        Some("fill" | "hug") => Ok(()),
        Some(text) => Err(format!("has value {text:?}; expected fill, hug or a fixed size")),
        None => {
            let Some(object) = value.as_object() else {
                return Err("must be fill, hug or a fixed size".to_owned());
            };
            if object.len() != 1 || !object.contains_key("fixed") {
                return Err("must be fill, hug or a fixed size".to_owned());
            }
            match object.get("fixed") {
                Some(Value::Number(number)) if number.as_f64().is_some_and(|value| value > 0.0) => Ok(()),
                _ => Err("fixed size must be a positive number".to_owned()),
            }
        }
    }
}

/// Where a design's own data lives, beneath the workspace.
const DATA_DIR: &str = ".solace/design";

fn versions_dir(top: &std::path::Path, id: &str) -> std::path::PathBuf {
    top.join(DATA_DIR).join(VERSIONS_DIR).join(id)
}

fn most_recent_raw(dir: &std::path::Path, number: u64) -> Result<Option<Vec<u8>>, Failure> {
    for past in (1..number).rev() {
        let Ok(version) = crate::jsonx::read(&dir.join(format!("{past}.json"))) else { continue };
        if let Some(raw) = raw_bytes(&version) {
            return Ok(Some(raw));
        }
    }
    Ok(None)
}

/// The workspace's valid designs, as an open page lists them.
pub(crate) fn listed(top: &std::path::Path) -> Result<Value, Failure> {
    Ok(list(top)?["designs"].clone())
}

/// Announces the workspace's designs to open pages when they differ from
/// `seen`, and keeps `seen` up to date. The first read announces too, so a
/// change between a page's load and the watcher's start is not missed.
pub(crate) fn announce_change(top: &std::path::Path, seen: &mut Option<Value>) {
    let Ok(current) = listed(top) else { return };
    if seen.as_ref() == Some(&current) {
        return;
    }
    *seen = Some(current.clone());
    crate::tasks::store::log(top, "", &serde_json::json!({"designs": current}));
}

struct Current {
    design: Option<Value>,
    raw: Option<Vec<u8>>,
}

fn read_current(dir: &std::path::Path, id: &str) -> Result<Current, Failure> {
    let path = dir.join(format!("{id}.json"));
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let design = bytes.as_deref().and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok());
    let raw = if design.is_none() { bytes } else { None };
    Ok(Current { design, raw })
}

fn keep_current(
    versions: &std::path::Path,
    number: u64,
    current: &Current,
    latest_design: Option<&Value>,
    latest_raw: Option<&[u8]>,
) -> Result<u64, Failure> {
    if let Some(design) = &current.design {
        if latest_design != Some(design) {
            append_design(versions, number, &serde_json::json!({"kind": "repo"}), design)?;
            return Ok(number + 1);
        }
    } else if let Some(raw) = current.raw.as_deref()
        && latest_raw != Some(raw)
    {
        append_raw(versions, number, raw)?;
        return Ok(number + 1);
    }
    Ok(number)
}

fn append_design(dir: &std::path::Path, number: u64, source: &Value, design: &Value) -> Result<(), Failure> {
    let version = serde_json::json!({
        "number": number,
        "time": crate::now(),
        "source": source,
        "design": design,
    });
    crate::fsx::replace_bytes(dir, &format!("{number}.json"), version.to_string().as_bytes())
}

fn append_raw(dir: &std::path::Path, number: u64, bytes: &[u8]) -> Result<(), Failure> {
    let mut version = serde_json::json!({
        "number": number,
        "time": crate::now(),
        "source": {"kind": "repo"},
    });
    if let Ok(text) = std::str::from_utf8(bytes) {
        version["raw"] = serde_json::json!(text);
    } else {
        version["rawBase64"] = serde_json::json!(crate::base64::encode(bytes));
    }
    crate::fsx::replace_bytes(dir, &format!("{number}.json"), version.to_string().as_bytes())
}

fn raw_bytes(version: &Value) -> Option<Vec<u8>> {
    if let Some(text) = version.get("raw").and_then(Value::as_str) {
        return Some(text.as_bytes().to_vec());
    }
    version.get("rawBase64").and_then(Value::as_str).and_then(crate::base64::decode)
}

/// A version number from a request; anything but a whole number is `E7001`.
pub(crate) fn number(value: &Value) -> Result<u64, Failure> {
    value.as_u64().ok_or_else(|| Failure("E7001 version must be a whole number".to_string(), 1))
}

/// The workspace's versions of `id`, newest first, each without its content.
pub(crate) fn versions(top: &std::path::Path, id: &str) -> Result<Value, Failure> {
    let dir = history(top, id)?;
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => {
            return Ok(Value::Array(Vec::new()));
        }
        Err(e) => return Err(e.into()),
    };
    let mut listed = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let Some(number) = number_of(&path) else { continue };
        let Ok(version) = crate::jsonx::read(&path) else { continue };
        listed.push(serde_json::json!({
            "number": number,
            "time": version["time"],
            "source": version["source"],
        }));
    }
    listed.sort_by(|a, b| b["number"].as_u64().cmp(&a["number"].as_u64()));
    Ok(Value::Array(listed))
}

/// One stored version, whole. An unknown version is `E7023`.
pub(crate) fn version(top: &std::path::Path, id: &str, number: u64) -> Result<Value, Failure> {
    let dir = history(top, id)?;
    let path = dir.join(format!("{number}.json"));
    if number == 0 || !path.is_file() {
        return Err(Failure(format!("E7023 unknown version {number} of design {id}"), 1));
    }
    crate::jsonx::read(&path).map_err(|e| invalid(format!("version {number}: {}", e.0)))
}

/// Saves version `number` of `id` as the design's next version, through `save`,
/// and returns the versions as they stand. A version of another design, or one
/// that is not a valid design, is refused and changes nothing.
pub(crate) fn restore(top: &std::path::Path, id: &str, number: u64) -> Result<Value, Failure> {
    let record = version(top, id, number)?;
    let Some(design) = record.get("design") else {
        return Err(invalid(format!("version {number} holds no design")));
    };
    if design["id"].as_str() != Some(id) {
        return Err(invalid(format!("version {number} belongs to another design")));
    }
    save(top, design, Source::Human)?;
    versions(top, id)
}

fn history(top: &std::path::Path, id: &str) -> Result<std::path::PathBuf, Failure> {
    if !valid_id(id) {
        return Err(unknown(id));
    }
    let dir = versions_dir(top, id);
    if !dir.is_dir() && !top.join(DESIGNS_DIR).join(format!("{id}.json")).is_file() {
        return Err(unknown(id));
    }
    Ok(dir)
}

fn number_of(path: &std::path::Path) -> Option<u64> {
    if path.extension()? != "json" {
        return None;
    }
    path.file_stem()?.to_str()?.parse::<u64>().ok().filter(|number| *number > 0)
}

const HEADING: ElementType = ElementType {
    children: false,
    properties: &[
        ("width", ValueKind::Size),
        ("height", ValueKind::Size),
        ("align", ValueKind::OneOf(TEXT_ALIGN)),
        ("text", ValueKind::Text),
        ("level", ValueKind::Level),
        ("action", ValueKind::Text),
        ("navigates_to", ValueKind::Screen),
    ],
    required: &["width", "height", "text", "level"],
};

const BUTTON: ElementType = ElementType {
    children: false,
    properties: &[
        ("width", ValueKind::Size),
        ("height", ValueKind::Size),
        ("label", ValueKind::Text),
        ("variant", ValueKind::OneOf(BUTTON_VARIANTS)),
        ("action", ValueKind::Text),
        ("navigates_to", ValueKind::Screen),
    ],
    required: &["width", "height", "label"],
};

const TEXT_INPUT: ElementType = ElementType {
    children: false,
    properties: &[("width", ValueKind::Size), ("height", ValueKind::Size), ("placeholder", ValueKind::Text)],
    required: &["width", "height", "placeholder"],
};

const IMAGE_PLACEHOLDER: ElementType = ElementType {
    children: false,
    properties: &[
        ("width", ValueKind::Size),
        ("height", ValueKind::Size),
        ("label", ValueKind::Text),
        ("action", ValueKind::Text),
        ("navigates_to", ValueKind::Screen),
    ],
    required: &["width", "height", "label"],
};

/// A design as its link finds it (D-234): its title and its page's tab name,
/// `architecture` or `ui-ux`. A malformed id is `E7001`; no such design is
/// `E7023`; a file that is not a valid design is `E7033`.
pub(crate) fn find(top: &std::path::Path, id: &str) -> Result<Value, (u16, Failure)> {
    if !valid_id(id) {
        return Err((400, Failure(format!("E7001 invalid design id {id}"), 1)));
    }
    let design = open(top, id)?;
    let tab = match design["tab"].as_str() {
        Some("ui") => "ui-ux",
        _ => "architecture",
    };
    Ok(serde_json::json!({"id": id, "title": design["title"], "tab": tab}))
}

/// The design `id` names, read and checked from the workspace: `E7023` when
/// its file is missing, `E7033` when the file is not a valid design. The
/// caller checks `id` first, because its own answer differs.
fn open(top: &std::path::Path, id: &str) -> Result<Value, (u16, Failure)> {
    let path = top.join(DESIGNS_DIR).join(format!("{id}.json"));
    if !path.is_file() {
        return Err((404, unknown(id)));
    }
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
    read_file(&path, name).map_err(|e| (409, e))
}

/// `design list | show <id> [--from <element>]`: the workspace's designs, read as the designs page reads them.
pub(crate) fn command(top: &std::path::Path, args: &[&str], budget: usize) -> Result<String, Failure> {
    match args {
        ["list"] => {
            let designs = list(top)?;
            Ok(limited(pretty(&designs), budget, "use `design show <id>` for one design."))
        }
        ["show", id, rest @ ..] => {
            let id = id.strip_prefix("design:").unwrap_or(id);
            let from = from_flag(rest)?;
            match show(top, id) {
                Ok(design) => show_page(&design, id, from, budget),
                Err(e) if e.0.starts_with(UNKNOWN) => Err(Failure(format!("{}; use `design list` to list designs", e.0), e.1)),
                Err(e) => Err(e),
            }
        }
        _ => crate::failure::usage("design list | design show <id> [--from <element>]; the id may be written `design:<id>`"),
    }
}

fn pretty(value: &Value) -> String {
    format!("{}\n", serde_json::to_string_pretty(value).unwrap())
}

fn limited(text: String, budget: usize, why: &str) -> String {
    if text.len() <= budget {
        return text;
    }
    cut_reply(text, budget, why)
}

/// The spacing tokens a design may name, shared by gap and padding.
const SPACING_TOKENS: &[&str] = &["none", "xs", "sm", "md", "lg", "xl"];

/// The words every unknown design id starts with, shared with the CLI's hint.
const UNKNOWN: &str = "E7023 unknown design";

/// `--from <element>` after `design show <id>`: the first element to read.
fn from_flag<'a>(rest: &[&'a str]) -> Result<Option<&'a str>, Failure> {
    match rest {
        [] => Ok(None),
        ["--from", first] => Ok(Some(*first)),
        _ => crate::failure::usage("after `design show <id>`, only `--from <element>` may follow"),
    }
}

/// One design, or its elements from `from` on, cut at whole elements.
fn show_page(design: &Value, id: &str, from: Option<&str>, budget: usize) -> Result<String, Failure> {
    let elements = design["elements"].as_array().cloned().unwrap_or_default();
    let count = elements.len();
    let start = match from {
        None => 0,
        Some(first) => match elements.iter().position(|e| e["id"].as_str() == Some(first)) {
            Some(start) => start,
            None => return crate::failure::usage(format!("design {id} has no element `{first}`")),
        },
    };
    let page = |end: usize| {
        let mut shown = design.clone();
        shown["elements"] = Value::Array(elements[start..end].to_vec());
        pretty(&shown)
    };
    let whole = page(count);
    if whole.len() <= budget {
        return Ok(whole);
    }
    if start + 1 >= count {
        let why = if count == 0 {
            "the design has no elements to show.".to_owned()
        } else {
            format!("element `{}` is too large to show whole.", elements[start]["id"].as_str().unwrap_or(""))
        };
        return Ok(limited(whole, budget, &why));
    }
    let why = |end: usize| {
        format!(
            "the design has {count} elements, starting at `{}`; read on with `design show {id} --from {}`.",
            elements[start]["id"].as_str().unwrap_or(""),
            elements[end]["id"].as_str().unwrap_or(""),
        )
    };
    let note = |end: usize| format!("\n… cut at the reply limit; {}\n", why(end));
    let (mut lo, mut hi, mut best) = (start + 1, count - 1, None);
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        if page(mid).len() + note(mid).len() <= budget {
            best = Some(mid);
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    match best {
        Some(end) => Ok(format!("{}{}", page(end), note(end))),
        None => Ok(cut_reply(page(start + 1), budget, &why(start + 1))),
    }
}

/// Cut `text` to leave room for the reply-limit note, which always survives.
fn cut_reply(text: String, budget: usize, why: &str) -> String {
    let note = format!("\n… cut at the reply limit; {why}\n");
    let room = budget.saturating_sub(note.len());
    let cut = (0..=room.min(text.len())).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
    let mut out = format!("{}{note}", &text[..cut]);
    out.truncate(out.floor_char_boundary(budget));
    out
}

const SCREEN: ElementType = ElementType {
    children: true,
    properties: &[
        ("width", ValueKind::Size),
        ("height", ValueKind::Size),
        ("gap", ValueKind::Spacing),
        ("padding", ValueKind::Spacing),
        ("align", ValueKind::OneOf(ALIGN)),
        ("justify", ValueKind::OneOf(JUSTIFY)),
    ],
    required: &["width", "height"],
};

const BUTTON_VARIANTS: &[&str] = &["primary", "secondary", "ghost"];

const TEXT_VARIANTS: &[&str] = &["body", "muted", "small"];

const CONTAINER_VARIANTS: &[&str] = &["plain", "card"];

/// The color tokens a style override may name.
const COLOR_TOKENS: &[&str] = &["background", "surface", "text", "muted", "primary", "on_primary", "border"];

/// The text-size tokens a style override may name.
const TEXT_SIZE_TOKENS: &[&str] = &["small", "body", "h3", "h2", "h1"];

/// The closed set of per-element style overrides and the kind of value each takes.
const STYLE_PROPERTIES: &[(&str, ValueKind)] = &[
    ("fill", ValueKind::Color),
    ("border", ValueKind::Color),
    ("text_color", ValueKind::Color),
    ("size", ValueKind::TextSize),
    ("padding", ValueKind::Spacing),
];

fn is_hex_color(text: &str) -> bool {
    let Some(digits) = text.strip_prefix('#') else {
        return false;
    };
    (digits.len() == 3 || digits.len() == 6) && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn check_color(value: &Value) -> Result<(), String> {
    match value.as_str() {
        Some(text) if COLOR_TOKENS.contains(&text) || is_hex_color(text) => Ok(()),
        Some(text) => Err(format!("has value {text:?}; expected one of {} or a hex color such as #fff", COLOR_TOKENS.join(", "))),
        None => Err(format!("must be a color token ({}) or a hex color such as #fff or #ffffff", COLOR_TOKENS.join(", "))),
    }
}

fn check_text_size(value: &Value) -> Result<(), String> {
    match value.as_str() {
        Some(text) if TEXT_SIZE_TOKENS.contains(&text) => Ok(()),
        Some(text) => Err(format!("has value {text:?}; expected one of {} or a positive number", TEXT_SIZE_TOKENS.join(", "))),
        None => match value.as_f64() {
            Some(number) if number > 0.0 => Ok(()),
            Some(_) => Err("must be a positive number".to_owned()),
            None => Err(format!("must be a text size token ({}) or a positive number", TEXT_SIZE_TOKENS.join(", "))),
        },
    }
}
