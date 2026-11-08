use super::*;
use serde_json::json;

fn design() -> Value {
    json!({
        "schemaVersion": SCHEMA_VERSION,
        "id": "0123456789abcdef",
        "title": "Home",
        "tab": "ui",
        "elements": [{
            "id": "root",
            "type": "screen",
            "properties": {"width": "fill", "height": "fill", "gap": 8, "padding": 16, "align": "start", "justify": "center"},
            "children": [{
                "id": "heading",
                "type": "text",
                "properties": {"align": "left", "width": "hug", "height": "hug", "text": "Home"},
            }]
        }]
    })
}

fn refusal(design: &Value, names: &[&str]) -> String {
    let e = validate(design).unwrap_err().0;
    assert!(e.starts_with("E7033"), "{e}");
    for name in names {
        assert!(e.contains(name), "{name:?} not in {e:?}");
    }
    e
}

#[test]
fn a_valid_design_passes() {
    validate(&design()).unwrap();
    let mut d = design();
    d["elements"].as_array_mut().unwrap().push(json!({"id": "side", "type": "screen", "properties": {"width": "fill", "height": "fill"}}));
    validate(&d).unwrap();
}

#[test]
fn a_design_must_be_an_object() {
    refusal(&json!([]), &["JSON object"]);
}

#[test]
fn an_unknown_schema_version_is_refused() {
    let mut d = design();
    d["schemaVersion"] = json!(2);
    refusal(&d, &["unknown schema version", "1"]);
    d["extra"] = json!(true);
    let e = refusal(&d, &["unknown schema version"]);
    assert!(!e.contains("unknown key"), "{e}");
    d.as_object_mut().unwrap().remove("schemaVersion");
    refusal(&d, &["unknown schema version"]);
}

#[test]
fn a_design_id_must_be_sixteen_lowercase_hex_digits() {
    for id in [json!("abc"), json!("0123456789ABCDEF"), json!("../../0123456789")] {
        let mut d = design();
        d["id"] = id;
        refusal(&d, &["design id", "16 lowercase hex digits"]);
    }
    let mut d = design();
    d.as_object_mut().unwrap().remove("id");
    refusal(&d, &["design", "id"]);
}

#[test]
fn a_design_needs_a_text_title_and_a_known_tab() {
    let mut d = design();
    d["title"] = json!(3);
    refusal(&d, &["design", "title"]);
    d = design();
    d["tab"] = json!("board");
    refusal(&d, &["unknown tab", "board", "architecture", "ui"]);
    d = design();
    d.as_object_mut().unwrap().remove("tab");
    refusal(&d, &["design", "tab"]);
}

#[test]
fn a_design_needs_a_list_of_elements() {
    let mut d = design();
    d["elements"] = json!({"id": "root"});
    refusal(&d, &["list of elements"]);
    d = design();
    d["elements"] = json!([1]);
    refusal(&d, &["element", "JSON object"]);
}

#[test]
fn an_unknown_design_key_is_refused() {
    let mut d = design();
    d["extra"] = json!(1);
    refusal(&d, &["design", "unknown key", "extra"]);
}

#[test]
fn a_repeated_element_id_is_refused() {
    let mut d = design();
    d["elements"].as_array_mut().unwrap().push(json!({"id": "heading", "type": "text", "properties": {}}));
    refusal(&d, &["heading", "repeated id"]);
}

#[test]
fn an_unknown_element_type_is_refused() {
    let mut d = design();
    d["elements"][0]["children"][0]["type"] = json!("canvas");
    refusal(&d, &["heading", "unknown type", "canvas"]);
}

#[test]
fn an_element_needs_a_text_id_and_type() {
    let mut d = design();
    d["elements"][0]["children"][0].as_object_mut().unwrap().remove("id");
    refusal(&d, &["element", "id"]);
    d = design();
    d["elements"][0]["children"][0]["id"] = json!(7);
    refusal(&d, &["element", "id"]);
    d = design();
    d["elements"][0]["children"][0].as_object_mut().unwrap().remove("type");
    refusal(&d, &["heading", "type"]);
}

#[test]
fn element_properties_must_be_an_object_of_known_properties() {
    let mut d = design();
    d["elements"][0]["children"][0]["properties"] = json!([]);
    refusal(&d, &["heading", "properties must be an object"]);
    d = design();
    d["elements"][0]["children"][0]["properties"] = json!({"weight": 1});
    refusal(&d, &["heading", "unknown property", "weight"]);
}

#[test]
fn a_property_must_hold_a_value_in_its_closed_list() {
    let mut d = design();
    d["elements"][0]["children"][0]["properties"]["align"] = json!("up");
    refusal(&d, &["heading", "align", "left", "center", "right"]);
    d = design();
    d["elements"][0]["children"][0]["properties"]["align"] = json!(2);
    refusal(&d, &["heading", "align", "must be one of"]);
}

#[test]
fn an_unknown_element_key_is_refused() {
    let mut d = design();
    d["elements"][0]["children"][0]["position"] = json!({"x": 1});
    refusal(&d, &["heading", "unknown key", "position"]);
}

#[test]
fn a_type_that_cannot_hold_children_is_refused() {
    let mut d = design();
    d["elements"][0]["children"][0]["children"] = json!([]);
    refusal(&d, &["heading", "text", "can't hold children"]);
}

#[test]
fn children_must_be_a_list() {
    let mut d = design();
    d["elements"][0]["children"] = json!("one");
    refusal(&d, &["root", "children must be a list"]);
}

#[test]
fn a_new_design_saves_its_first_version() {
    let top = crate::testing::Scratch::new();
    save(&top, &design(), Source::Human).unwrap();
    assert_eq!(show(&top, "0123456789abcdef").unwrap(), design());
    let version = crate::jsonx::read(&versions_dir(&top, "0123456789abcdef").join("1.json")).unwrap();
    assert_eq!(version["number"], json!(1));
    assert_eq!(version["design"], design());
    assert!(version["time"].as_u64().is_some(), "{version}");
    assert_eq!(version["source"], json!({"kind": "human"}));
}

#[test]
fn a_change_saves_the_next_version_and_keeps_the_old() {
    let top = crate::testing::Scratch::new();
    save(&top, &design(), Source::Human).unwrap();
    let mut changed = design();
    changed["title"] = json!("Home again");
    save(&top, &changed, Source::DesignRun("0123456789abcdef")).unwrap();
    let dir = versions_dir(&top, "0123456789abcdef");
    let first = crate::jsonx::read(&dir.join("1.json")).unwrap();
    let second = crate::jsonx::read(&dir.join("2.json")).unwrap();
    assert_eq!(first["design"], design());
    assert_eq!(second["design"], changed);
    assert_eq!(second["number"], json!(2));
    assert_eq!(second["source"], json!({"kind": "run", "id": "0123456789abcdef"}));
    assert_eq!(show(&top, "0123456789abcdef").unwrap(), changed);
}

#[test]
fn an_invalid_design_changes_nothing_on_disk() {
    let top = crate::testing::Scratch::new();
    let mut invalid = design();
    invalid["tab"] = json!("board");
    assert!(save(&top, &invalid, Source::Human).unwrap_err().0.starts_with("E7033"));
    assert!(!top.join("designs").exists(), "an invalid first save writes nothing");
    save(&top, &design(), Source::Human).unwrap();
    let path = top.join("designs/0123456789abcdef.json");
    let before = std::fs::read(&path).unwrap();
    assert!(save(&top, &invalid, Source::Human).unwrap_err().0.starts_with("E7033"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read_dir(versions_dir(&top, "0123456789abcdef")).unwrap().count(), 1);
}

#[test]
fn an_identical_save_adds_no_version() {
    let top = crate::testing::Scratch::new();
    save(&top, &design(), Source::Human).unwrap();
    save(&top, &design(), Source::DesignRun("0123456789abcdef")).unwrap();
    let dir = versions_dir(&top, "0123456789abcdef");
    assert_eq!(std::fs::read_dir(dir).unwrap().count(), 1);
}

#[test]
fn simultaneous_saves_keep_every_version() {
    let top = crate::testing::Scratch::new();
    const SAVES: u64 = 8;
    std::thread::scope(|scope| {
        for i in 0..SAVES {
            let top = &top;
            scope.spawn(move || {
                let mut changed = design();
                changed["title"] = json!(format!("Home {i}"));
                save(top, &changed, Source::Human).unwrap();
            });
        }
    });
    let dir = versions_dir(&top, "0123456789abcdef");
    let mut titles = std::collections::HashSet::new();
    for number in 1..=SAVES {
        let version = crate::jsonx::read(&dir.join(format!("{number}.json"))).unwrap();
        assert_eq!(version["number"], json!(number));
        assert_eq!(version["design"]["id"], json!("0123456789abcdef"));
        assert!(titles.insert(version["design"]["title"].as_str().unwrap().to_owned()));
    }
    assert_eq!(titles.len(), SAVES as usize);
}

#[test]
fn a_pre_existing_design_is_kept_before_the_change() {
    let top = crate::testing::Scratch::new();
    let path = top.join(DESIGNS_DIR).join("0123456789abcdef.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, design().to_string()).unwrap();
    let mut changed = design();
    changed["title"] = json!("Home again");
    save(&top, &changed, Source::Human).unwrap();
    let dir = versions_dir(&top, "0123456789abcdef");
    let first = crate::jsonx::read(&dir.join("1.json")).unwrap();
    let second = crate::jsonx::read(&dir.join("2.json")).unwrap();
    assert_eq!(first["design"], design());
    assert_eq!(first["source"], json!({"kind": "repo"}));
    assert_eq!(second["number"], json!(2));
    assert_eq!(second["design"], changed);
    assert_eq!(show(&top, "0123456789abcdef").unwrap(), changed);
}

#[test]
fn a_screen_may_nest_rows_and_columns() {
    let mut d = design();
    d["elements"][0]["children"] = json!([
        {
            "id": "toolbar",
            "type": "row",
            "properties": {"width": "fill", "height": "hug", "gap": 8, "padding": 4, "align": "center", "justify": "start"},
            "children": [{"id": "label", "type": "text", "properties": {"align": "left", "width": "hug", "height": "hug", "text": "Home"}}]
        },
        {
            "id": "body",
            "type": "column",
            "properties": {"width": "fill", "height": "fill", "gap": 12, "padding": 0, "align": "stretch", "justify": "between"},
            "children": []
        }
    ]);
    validate(&d).unwrap();
}

#[test]
fn an_unknown_align_value_is_refused() {
    let mut d = design();
    d["elements"][0]["properties"]["align"] = json!("middle");
    refusal(&d, &["root", "align", "middle"]);
}

#[test]
fn a_fixed_size_needs_a_positive_number() {
    for size in [json!({"fixed": null}), json!({"fixed": "wide"}), json!({"fixed": 0}), json!({"fixed": -5}), json!(120)] {
        let mut d = design();
        d["elements"][0]["properties"]["width"] = size;
        refusal(&d, &["root", "width"]);
    }
    let mut d = design();
    d["elements"][0]["properties"]["width"] = json!({"fixed": 120});
    validate(&d).unwrap();
}

#[test]
fn every_element_needs_a_width_and_height() {
    for axis in ["width", "height"] {
        let mut d = design();
        d["elements"][0]["properties"].as_object_mut().unwrap().remove(axis);
        refusal(&d, &["root", axis]);
    }
    let mut d = design();
    d["elements"][0]["children"][0]["properties"].as_object_mut().unwrap().remove("width");
    refusal(&d, &["heading", "width"]);
}

#[test]
fn gap_and_padding_are_non_negative_numbers() {
    for name in ["gap", "padding"] {
        for value in [json!(-1), json!(null), json!(true)] {
            let mut d = design();
            d["elements"][0]["properties"][name] = value;
            refusal(&d, &["root", name]);
        }
        for token in SPACING_TOKENS {
            let mut d = design();
            d["elements"][0]["properties"][name] = json!(token);
            validate(&d).unwrap();
        }
        let mut d = design();
        d["elements"][0]["properties"][name] = json!(0);
        validate(&d).unwrap();
    }
}

#[test]
fn an_element_has_no_position() {
    let mut d = design();
    d["elements"][0]["children"][0]["position"] = json!({"x": 1});
    refusal(&d, &["heading", "unknown key", "position"]);
    d = design();
    d["elements"][0]["children"][0]["properties"]["position"] = json!({"x": 1});
    refusal(&d, &["heading", "unknown property", "position"]);
}

#[test]
fn a_ui_design_needs_a_screen_at_the_top() {
    let mut d = design();
    d["elements"] = json!([]);
    refusal(&d, &["ui", "at least one screen"]);
    d = design();
    d["elements"] =
        json!([{"id": "loose", "type": "text", "properties": {"align": "left", "width": "hug", "height": "hug", "text": "Home"}}]);
    refusal(&d, &["loose", "top level", "must be a screen"]);
    d = design();
    d["elements"] = json!([{"id": "bar", "type": "row", "properties": {"width": "fill", "height": "fill"}}]);
    refusal(&d, &["bar", "top level", "must be a screen"]);
}

#[test]
fn a_screen_cannot_be_nested() {
    let mut d = design();
    d["elements"][0]["children"][0] = json!({"id": "inner", "type": "screen", "properties": {"width": "fill", "height": "fill"}});
    refusal(&d, &["inner", "screen", "below the top level"]);
}

#[test]
fn architecture_designs_keep_their_own_top_level() {
    let mut d = design();
    d["tab"] = json!("architecture");
    d["elements"] =
        json!([{"id": "loose", "type": "text", "properties": {"align": "left", "width": "hug", "height": "hug", "text": "Home"}}]);
    validate(&d).unwrap();
}

#[test]
fn an_empty_element_id_or_title_is_refused() {
    let mut d = design();
    d["title"] = json!("");
    refusal(&d, &["design", "title"]);
    d = design();
    d["elements"][0]["children"][0]["id"] = json!("");
    refusal(&d, &["element", "id"]);
}

#[test]
fn an_element_id_with_a_newline_keeps_its_refusal_on_one_line() {
    let mut d = design();
    d["elements"][0]["children"][0]["id"] = json!("head\ning");
    d["elements"][0]["children"][0]["type"] = json!("unknown");
    let e = refusal(&d, &["head\\ning"]);
    assert_eq!(e.lines().count(), 1, "{e:?}");
}

#[test]
fn a_file_named_designs_lists_no_designs() {
    let top = crate::testing::Scratch::new();
    std::fs::write(top.join(DESIGNS_DIR), b"not a folder").unwrap();
    assert_eq!(list(&top).unwrap(), json!({"designs": [], "invalid": []}));
}

#[test]
fn a_non_utf8_design_name_is_listed_as_invalid() {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let top = crate::testing::Scratch::new();
        let dir = top.join(DESIGNS_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        let mut name = b"design".to_vec();
        name.push(0xff);
        name.extend_from_slice(b".json");
        std::fs::write(dir.join(std::ffi::OsString::from_vec(name)), design().to_string()).unwrap();
        let listed = list(&top).unwrap();
        assert_eq!(listed["invalid"].as_array().unwrap().len(), 1, "{listed}");
    }
}

#[test]
fn a_damaged_current_file_is_kept_before_the_change() {
    let top = crate::testing::Scratch::new();
    let path = top.join(DESIGNS_DIR).join("0123456789abcdef.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"<<<<<<< conflict").unwrap();
    save(&top, &design(), Source::Human).unwrap();
    let first = crate::jsonx::read(&versions_dir(&top, "0123456789abcdef").join("1.json")).unwrap();
    assert_eq!(first["source"], json!({"kind": "repo"}));
    assert_eq!(first["raw"], json!("<<<<<<< conflict"));
    assert!(first.get("design").is_none(), "{first}");
    assert_eq!(show(&top, "0123456789abcdef").unwrap(), design());
}

#[test]
fn a_retry_does_not_save_the_same_damaged_text_twice() {
    let top = crate::testing::Scratch::new();
    let path = top.join(DESIGNS_DIR).join("0123456789abcdef.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"<<<<<<< conflict").unwrap();
    let dir = versions_dir(&top, "0123456789abcdef");
    std::fs::create_dir_all(&dir).unwrap();
    let raw = json!({"number": 1, "time": 0, "source": {"kind": "repo"}, "raw": "<<<<<<< conflict"});
    std::fs::write(dir.join("1.json"), raw.to_string()).unwrap();
    save(&top, &design(), Source::Human).unwrap();
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    let second = crate::jsonx::read(&dir.join("2.json")).unwrap();
    assert_eq!(second["design"], design());
}

#[test]
fn an_unreadable_current_file_refuses_the_save() {
    let top = crate::testing::Scratch::new();
    let path = top.join(DESIGNS_DIR).join("0123456789abcdef.json");
    std::fs::create_dir_all(&path).unwrap();
    assert!(save(&top, &design(), Source::Human).is_err());
    assert!(!versions_dir(&top, "0123456789abcdef").exists());
}

#[test]
fn a_retry_after_the_design_is_saved_keeps_the_raw_version_once() {
    let top = crate::testing::Scratch::new();
    let path = top.join(DESIGNS_DIR).join("0123456789abcdef.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"<<<<<<< conflict").unwrap();
    let dir = versions_dir(&top, "0123456789abcdef");
    std::fs::create_dir_all(&dir).unwrap();
    let raw = json!({"number": 1, "time": 0, "source": {"kind": "repo"}, "raw": "<<<<<<< conflict"});
    std::fs::write(dir.join("1.json"), raw.to_string()).unwrap();
    let saved = json!({"number": 2, "time": 0, "source": {"kind": "human"}, "design": design()});
    std::fs::write(dir.join("2.json"), saved.to_string()).unwrap();
    save(&top, &design(), Source::Human).unwrap();
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    assert_eq!(show(&top, "0123456789abcdef").unwrap(), design());
}

#[test]
fn a_reverted_file_saves_the_design_again() {
    let top = crate::testing::Scratch::new();
    let first = design();
    let mut second = design();
    second["title"] = json!("Home again");
    save(&top, &first, Source::Human).unwrap();
    save(&top, &second, Source::Human).unwrap();
    let path = top.join(DESIGNS_DIR).join("0123456789abcdef.json");
    std::fs::write(&path, first.to_string()).unwrap();
    save(&top, &second, Source::Human).unwrap();
    let dir = versions_dir(&top, "0123456789abcdef");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 4);
    let fourth = crate::jsonx::read(&dir.join("4.json")).unwrap();
    assert_eq!(fourth["design"], second);
    assert_eq!(fourth["source"], json!({"kind": "human"}));
    assert_eq!(show(&top, "0123456789abcdef").unwrap(), second);
}

#[test]
fn a_damaged_file_saves_the_design_after_its_raw_version() {
    let top = crate::testing::Scratch::new();
    save(&top, &design(), Source::Human).unwrap();
    let path = top.join(DESIGNS_DIR).join("0123456789abcdef.json");
    std::fs::write(&path, b"<<<<<<< conflict").unwrap();
    save(&top, &design(), Source::Human).unwrap();
    let dir = versions_dir(&top, "0123456789abcdef");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3);
    let third = crate::jsonx::read(&dir.join("3.json")).unwrap();
    assert_eq!(third["design"], design());
    assert_eq!(third["source"], json!({"kind": "human"}));
    assert_eq!(show(&top, "0123456789abcdef").unwrap(), design());
}

#[test]
fn a_design_change_is_announced_with_its_list() {
    let top = crate::testing::Scratch::new();
    let mut seen = None;
    announce_change(&top, &mut seen);
    announce_change(&top, &mut seen);
    let last = || -> Value {
        let data: String =
            crate::tasks::events(&top).unwrap().query_row("SELECT data FROM events ORDER BY seq DESC LIMIT 1", [], |r| r.get(0)).unwrap();
        serde_json::from_str(&data).unwrap()
    };
    assert_eq!(last(), json!({"designs": []}));
    save(&top, &design(), Source::Human).unwrap();
    announce_change(&top, &mut seen);
    assert_eq!(last(), json!({"designs": [{"id": "0123456789abcdef", "title": "Home", "tab": "ui"}]}));
    announce_change(&top, &mut seen);
    let count: i64 = crate::tasks::events(&top).unwrap().query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 2, "an unchanged list is not announced again");
}

#[test]
fn damaged_non_utf8_bytes_are_kept_exactly() {
    let top = crate::testing::Scratch::new();
    let path = top.join(DESIGNS_DIR).join("0123456789abcdef.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"\xff\xfeA").unwrap();
    save(&top, &design(), Source::Human).unwrap();
    let first = crate::jsonx::read(&versions_dir(&top, "0123456789abcdef").join("1.json")).unwrap();
    assert_eq!(first["source"], json!({"kind": "repo"}));
    assert!(first.get("raw").is_none(), "{first}");
    assert_eq!(first["rawBase64"], json!("//5B"));
    assert_eq!(raw_bytes(&first), Some(b"\xff\xfeA".to_vec()));
}

#[test]
fn a_retry_does_not_save_the_same_damaged_bytes_twice() {
    let top = crate::testing::Scratch::new();
    let path = top.join(DESIGNS_DIR).join("0123456789abcdef.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"\xff\xfeA").unwrap();
    let dir = versions_dir(&top, "0123456789abcdef");
    std::fs::create_dir_all(&dir).unwrap();
    let raw = json!({"number": 1, "time": 0, "source": {"kind": "repo"}, "rawBase64": "//5B"});
    std::fs::write(dir.join("1.json"), raw.to_string()).unwrap();
    std::fs::write(dir.join("2.json"), json!({"number": 2, "time": 0, "source": {"kind": "human"}, "design": design()}).to_string())
        .unwrap();
    save(&top, &design(), Source::Human).unwrap();
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
}

#[test]
fn versions_list_newest_first_without_content() {
    let top = crate::testing::Scratch::new();
    let mut second = design();
    second["title"] = json!("Second");
    let mut third = design();
    third["title"] = json!("Third");
    save(&top, &design(), Source::Human).unwrap();
    save(&top, &second, Source::DesignRun("0123456789abcdef")).unwrap();
    save(&top, &third, Source::Human).unwrap();
    let listed = versions(&top, "0123456789abcdef").unwrap();
    let numbers: Vec<_> = listed.as_array().unwrap().iter().map(|v| v["number"].clone()).collect();
    assert_eq!(numbers, vec![json!(3), json!(2), json!(1)]);
    assert_eq!(listed[0]["source"], json!({"kind": "human"}));
    assert_eq!(listed[1]["source"], json!({"kind": "run", "id": "0123456789abcdef"}));
    assert!(listed[0]["time"].as_u64().is_some(), "{listed}");
    assert!(listed[0].get("design").is_none(), "{listed}");
}

#[test]
fn a_design_with_a_file_but_no_history_lists_no_versions() {
    let top = crate::testing::Scratch::new();
    std::fs::create_dir_all(top.join(DESIGNS_DIR)).unwrap();
    std::fs::write(top.join(DESIGNS_DIR).join("0123456789abcdef.json"), design().to_string()).unwrap();
    assert_eq!(versions(&top, "0123456789abcdef").unwrap(), json!([]));
    let missing = version(&top, "0123456789abcdef", 1).unwrap_err();
    assert!(missing.0.starts_with("E7023"), "{}", missing.0);
}

#[test]
fn an_unknown_design_has_no_versions() {
    let top = crate::testing::Scratch::new();
    let missing = versions(&top, "0123456789abcdef").unwrap_err();
    assert!(missing.0.starts_with("E7023"), "{}", missing.0);
    let malformed = versions(&top, "not-an-id").unwrap_err();
    assert!(malformed.0.starts_with("E7023"), "{}", malformed.0);
    let zero = version(&top, "0123456789abcdef", 0).unwrap_err();
    assert!(zero.0.starts_with("E7023"), "{}", zero.0);
}

#[test]
fn a_restore_keeps_later_versions_and_adds_the_old_content() {
    let top = crate::testing::Scratch::new();
    let mut second = design();
    second["title"] = json!("Second");
    let mut third = design();
    third["title"] = json!("Third");
    save(&top, &design(), Source::Human).unwrap();
    save(&top, &second, Source::Human).unwrap();
    save(&top, &third, Source::Human).unwrap();
    assert_eq!(versions(&top, "0123456789abcdef").unwrap().as_array().unwrap().len(), 3);
    let listed = restore(&top, "0123456789abcdef", 1).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 4);
    assert_eq!(listed[0]["number"], json!(4));
    assert_eq!(show(&top, "0123456789abcdef").unwrap(), design());
    assert_eq!(version(&top, "0123456789abcdef", 2).unwrap()["design"], second);
    assert_eq!(version(&top, "0123456789abcdef", 3).unwrap()["design"], third);
}

#[test]
fn restoring_the_current_design_adds_no_version() {
    let top = crate::testing::Scratch::new();
    save(&top, &design(), Source::Human).unwrap();
    let listed = restore(&top, "0123456789abcdef", 1).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
}

#[test]
fn an_unknown_version_is_refused_and_changes_nothing() {
    let top = crate::testing::Scratch::new();
    save(&top, &design(), Source::Human).unwrap();
    let missing = restore(&top, "0123456789abcdef", 9).unwrap_err();
    assert!(missing.0.starts_with("E7023"), "{}", missing.0);
    assert_eq!(show(&top, "0123456789abcdef").unwrap(), design());
    assert_eq!(versions(&top, "0123456789abcdef").unwrap().as_array().unwrap().len(), 1);
}

#[test]
fn a_version_of_another_design_is_refused() {
    let top = crate::testing::Scratch::new();
    save(&top, &design(), Source::Human).unwrap();
    let dir = versions_dir(&top, "0123456789abcdef");
    let mut other = design();
    other["id"] = json!("fedcba9876543210");
    let version = json!({"number": 2, "time": 1, "source": {"kind": "human"}, "design": other});
    std::fs::write(dir.join("2.json"), version.to_string()).unwrap();
    let refused = restore(&top, "0123456789abcdef", 2).unwrap_err();
    assert!(refused.0.starts_with("E7033"), "{}", refused.0);
    assert_eq!(show(&top, "0123456789abcdef").unwrap(), design());
    assert!(!dir.join("3.json").exists());
}

#[test]
fn a_damaged_version_is_refused() {
    let top = crate::testing::Scratch::new();
    save(&top, &design(), Source::Human).unwrap();
    let dir = versions_dir(&top, "0123456789abcdef");
    append_raw(&dir, 2, b"<<<<<<< conflict").unwrap();
    let refused = restore(&top, "0123456789abcdef", 2).unwrap_err();
    assert!(refused.0.starts_with("E7033"), "{}", refused.0);
    assert!(refused.0.contains("no design"), "{}", refused.0);
    assert_eq!(show(&top, "0123456789abcdef").unwrap(), design());
}

#[test]
fn a_version_that_fails_validation_is_refused() {
    let top = crate::testing::Scratch::new();
    save(&top, &design(), Source::Human).unwrap();
    let dir = versions_dir(&top, "0123456789abcdef");
    let mut broken = design();
    broken["tab"] = json!("sideways");
    let version = json!({"number": 2, "time": 1, "source": {"kind": "human"}, "design": broken});
    std::fs::write(dir.join("2.json"), version.to_string()).unwrap();
    let refused = restore(&top, "0123456789abcdef", 2).unwrap_err();
    assert!(refused.0.starts_with("E7033"), "{}", refused.0);
    assert_eq!(show(&top, "0123456789abcdef").unwrap(), design());
    assert!(!dir.join("3.json").exists());
}

#[test]
fn a_version_number_must_be_a_whole_number() {
    assert_eq!(number(&json!(2)).unwrap(), 2);
    for bad in [json!(null), json!("3"), json!("two"), json!(-1), json!(1.5), json!({})] {
        let refused = number(&bad).unwrap_err();
        assert!(refused.0.starts_with("E7001"), "{bad}: {}", refused.0);
    }
}

#[test]
fn a_corrupted_version_file_is_skipped_and_the_others_are_listed() {
    let top = crate::testing::Scratch::new();
    save(&top, &design(), Source::Human).unwrap();
    let dir = versions_dir(&top, "0123456789abcdef");
    std::fs::write(dir.join("2.json"), "{").unwrap();
    let listed = versions(&top, "0123456789abcdef").unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed[0]["number"], json!(1), "{listed}");
}

#[test]
fn version_listing_ignores_other_files() {
    let top = crate::testing::Scratch::new();
    save(&top, &design(), Source::Human).unwrap();
    let dir = versions_dir(&top, "0123456789abcdef");
    std::fs::write(dir.join("draft.tmp"), "{}").unwrap();
    std::fs::write(dir.join("latest.json"), "{}").unwrap();
    assert_eq!(versions(&top, "0123456789abcdef").unwrap().as_array().unwrap().len(), 1);
}

fn default_properties(ty: &str) -> Value {
    match ty {
        "text" => json!({"width": "hug", "height": "hug", "text": "Body"}),
        "heading" => json!({"width": "hug", "height": "hug", "text": "Title", "level": 2}),
        "button" => json!({"width": "hug", "height": "hug", "label": "Save"}),
        "text_input" => json!({"width": "hug", "height": "hug", "placeholder": "Name"}),
        "image_placeholder" => json!({"width": "hug", "height": "hug", "label": "Photo"}),
        _ => json!({}),
    }
}

#[test]
fn every_default_component_validates() {
    for ty in ["text", "heading", "button", "text_input", "image_placeholder"] {
        let mut d = design();
        d["elements"][0]["children"] = json!([{
            "id": "part",
            "type": ty,
            "properties": default_properties(ty),
        }]);
        validate(&d).unwrap();
    }
}

#[test]
fn an_unknown_property_on_a_default_component_is_refused() {
    let mut d = design();
    d["elements"][0]["children"] = json!([{
        "id": "save",
        "type": "button",
        "properties": {"width": "hug", "height": "hug", "label": "Save", "weight": 1},
    }]);
    refusal(&d, &["save", "unknown property", "weight"]);
}

#[test]
fn default_components_cannot_hold_children() {
    for ty in ["text", "heading", "button", "text_input", "image_placeholder"] {
        let mut d = design();
        d["elements"][0]["children"] = json!([{
            "id": "part",
            "type": ty,
            "properties": default_properties(ty),
            "children": [],
        }]);
        refusal(&d, &["part", ty, "can't hold children"]);
    }
}

#[test]
fn a_text_property_must_be_text() {
    let mut d = design();
    d["elements"][0]["children"][0]["properties"]["text"] = json!(2);
    refusal(&d, &["heading", "text", "must be text"]);
}

#[test]
fn a_heading_level_is_a_whole_number_from_one_to_three() {
    for (level, why) in [(json!(0), "must be 1, 2 or 3"), (json!(4), "must be 1, 2 or 3"), (json!(1.5), "whole number")] {
        let mut d = design();
        d["elements"][0]["children"] = json!([{
            "id": "title",
            "type": "heading",
            "properties": {"width": "hug", "height": "hug", "text": "Title", "level": level},
        }]);
        refusal(&d, &["title", "level", why]);
    }
}

#[test]
fn a_corrupted_newest_version_does_not_block_a_restore() {
    let top = crate::testing::Scratch::new();
    let id = "0123456789abcdef";
    save(&top, &design(), Source::Human).unwrap();
    let dir = versions_dir(&top, id);
    std::fs::write(dir.join("2.json"), "{").unwrap();
    let mut current = design();
    current["title"] = json!("Current");
    std::fs::write(top.join(DESIGNS_DIR).join(format!("{id}.json")), current.to_string()).unwrap();
    let listed = restore(&top, id, 1).unwrap();
    assert_eq!(show(&top, id).unwrap(), design());
    assert_eq!(listed.as_array().unwrap().len(), 3, "{listed}");
    assert_eq!(version(&top, id, 3).unwrap()["source"], json!({"kind": "repo"}));
    assert_eq!(version(&top, id, 3).unwrap()["design"], current);
    assert_eq!(version(&top, id, 4).unwrap()["design"], design());
}

#[test]
fn a_corrupted_newest_version_does_not_block_a_restore_of_a_damaged_current_design() {
    let top = crate::testing::Scratch::new();
    let id = "0123456789abcdef";
    save(&top, &design(), Source::Human).unwrap();
    let dir = versions_dir(&top, id);
    std::fs::write(dir.join("2.json"), "{").unwrap();
    std::fs::write(top.join(DESIGNS_DIR).join(format!("{id}.json")), b"<<<<<<< conflict").unwrap();
    let listed = restore(&top, id, 1).unwrap();
    assert_eq!(show(&top, id).unwrap(), design());
    assert_eq!(listed.as_array().unwrap().len(), 3, "{listed}");
    let raw = version(&top, id, 3).unwrap();
    assert_eq!(raw["source"], json!({"kind": "repo"}));
    assert_eq!(raw["raw"], json!("<<<<<<< conflict"));
    assert_eq!(restore(&top, id, 1).unwrap(), listed);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 4);
}

#[test]
fn a_design_link_finds_its_title() {
    let top = crate::testing::Scratch::new();
    save(&top, &design(), Source::Human).unwrap();
    assert_eq!(find(&top, "0123456789abcdef").unwrap(), json!({"id": "0123456789abcdef", "title": "Home", "tab": "ui-ux"}));
    assert_eq!(find(&top, "fedcba9876543210").unwrap_err().0, 404);
    assert_eq!(find(&top, "bad.id").unwrap_err().0, 400);
}

#[test]
fn a_design_link_refuses_a_file_that_is_not_a_design() {
    let top = crate::testing::Scratch::new();
    let path = top.join(DESIGNS_DIR).join("0123456789abcdef.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, br#"{"id":"0123456789abcdef","title":"Home","tab":"board"}"#).unwrap();
    let (status, failure) = find(&top, "0123456789abcdef").unwrap_err();
    assert_eq!(status, 409);
    assert!(failure.0.starts_with("E7033"), "{}", failure.0);
}

#[test]
fn design_commands_read_the_workspace_designs() {
    let top = crate::testing::Scratch::new();
    let design = design();
    save(&top, &design, Source::Human).unwrap();
    std::fs::write(top.join(DESIGNS_DIR).join("broken.json"), b"{}").unwrap();
    let listed: Value = serde_json::from_str(&command(&top, &["list"], 4000).unwrap()).unwrap();
    assert_eq!(listed["designs"][0]["id"], design["id"]);
    assert_eq!(listed["invalid"][0]["file"], "broken.json");
    let shown = command(&top, &["show", "design:0123456789abcdef"], 4000).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&shown).unwrap(), design);
}

#[test]
fn an_unknown_design_id_says_how_to_list() {
    let top = crate::testing::Scratch::new();
    let e = command(&top, &["show", "design:fedcba9876543210"], 4000).unwrap_err();
    assert!(e.0.starts_with("E7023") && e.0.contains("`design list`"), "{}", e.0);
}

#[test]
fn a_large_design_reply_is_cut() {
    let top = crate::testing::Scratch::new();
    let mut design = design();
    design["tab"] = json!("architecture");
    design["elements"] = json!(
        (0..200)
            .map(|i| {
                json!({"id": format!("e{i}"), "type": "text",
                   "properties": {"width": "hug", "height": "hug", "text": format!("Line {i}")}})
            })
            .collect::<Vec<_>>()
    );
    save(&top, &design, Source::Human).unwrap();
    let out = command(&top, &["show", "0123456789abcdef"], 800).unwrap();
    assert!(out.len() <= 800, "{} bytes", out.len());
    assert!(out.contains("cut at the reply limit") && out.contains("200 elements"), "{out}");
    assert!(!out.contains("Line 199"), "{out}");
}

#[test]
fn an_unknown_spacing_token_is_refused() {
    for name in ["gap", "padding"] {
        for token in ["huge", "8"] {
            let mut d = design();
            d["elements"][0]["properties"][name] = json!(token);
            refusal(&d, &["root", name, token]);
        }
    }
}

#[test]
fn a_cut_design_reads_on_from_an_element() {
    let top = crate::testing::Scratch::new();
    let mut design = design();
    design["tab"] = json!("architecture");
    design["elements"] = json!(
        (0..200)
            .map(|i| {
                json!({"id": format!("e{i}"), "type": "text",
                   "properties": {"width": "hug", "height": "hug", "text": format!("Line {i}")}})
            })
            .collect::<Vec<_>>()
    );
    save(&top, &design, Source::Human).unwrap();
    let first = command(&top, &["show", "0123456789abcdef"], 20_000).unwrap();
    assert!(!first.contains("Line 199"));
    let next = first.split("--from ").nth(1).and_then(|s| s.split('`').next()).expect("a cut design names its next element");
    let second = command(&top, &["show", "0123456789abcdef", "--from", next], 20_000).unwrap();
    assert!(second.contains("Line 199"));
    let mut shown = Vec::new();
    for part in [first.as_str(), second.as_str()] {
        let body = part.split("\n… cut at the reply limit").next().unwrap();
        let page: Value = serde_json::from_str(body).unwrap();
        shown.extend(page["elements"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap().to_string()));
    }
    assert_eq!(shown, (0..200).map(|i| format!("e{i}")).collect::<Vec<_>>());
}

#[test]
fn design_show_refuses_an_unknown_element_or_option() {
    let top = crate::testing::Scratch::new();
    save(&top, &design(), Source::Human).unwrap();
    let missing = command(&top, &["show", "0123456789abcdef", "--from", "nope"], 4000).unwrap_err();
    assert!(missing.0.contains("no element `nope`"), "{}", missing.0);
    let option = command(&top, &["show", "0123456789abcdef", "--after", "root"], 4000).unwrap_err();
    assert!(option.0.contains("--from"), "{}", option.0);
}

#[test]
fn a_large_element_stays_within_the_reply_limit() {
    let top = crate::testing::Scratch::new();
    let mut design = design();
    design["tab"] = json!("architecture");
    let long = "x".repeat(1000);
    design["elements"] = json!([
        {"id": "e0", "type": "text", "properties": {"width": "hug", "height": "hug", "text": "small"}},
        {"id": "e1", "type": "text", "properties": {"width": "hug", "height": "hug", "text": long.clone()}},
        {"id": "e2", "type": "text", "properties": {"width": "hug", "height": "hug", "text": long}},
    ]);
    save(&top, &design, Source::Human).unwrap();
    let first = command(&top, &["show", "0123456789abcdef"], 400).unwrap();
    assert!(first.len() <= 400, "{} bytes", first.len());
    assert!(first.contains("--from e1"), "{first}");
    let middle = command(&top, &["show", "0123456789abcdef", "--from", "e1"], 400).unwrap();
    assert!(middle.len() <= 400, "{} bytes", middle.len());
    assert!(middle.contains("--from e2"), "{middle}");
    let last = command(&top, &["show", "0123456789abcdef", "--from", "e2"], 400).unwrap();
    assert!(last.len() <= 400, "{} bytes", last.len());
    assert!(last.contains("too large to show whole"), "{last}");
}
