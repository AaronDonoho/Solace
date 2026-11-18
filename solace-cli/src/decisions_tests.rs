#[test]
fn an_entry_needs_a_title_a_body_and_a_why() {
    let e = crate::decisions::entry("## Toolchain in Rust\nUse Rust.\nWhy: fast.\n").unwrap();
    assert_eq!(e.title, "Toolchain in Rust");
    assert_eq!(e.body, "Use Rust.\nWhy: fast.");
    assert!(crate::decisions::entry("Use Rust.\nWhy: fast.").is_err());
    assert!(crate::decisions::entry("## No why\nUse Rust.").is_err());
    assert!(crate::decisions::entry("## Only a title").is_err());
    let heading = crate::decisions::entry("## Split\nUse Rust.\n## D-90 Inner\nWhy: fast.").err().unwrap();
    assert!(heading.0.contains("only the title line"), "{}", heading.0);
    // A number the agent wrote names the record the text changes (D-255).
    let numbered = crate::decisions::entry("## D-9 Numbered\nUse Rust.\nWhy: fast.").unwrap();
    assert_eq!((numbered.edit, numbered.title.as_str()), (Some(9), "Numbered"));
}

#[test]
fn approving_appends_the_next_number_and_keeps_older_entries() {
    let root = crate::testing::Scratch::new();
    crate::decisions::ensure(&root).unwrap();
    let entry = |title: &str| crate::decisions::Entry { title: title.into(), body: "Do it.\nWhy: because.".into(), edit: None };
    assert_eq!(crate::decisions::append(&root, &entry("One"), "human", "board task \"Build it\"", None).unwrap(), "D-1 One");
    assert_eq!(crate::decisions::append(&root, &entry("Two"), "human", "board task \"Ship it\"", None).unwrap(), "D-2 Two");
    let text = std::fs::read_to_string(crate::decisions::file(&root)).unwrap();
    assert!(text.starts_with("# Decisions\n\n"), "{text}");
    let date = super::today();
    let all = crate::decisions::records(&text);
    assert_eq!(
        all.iter().map(|r| r.text.clone()).collect::<Vec<_>>(),
        [
            format!("## D-1 One\n(human, board task \"Build it\", {date})\nDo it.\nWhy: because."),
            format!("## D-2 Two\n(human, board task \"Ship it\", {date})\nDo it.\nWhy: because."),
        ]
    );
}

#[test]
fn two_open_entries_both_apply_in_the_order_they_are_approved() {
    let root = crate::testing::Scratch::new();
    let mut first = serde_json::json!({"settings":{"decision":{"title":"A","body":"Do A.\nWhy: a."}}});
    let mut second = serde_json::json!({"settings":{"decision":{"title":"B","body":"Do B.\nWhy: b."}}});
    crate::board::settings::apply_settings(&root, "Fix login", &mut first).unwrap();
    crate::board::settings::apply_settings(&root, "Fix login", &mut second).unwrap();
    assert_eq!(first["answer"], "Approved and added D-1 A");
    assert_eq!(second["answer"], "Approved and added D-2 B");
    let text = std::fs::read_to_string(crate::decisions::file(&root)).unwrap();
    let date = super::today();
    assert!(text.contains(&format!("## D-1 A\n(human, board task \"Fix login\", {date})\n")), "{text}");
    assert!(text.contains(&format!("## D-2 B\n(human, board task \"Fix login\", {date})\n")), "{text}");
    assert!(!text.contains("t1"), "{text}");
}

#[test]
fn titles_are_newest_first_and_a_brief_names_the_earlier_ones() {
    let root = crate::testing::Scratch::new();
    let records: String = (1..=14).map(|n| format!("## D-{n} Thing {n}\nWhy: x\n\n")).collect();
    std::fs::write(crate::decisions::file(&root), format!("# Decisions\n\n{records}")).unwrap();
    let api = crate::decisions::api(&root);
    assert_eq!(api["decisions"][0]["id"], "D-14");
    assert_eq!(api["decisions"][0]["title"], "Thing 14");
    assert_eq!(api["decisions"][0]["text"], "## D-14 Thing 14\nWhy: x");
    let brief = crate::decisions::section(&root, "work");
    assert!(brief.contains("- D-5 Thing 5") && brief.contains("- D-14 Thing 14") && !brief.contains("- D-4 Thing"), "{brief}");
    // The header is lines 1–2; each record is two lines and a blank one.
    assert!(brief.contains("- D-14 Thing 14 (lines 42..43)"), "{brief}");
    assert!(brief.contains("… 4 earlier decisions: `read DECISIONS.md` lists them."), "{brief}");
    assert!(brief.contains("`read DECISIONS.md --lines a..b`"), "{brief}");
    assert!(brief.contains("what the product does or refuses to do"), "{brief}");
    assert!(brief.contains("`task decision`"), "{brief}");
    assert!(brief.contains("a body never carries one"), "{brief}");
    let review = crate::decisions::section(&root, "review");
    assert!(review.contains("A review can't propose one") && !review.contains("`task decision`"), "{review}");
    assert!(crate::decisions::section(&root, "strategy").contains("`strategy decision`"));
}

#[test]
fn an_approved_edit_replaces_only_that_record() {
    let root = crate::testing::Scratch::new();
    crate::decisions::ensure(&root).unwrap();
    let entry = |title: &str| super::Entry { title: title.into(), body: "Do it.\nWhy: because.".into(), edit: None };
    super::append(&root, &entry("One"), "human", "board task \"A\"", None).unwrap();
    super::append(&root, &entry("Two"), "human", "board task \"A\"", None).unwrap();
    let two = super::records(&std::fs::read_to_string(super::file(&root)).unwrap())[1].text.clone();
    let base = super::current(&root, 1).unwrap();
    let edit = super::Entry { title: "One, again".into(), body: "Do it better.\nWhy: clearer.".into(), edit: Some(1) };
    assert_eq!(super::append(&root, &edit, "human", "board task \"B\"", Some(&base)).unwrap(), "D-1 One, again");
    let text = std::fs::read_to_string(super::file(&root)).unwrap();
    let all = super::records(&text);
    assert_eq!(all.len(), 2);
    assert!(all[0].text.starts_with("## D-1 One, again\n(human, board task \"A\", "), "{}", all[0].text);
    assert!(all[0].text.contains("\nChanged by (human, board task \"B\", "), "{}", all[0].text);
    assert!(all[0].text.ends_with("\nDo it better.\nWhy: clearer."), "{}", all[0].text);
    assert_eq!(all[1].text, two);
}

#[test]
fn an_edit_after_the_record_changed_writes_nothing() {
    let root = crate::testing::Scratch::new();
    crate::decisions::ensure(&root).unwrap();
    let entry = |title: &str| super::Entry { title: title.into(), body: "Do it.\nWhy: because.".into(), edit: None };
    super::append(&root, &entry("One"), "human", "board task \"A\"", None).unwrap();
    let base = super::current(&root, 1).unwrap();
    let first = super::Entry { title: "First change".into(), body: "One.\nWhy: x.".into(), edit: Some(1) };
    super::append(&root, &first, "human", "board task \"B\"", Some(&base)).unwrap();
    let before = std::fs::read_to_string(super::file(&root)).unwrap();
    let stale = super::Entry { title: "Second change".into(), body: "Two.\nWhy: y.".into(), edit: Some(1) };
    let e = super::append(&root, &stale, "human", "board task \"C\"", Some(&base)).unwrap_err();
    assert!(e.0.contains("changed since the proposal"), "{}", e.0);
    assert_eq!(std::fs::read_to_string(super::file(&root)).unwrap(), before);
}

#[test]
fn a_decision_edit_shows_the_current_and_proposed_text() {
    let root = crate::testing::Scratch::new();
    crate::decisions::ensure(&root).unwrap();
    let entry = super::Entry { title: "One".into(), body: "Do it.\nWhy: because.".into(), edit: None };
    super::append(&root, &entry, "human", "board task \"A\"", None).unwrap();
    let q = crate::tasks::strategy::proposal(&root, "r1", "decision", None, "## D-1 Better\nDo more.\nWhy: x.").unwrap();
    let text = q["text"].as_str().unwrap();
    assert!(text.contains("## D-1 One") && text.contains("Why: because."), "{text}");
    assert!(text.contains("## D-1 Better") && text.contains("Do more."), "{text}");
}

#[test]
fn a_decision_number_that_names_no_record_is_refused() {
    let root = crate::testing::Scratch::new();
    crate::decisions::ensure(&root).unwrap();
    let e = crate::tasks::strategy::proposal(&root, "r1", "decision", None, "## D-99 Missing\nNo.\nWhy: x.").err().unwrap();
    assert!(e.0.contains("E6021") && e.0.contains("D-99"), "{}", e.0);
}

#[test]
fn a_local_offset_changes_the_day() {
    // 2026-11-17 23:30 UTC, then crossing midnight at an hour east of it.
    assert_eq!(super::date_at(1_794_958_200, 0), "2026-11-17");
    assert_eq!(super::date_at(1_794_958_200, 3_600), "2026-11-18");
}

#[test]
fn an_edit_of_an_old_layout_record_keeps_its_attribution() {
    let root = crate::testing::Scratch::new();
    super::ensure(&root).unwrap();
    std::fs::write(super::file(&root), "# Decisions\n\n## D-1 T\nAmends D-0.\nDo it.\nWhy: x.\n(human, board task abc, 2026-11-18)\n")
        .unwrap();
    let base = super::current(&root, 1).unwrap();
    let edit = super::Entry { title: "T2".into(), body: "Do more.\nWhy: y.".into(), edit: Some(1) };
    super::append(&root, &edit, "human", "board task \"B\"", Some(&base)).unwrap();
    let text = std::fs::read_to_string(super::file(&root)).unwrap();
    let all = super::records(&text);
    assert_eq!(all.len(), 1);
    let lines: Vec<&str> = all[0].text.lines().collect();
    assert_eq!(lines[0], "## D-1 T2");
    assert_eq!(lines[1], "(human, board task abc, 2026-11-18)");
    assert!(lines[2].starts_with("Changed by ("), "{}", all[0].text);
    assert_eq!(lines.iter().filter(|l| l.contains(", board task ")).count(), 2, "{}", all[0].text);
    assert!(all[0].text.ends_with("\nDo more.\nWhy: y."), "{}", all[0].text);

    std::fs::write(super::file(&root), "# Decisions\n\n## D-1 T\n(human, board task abc, 2026-11-18)\n(Side note.)\nWhy: x.\n").unwrap();
    let base = super::current(&root, 1).unwrap();
    let edit = super::Entry { title: "T3".into(), body: "New.\nWhy: z.".into(), edit: Some(1) };
    super::append(&root, &edit, "human", "board task \"B\"", Some(&base)).unwrap();
    let text = std::fs::read_to_string(super::file(&root)).unwrap();
    let all = super::records(&text);
    let lines: Vec<&str> = all[0].text.lines().collect();
    assert_eq!(lines[1], "(human, board task abc, 2026-11-18)");
    assert!(lines[2].starts_with("Changed by ("), "{}", all[0].text);
    assert!(!lines.contains(&"(Side note.)"), "{}", all[0].text);
    assert_eq!(lines.iter().filter(|l| **l == "New.").count(), 1, "{}", all[0].text);
}
