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
    /// The id of a node in the same flow diagram.
    Node,
    /// The id of a tab inside the same tabs element.
    Tab,
}

/// An element type and the properties it accepts.
pub(crate) struct ElementType {
    /// Whether the type may hold children.
    pub(crate) children: bool,
    /// A minimal valid element of this type, as JSON text.
    pub(crate) example: &'static str,
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
    ("component", COMPONENT),
    ("tabs", TABS_ELEMENT),
    ("tab", TAB),
    ("scroll_area", SCROLL_AREA),
    ("split_pane", SPLIT_PANE),
    ("flow", FLOW),
    ("node", NODE),
    ("edge", EDGE),
];

const DESIGN_KEYS: &[&str] = &["schemaVersion", "id", "title", "tab", "elements"];

const ELEMENT_KEYS: &[&str] = &["id", "type", "properties", "style", "children", "conditions"];

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
        let (id, ty) = validate_element(element, &mut ids, &screens, None, true, None)?;
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
    parent: Option<&str>,
    top_level: bool,
    flow: Option<&HashSet<String>>,
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
    if ty == "flow" && !top_level {
        return Err(invalid(format!("{at} is a flow below the top level")));
    }
    if ty == "tab" && parent != Some("tabs") {
        return Err(invalid(format!("{at} of type {ty:?} must be inside a tabs element")));
    }
    if matches!(ty, "node" | "edge") {
        if flow.is_none() {
            return Err(invalid(format!("{at} of type {ty:?} must be inside a flow")));
        }
    } else if flow.is_some() {
        return Err(invalid(format!("{at} of type {ty:?} cannot be inside a flow")));
    }
    if ty != "screen" && object.contains_key("conditions") {
        return Err(invalid(format!("{at} of type {ty:?} cannot hold conditions; only a screen can")));
    }
    let properties =
        object.get("properties").and_then(Value::as_object).ok_or_else(|| invalid(format!("{at} properties must be an object")))?;
    for (name, value) in properties {
        let Some((_, values)) = kind.properties.iter().find(|(property, _)| *property == name) else {
            return Err(invalid(format!("{at} has unknown property {name:?}")));
        };
        values.check(value).map_err(|why| invalid(format!("{at} property {name:?} {why}")))?;
        if let ValueKind::Screen = values {
            check_reference(values, value, screens, None, &at, name)?;
        }
        if let ValueKind::Node = values {
            let target = value.as_str().unwrap_or_default();
            if !flow.is_some_and(|nodes| nodes.contains(target)) {
                return Err(invalid(format!("{at} property {name:?} names no node {target:?}")));
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
        if ty == "tabs" && children.is_empty() {
            return Err(invalid(format!("{at} needs at least one tab")));
        }
        if ty == "split_pane" && children.len() != 2 {
            return Err(invalid(format!("{at} needs exactly two regions, not {}", children.len())));
        }
        let nodes: Option<HashSet<String>> = (ty == "flow").then(|| {
            children
                .iter()
                .filter(|child| child.get("type").and_then(Value::as_str) == Some("node"))
                .filter_map(|child| child.get("id").and_then(Value::as_str).map(str::to_owned))
                .collect()
        });
        for child in children {
            validate_element(child, ids, screens, Some(ty), false, nodes.as_ref())?;
        }
        if ty == "tabs" {
            if let Some(child) = children.iter().find(|child| child.get("type").and_then(Value::as_str) != Some("tab")) {
                let child_id = child.get("id").and_then(Value::as_str).unwrap_or_default();
                let child_ty = child.get("type").and_then(Value::as_str).unwrap_or_default();
                return Err(invalid(format!("{at} holds {child_id:?} of type {child_ty:?}; tabs hold only tabs")));
            }
            let current = properties.get("current").unwrap_or(&Value::Null);
            check_reference(&ValueKind::Tab, current, screens, Some(children.as_slice()), &at, "current")?;
        }
    } else if ty == "tabs" {
        return Err(invalid(format!("{at} needs at least one tab")));
    } else if ty == "split_pane" {
        return Err(invalid(format!("{at} needs exactly two regions, not 0")));
    }
    if ty == "screen" {
        validate_conditions(value, &at, screens)?;
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
            ValueKind::Node => match value.as_str() {
                Some(_) => Ok(()),
                None => Err("must name a node".to_owned()),
            },
            ValueKind::Tab => match value.as_str() {
                Some(_) => Ok(()),
                None => Err("must name a tab".to_owned()),
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
                let id = design["id"].as_str().unwrap_or_default();
                designs.push(serde_json::json!({
                    "id": design["id"],
                    "title": design["title"],
                    "tab": design["tab"],
                    "version": saved_version(top, id),
                }));
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
pub(crate) fn save(top: &std::path::Path, design: &Value, source: Source<'_>) -> Result<Option<u64>, Failure> {
    validate(design)?;
    let data = top.join(DATA_DIR);
    std::fs::create_dir_all(&data)?;
    let _lock = crate::fsx::lock_file(&data.join("lock"))?;
    save_locked(top, design, source)
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
    example: r#"{"id":"row","type":"row","properties":{"width":"fill","height":"hug","gap":"md"}}"#,
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
    example: r#"{"id":"text","type":"text","properties":{"width":"hug","height":"hug","text":"Hello"}}"#,
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
/// change between a page's load and the watcher's start is not missed. A
/// notice that cannot be written leaves `seen` unchanged, so a later check
/// tries again.
pub(crate) fn announce_change(top: &std::path::Path, seen: &mut Option<Value>) {
    let Ok(current) = listed(top) else { return };
    if seen.as_ref() == Some(&current) {
        return;
    }
    if crate::tasks::store::log(top, "", &serde_json::json!({"designs": current.clone()})) {
        *seen = Some(current);
    }
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
    example: r#"{"id":"heading","type":"heading","properties":{"width":"hug","height":"hug","text":"Title","level":2}}"#,
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
    example: r#"{"id":"button","type":"button","properties":{"width":"hug","height":"hug","label":"Save"}}"#,
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
    example: r#"{"id":"text_input","type":"text_input","properties":{"width":"fill","height":"hug","placeholder":"Name"}}"#,
    properties: &[("width", ValueKind::Size), ("height", ValueKind::Size), ("placeholder", ValueKind::Text)],
    required: &["width", "height", "placeholder"],
};

const IMAGE_PLACEHOLDER: ElementType = ElementType {
    children: false,
    example: concat!(
        r#"{"id":"image_placeholder","type":"image_placeholder","properties":"#,
        r#"{"width":"fill","height":{"fixed":200},"label":"Photo"}}"#,
    ),
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
    command_with_version(top, args, budget).map(|(text, _)| text)
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
    example: r#"{"id":"screen","type":"screen","properties":{"width":"fill","height":"fill"}}"#,
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
pub(crate) const STYLE_PROPERTIES: &[(&str, ValueKind)] = &[
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

/// Applies design `operations` to `design` in place, one element at a time (D-241).
/// An element added without an `id` gets one that the design does not already
/// use, and `apply` returns every id it gave out.
pub(crate) fn apply(design: &mut Value, operations: &[Value]) -> Result<Vec<GivenElementId>, Failure> {
    fn used_ids(design: &Value, operations: &[Value]) -> HashSet<String> {
        fn walk(element: &Value, ids: &mut HashSet<String>) {
            if let Some(id) = element.get("id").and_then(Value::as_str) {
                ids.insert(id.to_string());
            }
            if let Some(children) = element.get("children").and_then(Value::as_array) {
                for child in children {
                    walk(child, ids);
                }
            }
        }
        let mut ids = HashSet::new();
        if let Some(elements) = design.get("elements").and_then(Value::as_array) {
            for element in elements {
                walk(element, &mut ids);
            }
        }
        for operation in operations {
            if operation.get("op").and_then(Value::as_str) == Some("add") {
                if let Some(element) = operation.get("element") {
                    walk(element, &mut ids);
                }
            }
        }
        ids
    }

    let mut used = used_ids(design, operations);
    let mut given = Vec::new();
    for operation in operations {
        apply_operation(design, operation, &mut used, &mut given)?;
    }
    Ok(given)
}

fn apply_operation(
    design: &mut Value,
    operation: &Value,
    used: &mut HashSet<String>,
    given: &mut Vec<GivenElementId>,
) -> Result<(), Failure> {
    let op = operation.as_object().ok_or_else(|| operation_error("each operation must be a JSON object"))?;
    let kind = op.get("op").and_then(Value::as_str).ok_or_else(|| operation_error("an operation needs an `op`"))?;
    let root = design["elements"].as_array_mut().ok_or_else(|| invalid("design has no list of elements"))?;
    match kind {
        "add" => add_element(root, op, used, given),
        "remove" => remove_element(root, op),
        "move" => move_element(root, op),
        "set" => set_property(root, op),
        other => Err(operation_error(format!("unknown operation {other:?}"))),
    }
}

fn operation_error(why: impl std::fmt::Display) -> Failure {
    Failure(format!("E7001 invalid operation: {why}"), 1)
}

fn element_id<'a>(operation: &'a Map<String, Value>) -> Result<&'a str, Failure> {
    operation.get("id").and_then(Value::as_str).ok_or_else(|| operation_error("the operation needs an element `id`"))
}

fn parent_id(operation: &Map<String, Value>) -> Result<Option<&str>, Failure> {
    match operation.get("parent") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) => Ok(Some(id)),
        Some(_) => Err(operation_error("`parent` must be an element id or null")),
    }
}

fn index_of(operation: &Map<String, Value>) -> Result<usize, Failure> {
    operation
        .get("index")
        .and_then(Value::as_u64)
        .map(|index| index as usize)
        .ok_or_else(|| operation_error("the operation needs a whole-number `index`"))
}

fn element_path(elements: &[Value], id: &str) -> Option<Vec<usize>> {
    for (index, element) in elements.iter().enumerate() {
        if element["id"].as_str() == Some(id) {
            return Some(vec![index]);
        }
        if let Some(children) = element["children"].as_array() {
            if let Some(mut path) = element_path(children, id) {
                path.insert(0, index);
                return Some(path);
            }
        }
    }
    None
}

fn children_mut<'a>(elements: &'a mut Vec<Value>, parent: &[usize]) -> Result<&'a mut Vec<Value>, Failure> {
    if parent.is_empty() {
        return Ok(elements);
    }
    let mut element = &mut elements[parent[0]];
    for &index in &parent[1..] {
        element = &mut element["children"].as_array_mut().ok_or_else(|| invalid("an element lost its children"))?[index];
    }
    let object = element.as_object_mut().ok_or_else(|| invalid("an element is not an object"))?;
    if !object.contains_key("children") {
        object.insert("children".to_string(), serde_json::json!([]));
    }
    object.get_mut("children").and_then(Value::as_array_mut).ok_or_else(|| invalid("an element's children must be a list"))
}

fn element_at_mut<'a>(elements: &'a mut Vec<Value>, path: &[usize]) -> Result<&'a mut Value, Failure> {
    let Some((first, rest)) = path.split_first() else {
        return Err(invalid("an element is missing"));
    };
    let mut element = elements.get_mut(*first).ok_or_else(|| invalid("an element is missing"))?;
    for &index in rest {
        element = &mut element["children"].as_array_mut().ok_or_else(|| invalid("an element lost its children"))?[index];
    }
    Ok(element)
}

fn add_element(
    root: &mut Vec<Value>,
    op: &Map<String, Value>,
    used: &mut HashSet<String>,
    given: &mut Vec<GivenElementId>,
) -> Result<(), Failure> {
    let mut element = op.get("element").cloned().ok_or_else(|| operation_error("add needs an `element`"))?;
    if !element.is_object() {
        return Err(operation_error("add's `element` must be a JSON object"));
    }
    let index = index_of(op)?;
    let parent = parent_id(op)?;
    give_ids(&mut element, parent, used, given);
    let parent_path = match parent {
        Some(id) => element_path(root, id).ok_or_else(|| invalid(format!("element {id:?} does not exist")))?,
        None => Vec::new(),
    };
    let list = children_mut(root, &parent_path)?;
    if index > list.len() {
        return Err(operation_error(format!("add index {index} is outside {} children", list.len())));
    }
    list.insert(index, element);
    Ok(())
}

fn remove_element(root: &mut Vec<Value>, op: &Map<String, Value>) -> Result<(), Failure> {
    let id = element_id(op)?;
    let path = element_path(root, id).ok_or_else(|| invalid(format!("element {id:?} does not exist")))?;
    let (last, parent_path) = path.split_last().ok_or_else(|| invalid(format!("element {id:?} does not exist")))?;
    children_mut(root, parent_path)?.remove(*last);
    Ok(())
}

fn move_element(root: &mut Vec<Value>, op: &Map<String, Value>) -> Result<(), Failure> {
    let id = element_id(op)?;
    let index = index_of(op)?;
    let parent = parent_id(op)?;
    let path = element_path(root, id).ok_or_else(|| invalid(format!("element {id:?} does not exist")))?;
    if let Some(parent_id) = parent {
        let target = element_path(root, parent_id).ok_or_else(|| invalid(format!("element {parent_id:?} does not exist")))?;
        if target.starts_with(&path) {
            return Err(invalid(format!("element {id:?} can't move inside itself")));
        }
    }
    let (last, parent_path) = path.split_last().ok_or_else(|| invalid(format!("element {id:?} does not exist")))?;
    let element = children_mut(root, parent_path)?.remove(*last);
    let target_path = match parent {
        Some(id) => element_path(root, id).ok_or_else(|| invalid(format!("element {id:?} does not exist")))?,
        None => Vec::new(),
    };
    let length = children_mut(root, &target_path)?.len();
    if index > length {
        children_mut(root, parent_path)?.insert(*last, element);
        return Err(operation_error(format!("move index {index} is outside {length} children")));
    }
    let list = children_mut(root, &target_path)?;
    list.insert(index, element);
    Ok(())
}

fn set_property(root: &mut Vec<Value>, op: &Map<String, Value>) -> Result<(), Failure> {
    let id = element_id(op)?;
    let field = op.get("field").and_then(Value::as_str).ok_or_else(|| operation_error("set needs a `field`"))?;
    let value = op.get("value").ok_or_else(|| operation_error("set needs a `value`"))?;
    let path = element_path(root, id).ok_or_else(|| invalid(format!("element {id:?} does not exist")))?;
    let element = element_at_mut(root, &path)?;
    if field == "conditions" {
        return set_condition(element, op);
    }
    let property = op.get("property").and_then(Value::as_str).ok_or_else(|| operation_error("set needs a `property`"))?;
    let object = match field {
        "properties" => element
            .get_mut("properties")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| invalid(format!("element {id:?} has no properties")))?,
        "style" => {
            let element = element.as_object_mut().ok_or_else(|| invalid(format!("element {id:?} is not an object")))?;
            if element.get("style").is_none() {
                if value.is_null() {
                    return Ok(());
                }
                element.insert("style".to_string(), serde_json::json!({}));
            }
            element["style"].as_object_mut().ok_or_else(|| invalid(format!("element {id:?} style must be an object")))?
        }
        other => return Err(operation_error(format!("set field must be \"properties\", \"style\" or \"conditions\", not {other:?}"))),
    };
    if value.is_null() {
        object.remove(property);
    } else {
        object.insert(property.to_string(), value.clone());
    }
    Ok(())
}

const FLOW: ElementType =
    ElementType { children: true, example: r#"{"id":"flow","type":"flow","properties":{}}"#, properties: &[], required: &[] };

const NODE: ElementType = ElementType {
    children: false,
    example: r#"{"id":"node","type":"node","properties":{"label":"Step","description":"What happens"}}"#,
    properties: &[("label", ValueKind::Text), ("description", ValueKind::Text)],
    required: &["label", "description"],
};

const EDGE: ElementType = ElementType {
    children: false,
    example: r#"{"id":"edge","type":"edge","properties":{"from":"step","to":"done"}}"#,
    properties: &[("from", ValueKind::Node), ("to", ValueKind::Node), ("label", ValueKind::Text)],
    required: &["from", "to"],
};

/// The element `id` in `design`, with its type and its ancestors, top first.
pub(crate) fn element_details(design: &Value, id: &str) -> Option<Value> {
    fn walk(elements: &[Value], id: &str, parents: &mut Vec<Value>) -> Option<Value> {
        for element in elements {
            if element.get("id").and_then(Value::as_str) == Some(id) {
                return Some(serde_json::json!({
                    "id": id,
                    "type": element.get("type").and_then(Value::as_str),
                    "parents": parents.clone(),
                }));
            }
            if let Some(children) = element.get("children").and_then(Value::as_array) {
                parents.push(serde_json::json!({
                    "id": element.get("id").and_then(Value::as_str),
                    "type": element.get("type").and_then(Value::as_str),
                }));
                if let Some(found) = walk(children, id, parents) {
                    return Some(found);
                }
                parents.pop();
            }
        }
        None
    }
    walk(design.get("elements").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]), id, &mut vec![])
}

fn note_invalid(why: impl std::fmt::Display) -> Failure {
    Failure(format!("E7001 {why}"), 1)
}

/// A message's design note (D-245): its design and text, and each named element
/// with its type and ancestors, worked out when the message is sent.
pub(crate) fn resolve_note(top: &std::path::Path, value: &Value) -> Result<Value, (u16, Failure)> {
    let object = value.as_object().ok_or_else(|| (400, note_invalid("a note must be a JSON object")))?;
    unknown_keys(object, &["design", "elements", "text"], "a note").map_err(|e| (400, e))?;
    let design_id = object
        .get("design")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| (400, note_invalid("a note needs a design")))?;
    let text = object
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| (400, note_invalid("a note needs text")))?;
    let ids = object
        .get("elements")
        .and_then(Value::as_array)
        .filter(|ids| !ids.is_empty())
        .ok_or_else(|| (400, note_invalid("a note needs at least one element")))?;
    let design = show(top, design_id).map_err(|failure| {
        let status = if failure.0.starts_with("E7023") { 404 } else { 409 };
        (status, failure)
    })?;
    let elements = ids
        .iter()
        .map(|id| {
            let id = id.as_str().ok_or_else(|| (400, note_invalid("a note's elements must be ids")))?;
            element_details(&design, id).ok_or_else(|| (404, Failure(format!("E7023 unknown element {id} in design {design_id}"), 1)))
        })
        .collect::<Result<Vec<Value>, (u16, Failure)>>()?;
    Ok(serde_json::json!({
        "design": design_id,
        "title": design["title"],
        "text": text,
        "elements": elements,
    }))
}

/// The newest saved version of a design, or zero when it has no history.
fn saved_version(top: &std::path::Path, id: &str) -> u64 {
    next_version(&versions_dir(top, id)).map(|next| next.saturating_sub(1)).unwrap_or(0)
}

/// Where deleted designs wait for Undo, beneath a design's own data.
const DELETED_DIR: &str = "deleted";

/// How long a deleted design stays available to Undo.
const UNDO_SECONDS: u64 = 24 * 60 * 60;

/// Renames `id` through the save path, so the old title stays in its versions.
/// A blank title is refused by validation.
pub(crate) fn rename(top: &std::path::Path, id: &str, title: &str) -> Result<Value, Failure> {
    let data = top.join(DATA_DIR);
    std::fs::create_dir_all(&data)?;
    let _lock = crate::fsx::lock_file(&data.join("lock"))?;
    let mut design = show(top, id)?;
    let title = title.trim();
    if design["title"].as_str() == Some(title) {
        return Ok(design);
    }
    design["title"] = serde_json::json!(title);
    save_locked(top, &design, Source::Human)?;
    Ok(design)
}

/// Whether `id` has been deleted and not put back by Undo.
pub(crate) fn deleted(top: &std::path::Path, id: &str) -> bool {
    deleted_at(top, id).is_some()
}

/// Removes `id` and all its versions from the workspace, keeping them in the
/// trash so `undelete` can put them back. Takes the save lock, so a delete
/// cannot interleave with a save.
pub(crate) fn delete(top: &std::path::Path, id: &str) -> Result<Value, Failure> {
    if !valid_id(id) {
        return Err(unknown(id));
    }
    let data = top.join(DATA_DIR);
    std::fs::create_dir_all(&data)?;
    let _lock = crate::fsx::lock_file(&data.join("lock"))?;
    purge_trash(&data);
    let file = top.join(DESIGNS_DIR).join(format!("{id}.json"));
    let versions = versions_dir(top, id);
    if !file.is_file() && !versions.is_dir() {
        return Err(unknown(id));
    }
    let trash = trash_dir(top, id);
    if trash.exists() {
        std::fs::remove_dir_all(&trash)?;
    }
    std::fs::create_dir_all(&trash)?;
    let mut moved: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();
    for (from, to) in [(file, trash.join("design.json")), (versions, trash.join(VERSIONS_DIR))] {
        if !from.exists() {
            continue;
        }
        if let Err(e) = std::fs::rename(&from, &to) {
            for (from, to) in moved.iter().rev() {
                let _ = std::fs::rename(to, from);
            }
            let _ = std::fs::remove_dir_all(&trash);
            return Err(e.into());
        }
        moved.push((from, to));
    }
    let time = crate::now();
    crate::jsonx::replace_pretty(&trash, "deleted.json", &serde_json::json!({"time": time}))?;
    if let Err(e) = remember_deleted(&data, id, time) {
        for (from, to) in moved.iter().rev() {
            let _ = std::fs::rename(to, from);
        }
        let _ = std::fs::remove_dir_all(&trash);
        return Err(e);
    }
    list(top)
}

/// Puts a deleted design and its versions back. A design saved again under the
/// same id is left alone: Undo never overwrites it.
pub(crate) fn undelete(top: &std::path::Path, id: &str) -> Result<Value, Failure> {
    if !valid_id(id) {
        return Err(unknown(id));
    }
    let data = top.join(DATA_DIR);
    std::fs::create_dir_all(&data)?;
    let _lock = crate::fsx::lock_file(&data.join("lock"))?;
    purge_trash(&data);
    let trash = trash_dir(top, id);
    if !trash.is_dir() {
        return Err(unknown(id));
    }
    let file = top.join(DESIGNS_DIR).join(format!("{id}.json"));
    let versions = versions_dir(top, id);
    if file.is_file() || versions.is_dir() {
        return Err(Failure(format!("E7023 design {id} was saved again; undo would overwrite it"), 1));
    }
    std::fs::create_dir_all(top.join(DESIGNS_DIR))?;
    if let Some(parent) = versions.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut moved: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();
    for (from, to) in [(trash.join("design.json"), file), (trash.join(VERSIONS_DIR), versions)] {
        if !from.exists() {
            continue;
        }
        if let Err(e) = std::fs::rename(&from, &to) {
            for (from, to) in moved.iter().rev() {
                let _ = std::fs::rename(to, from);
            }
            return Err(e.into());
        }
        moved.push((from, to));
    }
    std::fs::remove_dir_all(&trash)?;
    forget_deleted(&data, id)?;
    list(top)
}

pub(crate) fn trash_dir(top: &std::path::Path, id: &str) -> std::path::PathBuf {
    top.join(DATA_DIR).join(DELETED_DIR).join(id)
}

/// Drops deleted designs whose Undo window has passed.
pub(crate) fn purge_trash(data: &std::path::Path) {
    let dir = data.join(DELETED_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for entry in entries.flatten() {
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else { continue };
        let Ok(age) = std::time::SystemTime::now().duration_since(modified) else { continue };
        if age.as_secs() > UNDO_SECONDS {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Saves `design` under the save lock after `guard` agrees. The guard sees the
/// same locked state the save writes from.
pub(crate) fn save_guarded(
    top: &std::path::Path,
    design: &Value,
    source: Source<'_>,
    guard: impl FnOnce() -> Result<(), Failure>,
) -> Result<Option<u64>, Failure> {
    validate(design)?;
    let data = top.join(DATA_DIR);
    std::fs::create_dir_all(&data)?;
    let _lock = crate::fsx::lock_file(&data.join("lock"))?;
    guard()?;
    save_locked(top, design, source)
}

fn save_locked(top: &std::path::Path, design: &Value, source: Source<'_>) -> Result<Option<u64>, Failure> {
    validate(design)?;
    let id = design["id"].as_str().unwrap_or_default();
    let dir = top.join(DESIGNS_DIR);
    let current = read_current(&dir, id)?;
    if current.design.as_ref() == Some(design) {
        return Ok(None);
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
    let wrote = if number != first || !design_is_latest {
        append_design(&versions, number, &source, design)?;
        Some(number)
    } else {
        None
    };
    crate::jsonx::replace_pretty(&dir, &format!("{id}.json"), design)?;
    Ok(wrote)
}

/// Deleted ids and their delete times, kept after their trash is purged so a
/// proposal made before a delete can never bring the design back.
const DELETIONS_FILE: &str = "deletions.json";

/// When `id` was last deleted, in seconds since the epoch.
pub(crate) fn deleted_at(top: &std::path::Path, id: &str) -> Option<u64> {
    deletion_record(&top.join(DATA_DIR)).get(id).and_then(Value::as_u64)
}

fn deletion_record(data: &std::path::Path) -> Value {
    crate::jsonx::read(&data.join(DELETIONS_FILE)).unwrap_or_else(|_| serde_json::json!({}))
}

fn remember_deleted(data: &std::path::Path, id: &str, time: u64) -> Result<(), Failure> {
    let mut record = deletion_record(data);
    if !record.is_object() {
        record = serde_json::json!({});
    }
    record[id] = serde_json::json!(time);
    crate::jsonx::replace_pretty(data, DELETIONS_FILE, &record)
}

fn forget_deleted(data: &std::path::Path, id: &str) -> Result<(), Failure> {
    let mut record = deletion_record(data);
    let Some(object) = record.as_object_mut() else { return Ok(()) };
    if object.remove(id).is_none() {
        return Ok(());
    }
    crate::jsonx::replace_pretty(data, DELETIONS_FILE, &record)
}

/// The newest saved version of `id`, or null when it has none.
pub(crate) fn newest_version(top: &std::path::Path, id: &str) -> Result<Value, Failure> {
    Ok(versions(top, id)?.as_array().and_then(|versions| versions.first()).map(|version| version["number"].clone()).unwrap_or(Value::Null))
}

/// One design and the newest version of its content, read under the save lock
/// so a save cannot put a newer version beside content it does not describe.
pub(crate) fn show_with_version(top: &std::path::Path, id: &str) -> Result<(Value, Value), Failure> {
    if !valid_id(id) {
        return Err(unknown(id));
    }
    let data = top.join(DATA_DIR);
    std::fs::create_dir_all(&data)?;
    let _lock = crate::fsx::lock_file(&data.join("lock"))?;
    let design = open(top, id).map_err(|(_, failure)| failure)?;
    let version = newest_version(top, id)?;
    Ok((design, version))
}

/// `design list | show <id> [--from <element>]`, and the version a show displayed.
pub(crate) fn command_with_version(top: &std::path::Path, args: &[&str], budget: usize) -> Result<(String, Option<Value>), Failure> {
    match args {
        ["list"] => {
            let designs = list(top)?;
            Ok((limited(pretty(&designs), budget, "use `design show <id>` for one design."), None))
        }
        ["show", id, rest @ ..] => {
            let id = id.strip_prefix("design:").unwrap_or(id);
            let from = from_flag(rest)?;
            match show_with_version(top, id) {
                Ok((design, version)) => Ok((show_page(&design, id, from, budget)?, Some(version))),
                Err(e) if e.0.starts_with(UNKNOWN) => Err(Failure(format!("{}; use `design list` to list designs", e.0), e.1)),
                Err(e) => Err(e),
            }
        }
        ["schema"] => Ok((schema(), None)),
        _ => {
            crate::failure::usage("design list | design show <id> [--from <element>] | design schema; the id may be written `design:<id>`")
        }
    }
}

/// Applies a human's `operations` to design `id` through the save path.
/// Nothing is saved unless every operation applies and the result validates.
pub(crate) fn edit(top: &std::path::Path, id: &str, operations: &[Value]) -> Result<Value, Failure> {
    let data = top.join(DATA_DIR);
    std::fs::create_dir_all(&data)?;
    let _lock = crate::fsx::lock_file(&data.join("lock"))?;
    let mut design = show(top, id)?;
    apply(&mut design, operations)?;
    save_locked(top, &design, Source::Human)?;
    Ok(design)
}

/// The design format, built from the same table `validate` checks.
pub(crate) fn schema() -> String {
    let mut out = String::new();
    line(&mut out, format!("Design format (schemaVersion {SCHEMA_VERSION})"));
    line(&mut out, "");
    line(&mut out, format!("Design keys: {}", DESIGN_KEYS.join(", ")));
    line(&mut out, format!("Tabs: {}", TABS.join(", ")));
    line(&mut out, format!("Element keys: {}", ELEMENT_KEYS.join(", ")));
    line(&mut out, "Design id: 16 lowercase hex digits; element ids: any unique text; title: any text");
    line(&mut out, "");
    line(
        &mut out,
        concat!(
            "Every element is an object with `type`, `properties`, an optional `style` and optional `children`; ",
            "`id` is any unique text and may be left out in an `add`, where the server gives it one.",
        ),
    );
    line(&mut out, format!("Style overrides: {}", describe_properties(STYLE_PROPERTIES)));
    line(&mut out, "");
    line(&mut out, "Placement");
    line(&mut out, "- `screen` and `flow` are only at the top level.");
    line(&mut out, "- `node` and `edge` are only inside a `flow`; an edge's `from` and `to` name nodes in that flow.");
    line(&mut out, "- A `flow` holds only `node` and `edge`.");
    line(&mut out, "- A `tab` is only inside a `tabs` element; `tabs` holds only tabs and needs at least one.");
    line(&mut out, "- A `tabs` element's `current` names one of its tab children.");
    line(&mut out, "- A `split_pane` holds exactly two regions; its `direction` is `row` (side by side) or `column` (stacked).");
    line(&mut out, "- `navigates_to` names a screen in the same design.");
    line(&mut out, "- A `screen` may state `conditions`, a list of window conditions; no other element may.");
    line(
        &mut out,
        concat!(
            "  A condition is {\"when\": {\"ratio\" | \"width\" | \"height\": {\"at_least\" | \"at_most\": <positive number>}}, ",
            "\"changes\": [<change>]}. A change is ",
            "{\"id\": <element id in that screen>, \"direction\": \"row\" | \"column\"} for a row or column, or ",
            "{\"id\": <element id in that screen>, \"property\": <name>, \"value\": <value>} for one of its properties.",
        ),
    );
    line(&mut out, "  Conditions apply in list order when their bound matches; a later matching condition overrides an earlier one.");
    line(&mut out, "- An `add` may carry an element with its children nested inside it; children keep their listed order.");
    line(
        &mut out,
        concat!(
            "- References inside the same proposal (an edge's `from`/`to`, a tabs `current`, `navigates_to`, ",
            "or a later operation's `id`) can only name ids the agent supplied.",
        ),
    );
    line(&mut out, "- A `ui` design's top level holds only screens and needs at least one.");
    line(&mut out, "- An `architecture` design may hold screens, flows and other elements at its top level.");
    line(&mut out, "");
    line(&mut out, "Element types");
    for (name, ty) in ELEMENT_TYPES {
        let children = if ty.children { ", may hold children" } else { "" };
        line(&mut out, format!("{name}{children}"));
        line(&mut out, format!("  required: {}", describe_element_properties(ty, true)));
        line(&mut out, format!("  optional: {}", describe_element_properties(ty, false)));
        line(&mut out, format!("  example: {}", ty.example));
        line(&mut out, "");
    }
    line(&mut out, "Proposal operations");
    line(&mut out, "A proposal is a JSON object with `operations`, and `title` and `tab` when creating a design.");
    line(
        &mut out,
        concat!(
            r#"  add:    {"op": "add", "element": <element, children nested and id optional>, "#,
            r#""parent": <element id or null>, "index": <number>}"#,
        ),
    );
    line(&mut out, r#"  remove: {"op": "remove", "id": <element id>}"#);
    line(&mut out, r#"  move:   {"op": "move", "id": <element id>, "parent": <element id or null>, "index": <number>}"#);
    line(&mut out, r#"  set:    {"op": "set", "id": <element id>, "field": "properties" | "style", "property": <name>, "value": <value>}"#);
    line(
        &mut out,
        r#"  set conditions: {"op": "set", "id": <screen id>, "field": "conditions", "index": <number>, "value": <condition>}; no index appends, an index replaces, and a null value removes that index."#,
    );
    line(
        &mut out,
        "`parent` null or absent means the top level; `index` is the position in its parent's list; a null `value` removes the property.",
    );
    out
}

fn line(out: &mut String, text: impl std::fmt::Display) {
    out.push_str(&text.to_string());
    out.push('\n');
}

fn describe_element_properties(ty: &ElementType, required: bool) -> String {
    let items: Vec<String> = ty
        .properties
        .iter()
        .filter(|(name, _)| ty.required.contains(name) == required)
        .map(|(name, kind)| format!("{name}: {}", describe_kind(kind)))
        .collect();
    if items.is_empty() { "(none)".to_owned() } else { items.join("; ") }
}

fn describe_properties(properties: &[(&str, ValueKind)]) -> String {
    properties.iter().map(|(name, kind)| format!("{name}: {}", describe_kind(kind))).collect::<Vec<_>>().join("; ")
}

fn describe_kind(kind: &ValueKind) -> String {
    match kind {
        ValueKind::OneOf(values) => values.join(", "),
        ValueKind::Spacing => format!("a spacing token ({}) or a non-negative number", SPACING_TOKENS.join(", ")),
        ValueKind::Size => "fill, hug or {\"fixed\": <positive number>}".to_owned(),
        ValueKind::Color => format!("a color token ({}) or a #rgb or #rrggbb hex color", COLOR_TOKENS.join(", ")),
        ValueKind::TextSize => format!("a text-size token ({}) or a positive number", TEXT_SIZE_TOKENS.join(", ")),
        ValueKind::Text => "any text".to_owned(),
        ValueKind::Level => "1, 2 or 3".to_owned(),
        ValueKind::Screen => "the id of a top-level screen in the same design".to_owned(),
        ValueKind::Node => "the id of a node in the same flow".to_owned(),
        ValueKind::Tab => "the id of a tab inside the same tabs element".to_owned(),
    }
}

const SPLIT_DIRECTIONS: &[&str] = &["row", "column"];

const TABS_ELEMENT: ElementType = ElementType {
    children: true,
    example: r#"{"id":"tabs","type":"tabs","properties":{"width":"fill","height":"hug","current":"one"},"children":[{"id":"one","type":"tab","properties":{"width":"hug","height":"hug","label":"One"}},{"id":"two","type":"tab","properties":{"width":"hug","height":"hug","label":"Two"}}]}"#,
    properties: &[("width", ValueKind::Size), ("height", ValueKind::Size), ("current", ValueKind::Tab)],
    required: &["width", "height", "current"],
};

const TAB: ElementType = ElementType {
    children: true,
    example: r#"{"id":"one","type":"tab","properties":{"width":"hug","height":"hug","label":"One"}}"#,
    properties: &[("width", ValueKind::Size), ("height", ValueKind::Size), ("label", ValueKind::Text)],
    required: &["width", "height", "label"],
};

const SCROLL_AREA: ElementType = ElementType {
    children: true,
    example: r#"{"id":"scroll","type":"scroll_area","properties":{"width":"fill","height":{"fixed":240}},"children":[{"id":"scrolled","type":"text","properties":{"width":"hug","height":"hug","text":"Long content"}}]}"#,
    properties: &[("width", ValueKind::Size), ("height", ValueKind::Size)],
    required: &["width", "height"],
};

const SPLIT_PANE: ElementType = ElementType {
    children: true,
    example: r#"{"id":"split","type":"split_pane","properties":{"width":"fill","height":"fill","direction":"row"},"children":[{"id":"left","type":"text","properties":{"width":"fill","height":"fill","text":"Left"}},{"id":"right","type":"text","properties":{"width":"fill","height":"fill","text":"Right"}}]}"#,
    properties: &[("width", ValueKind::Size), ("height", ValueKind::Size), ("direction", ValueKind::OneOf(SPLIT_DIRECTIONS))],
    required: &["width", "height", "direction"],
};

const COMPONENT: ElementType = ElementType {
    children: false,
    example: r#"{"id":"component","type":"component","properties":{"width":"fill","height":{"fixed":240},"name":"Task details"}}"#,
    properties: &[("width", ValueKind::Size), ("height", ValueKind::Size), ("name", ValueKind::Text)],
    required: &["width", "height", "name"],
};

/// A short account of what `operations` changed, one line per element added,
/// removed, moved or changed.
pub(crate) fn outline(before: &Value, after: &Value, operations: &[Value]) -> String {
    type Flat = (String, Option<String>, usize, String);

    fn flat(design: &Value) -> Vec<Flat> {
        fn walk(elements: &[Value], parent: Option<&str>, out: &mut Vec<Flat>) {
            for (index, element) in elements.iter().enumerate() {
                let id = element.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                let ty = element.get("type").and_then(Value::as_str).unwrap_or("").to_string();
                out.push((id.clone(), parent.map(|p| p.to_string()), index, ty));
                if let Some(children) = element.get("children").and_then(Value::as_array) {
                    walk(children, Some(&id), out);
                }
            }
        }
        let mut out = Vec::new();
        walk(design.get("elements").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]), None, &mut out);
        out
    }

    fn find<'a>(design: &'a Value, id: &str) -> Option<&'a Value> {
        fn walk<'a>(elements: &'a [Value], id: &str) -> Option<&'a Value> {
            for element in elements {
                if element.get("id").and_then(Value::as_str) == Some(id) {
                    return Some(element);
                }
                if let Some(children) = element.get("children").and_then(Value::as_array) {
                    if let Some(found) = walk(children, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        walk(design.get("elements").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]), id)
    }

    fn placement(parent: &Option<String>, index: usize) -> String {
        match parent {
            Some(parent) => format!("under {parent} at position {index}"),
            None => format!("at the top level at position {index}"),
        }
    }

    fn value_text(value: Option<&Value>) -> String {
        value.map(|value| value.to_string()).unwrap_or_else(|| "null".into())
    }

    let old = flat(before);
    let new = flat(after);
    let mut lines = Vec::new();
    for (id, parent, index, ty) in &new {
        if !old.iter().any(|(old_id, ..)| old_id == id) {
            lines.push(format!("Would add {id} ({ty}) {}", placement(parent, *index)));
        }
    }
    for (id, _, _, ty) in &old {
        if !new.iter().any(|(new_id, ..)| new_id == id) {
            lines.push(format!("Would remove {id} ({ty})"));
        }
    }
    let mut moved: Vec<&str> = Vec::new();
    for operation in operations {
        if operation.get("op").and_then(Value::as_str) == Some("move") {
            if let Some(id) = operation.get("id").and_then(Value::as_str) {
                if !moved.contains(&id) {
                    moved.push(id);
                }
            }
        }
    }
    for id in moved {
        let Some(old_element) = old.iter().find(|(element, ..)| element.as_str() == id) else { continue };
        let Some(new_element) = new.iter().find(|(element, ..)| element.as_str() == id) else { continue };
        if old_element.1 != new_element.1 || old_element.2 != new_element.2 {
            lines.push(format!("Would move {id} ({}) {}", new_element.3, placement(&new_element.1, new_element.2)));
        }
    }
    for (id, _, _, ty) in &new {
        let Some(old_element) = find(before, id) else { continue };
        let Some(new_element) = find(after, id) else { continue };
        for field in ["properties", "style"] {
            let old_field = old_element.get(field).and_then(Value::as_object);
            let new_field = new_element.get(field).and_then(Value::as_object);
            let mut keys = std::collections::BTreeSet::new();
            if let Some(object) = old_field {
                keys.extend(object.keys().map(String::as_str));
            }
            if let Some(object) = new_field {
                keys.extend(object.keys().map(String::as_str));
            }
            for key in keys {
                let old_value = old_field.and_then(|object| object.get(key));
                let new_value = new_field.and_then(|object| object.get(key));
                if old_value != new_value {
                    lines.push(format!("Would set {id} ({ty}) {field}.{key} to {}", value_text(new_value)));
                }
            }
        }
        if ty.as_str() == "screen" {
            let old_conditions = old_element.get("conditions");
            let new_conditions = new_element.get("conditions");
            if old_conditions != new_conditions {
                lines.push(format!("Would set {id} (screen) conditions to {}", value_text(new_conditions)));
            }
        }
    }
    if lines.is_empty() { "Nothing would change".into() } else { lines.join("\n") }
}

/// An id the server gave an element added without one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GivenElementId {
    pub(crate) id: String,
    pub(crate) ty: String,
    pub(crate) parent: Option<String>,
}

/// The ids `apply` gave out, each with its element's type and parent.
pub(crate) fn given_ids(given: &[GivenElementId]) -> Option<String> {
    if given.is_empty() {
        return None;
    }
    let listed: Vec<String> = given
        .iter()
        .map(|given| match &given.parent {
            Some(parent) => format!("{} ({}, under {parent})", given.id, given.ty),
            None => format!("{} ({}, at the top level)", given.id, given.ty),
        })
        .collect();
    Some(format!("Given ids: {}", listed.join(", ")))
}

/// Fills in an id for `element` and each child that has none, recording each one.
fn give_ids(element: &mut Value, parent: Option<&str>, used: &mut HashSet<String>, given: &mut Vec<GivenElementId>) {
    fn free_id(ty: &str, used: &mut HashSet<String>) -> String {
        let mut number = 1;
        loop {
            let id = format!("{ty}-{number}");
            if used.insert(id.clone()) {
                return id;
            }
            number += 1;
        }
    }

    let Some(object) = element.as_object_mut() else { return };
    let id = match object.get("id") {
        Some(Value::String(id)) if !id.is_empty() => id.clone(),
        Some(_) => return,
        None => {
            let ty = object.get("type").and_then(Value::as_str).unwrap_or("element").to_string();
            let id = free_id(&ty, used);
            object.insert("id".into(), Value::String(id.clone()));
            given.push(GivenElementId { id: id.clone(), ty, parent: parent.map(str::to_owned) });
            id
        }
    };
    if let Some(children) = object.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            give_ids(child, Some(&id), used, given);
        }
    }
}

const CONDITION_KEYS: &[&str] = &["when", "changes"];

const WHEN_KEYS: &[&str] = &["ratio", "width", "height"];

const BOUND_KEYS: &[&str] = &["at_least", "at_most"];

const CHANGE_KEYS: &[&str] = &["id", "direction", "property", "value"];

fn validate_conditions(screen: &Value, at: &str, screens: &HashSet<String>) -> Result<(), Failure> {
    let Some(conditions) = screen.get("conditions") else {
        return Ok(());
    };
    let conditions = conditions.as_array().ok_or_else(|| invalid(format!("{at} conditions must be a list")))?;
    for (index, condition) in conditions.iter().enumerate() {
        let at = format!("{at} condition {index}");
        let object = condition.as_object().ok_or_else(|| invalid(format!("{at} must be an object")))?;
        unknown_keys(object, CONDITION_KEYS, &at)?;
        let when = object.get("when").and_then(Value::as_object).ok_or_else(|| invalid(format!("{at} needs a when object")))?;
        let when_at = format!("{at} when");
        unknown_keys(when, WHEN_KEYS, &when_at)?;
        let subjects: Vec<&str> = WHEN_KEYS.iter().copied().filter(|name| when.contains_key(*name)).collect();
        if subjects.len() != 1 {
            return Err(invalid(format!("{when_at} names exactly one of ratio, width and height, not {}", subjects.len())));
        }
        let subject = subjects[0];
        let bound = when[subject].as_object().ok_or_else(|| invalid(format!("{when_at} {subject} must be an object")))?;
        let bound_at = format!("{when_at} {subject}");
        unknown_keys(bound, BOUND_KEYS, &bound_at)?;
        let limits: Vec<&str> = BOUND_KEYS.iter().copied().filter(|name| bound.contains_key(*name)).collect();
        if limits.len() != 1 {
            return Err(invalid(format!("{bound_at} names exactly one of at_least and at_most, not {}", limits.len())));
        }
        if !bound[limits[0]].as_f64().is_some_and(|number| number.is_finite() && number > 0.0) {
            return Err(invalid(format!("{bound_at} {} must be a positive number", limits[0])));
        }
        let changes = object.get("changes").and_then(Value::as_array).ok_or_else(|| invalid(format!("{at} needs a changes list")))?;
        if changes.is_empty() {
            return Err(invalid(format!("{at} changes nothing")));
        }
        for (change_index, change) in changes.iter().enumerate() {
            validate_condition_change(screen, screens, &format!("{at} change {change_index}"), change)?;
        }
    }
    Ok(())
}

fn validate_condition_change(screen: &Value, screens: &HashSet<String>, at: &str, change: &Value) -> Result<(), Failure> {
    let object = change.as_object().ok_or_else(|| invalid(format!("{at} must be an object")))?;
    unknown_keys(object, CHANGE_KEYS, at)?;
    let id = text(object, "id", at)?;
    let target = condition_target(screen, id).ok_or_else(|| invalid(format!("{at} names no element {id:?} in this screen")))?;
    let ty = target.get("type").and_then(Value::as_str).unwrap_or_default();
    let Some(kind) = ELEMENT_TYPES.iter().find_map(|(name, kind)| (*name == ty).then_some(kind)) else {
        return Err(invalid(format!("{at} names element {id:?} with unknown type {ty:?}")));
    };
    match (object.contains_key("direction"), object.contains_key("property")) {
        (true, false) => {
            if !matches!(ty, "row" | "column") {
                return Err(invalid(format!(
                    "{at} changes the direction of element {id:?} of type {ty:?}; only a row or column can switch"
                )));
            }
            let direction = object.get("direction").and_then(Value::as_str).unwrap_or_default();
            if !matches!(direction, "row" | "column") {
                return Err(invalid(format!("{at} gives element {id:?} direction {direction:?}; expected row or column")));
            }
        }
        (false, true) => {
            let property = object.get("property").and_then(Value::as_str).unwrap_or_default();
            let Some((_, value_kind)) = kind.properties.iter().find(|(name, _)| *name == property) else {
                return Err(invalid(format!("{at} changes property {property:?}, which element {id:?} of type {ty:?} does not accept")));
            };
            let value =
                object.get("value").ok_or_else(|| invalid(format!("{at} needs a value for element {id:?} property {property:?}")))?;
            value_kind.check(value).map_err(|why| invalid(format!("{at} gives element {id:?} property {property:?} {why}")))?;
            let children = target.get("children").and_then(Value::as_array).map(Vec::as_slice);
            check_reference(value_kind, value, screens, children, &format!("{at} of element {id:?}"), property)?;
        }
        _ => {
            return Err(invalid(format!("{at} must give element {id:?} either a direction or a property and value")));
        }
    }
    Ok(())
}

fn condition_target<'a>(element: &'a Value, id: &str) -> Option<&'a Value> {
    if element.get("id").and_then(Value::as_str) == Some(id) {
        return Some(element);
    }
    element.get("children").and_then(Value::as_array)?.iter().find_map(|child| condition_target(child, id))
}

fn set_condition(element: &mut Value, op: &Map<String, Value>) -> Result<(), Failure> {
    let value = op.get("value").ok_or_else(|| operation_error("set needs a `value`"))?;
    let index = match op.get("index") {
        None => None,
        Some(index) => Some(index.as_u64().ok_or_else(|| operation_error("set conditions `index` must be a whole number"))? as usize),
    };
    let object = element.as_object_mut().ok_or_else(|| invalid("the element is not an object"))?;
    let conditions = object.entry("conditions").or_insert_with(|| Value::Array(Vec::new()));
    let conditions = conditions.as_array_mut().ok_or_else(|| invalid("element conditions must be a list"))?;
    if value.is_null() {
        let index = index.ok_or_else(|| operation_error("set conditions with a null value needs an `index`"))?;
        if index >= conditions.len() {
            return Err(operation_error(format!("set conditions index {index} is outside the {} conditions", conditions.len())));
        }
        conditions.remove(index);
    } else {
        if index.is_some_and(|index| index > conditions.len()) {
            return Err(operation_error(format!("set conditions index is outside the {} conditions", conditions.len())));
        }
        match index {
            Some(index) if index < conditions.len() => conditions[index] = value.clone(),
            _ => conditions.push(value.clone()),
        }
    }
    Ok(())
}

/// Checks that a reference property names a target that exists: a screen in
/// the design, or a tab among `children`.
fn check_reference(
    kind: &ValueKind,
    value: &Value,
    screens: &HashSet<String>,
    children: Option<&[Value]>,
    at: &str,
    property: &str,
) -> Result<(), Failure> {
    let name = value.as_str().unwrap_or_default();
    let found = match kind {
        ValueKind::Screen => screens.contains(name),
        ValueKind::Tab => children.is_some_and(|children| {
            children.iter().any(|child| {
                child.get("type").and_then(Value::as_str) == Some("tab") && child.get("id").and_then(Value::as_str) == Some(name)
            })
        }),
        _ => return Ok(()),
    };
    if found {
        return Ok(());
    }
    let target = if matches!(kind, ValueKind::Screen) { "screen" } else { "tab" };
    Err(invalid(format!("{at} property {property:?} names no {target} {name:?}")))
}
