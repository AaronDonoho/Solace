use super::*;
use testing::Scratch;

/// D-27: a folder skill with front matter and a single-file skill are listed
/// with name, description and path; a skill with no description is listed by
/// its first line; a top-level README is the folder's note, not a skill; the
/// section carries no skill contents.
#[test]
fn the_skills_folder_lists_each_skill_by_name_description_and_path() {
    let root = Scratch::new().with(&[
        ("skills/impeccable/SKILL.md", "---\nname: impeccable\ndescription: \"Design direction for apps: layout, color and type.\"\n---\n# Impeccable\n\nRead the rules below.\n"),
        ("skills/plain.md", "Plain style\n\nKeep it simple.\n"),
        ("skills/README.md", "How this folder works.\n"),
        ("skills/impeccable/examples.md", "examples\n"),
    ]);
    let skills = list(&root);
    let shown: Vec<(&str, &str, &str)> = skills.iter().map(|s| (s.name.as_str(), s.line.as_str(), s.path.as_str())).collect();
    assert_eq!(
        shown,
        vec![
            ("impeccable", "Design direction for apps: layout, color and type.", "skills/impeccable/SKILL.md"),
            ("plain", "Plain style", "skills/plain.md"),
        ]
    );
    let s = section_for(&[&root], Some(&root), "work");
    assert!(s.contains("\nSkills (the project's house rules"), "{s}");
    assert!(s.contains("- impeccable: Design direction for apps: layout, color and type. (`read skills/impeccable/SKILL.md`)"), "{s}");
    assert!(s.contains("- plain: Plain style (`read skills/plain.md`)"), "{s}");
    assert!(s.contains("when the task matches a skill's description"), "{s}");
    assert!(!s.contains("Read the rules below") && !s.contains("Keep it simple"), "the brief carries no skill contents: {s}");
}

/// D-27: no skills folder, no section.
#[test]
fn a_project_without_skills_has_no_section() {
    let root = Scratch::new().with(&[("README.md", "x\n")]);
    assert!(list(&root).is_empty() && section_for(&[&root], Some(&root), "work").is_empty());
}

/// A CRLF skill file: the body starts after the closing `---` line, so a
/// skill with no description is listed by the body's first line, not `--`.
#[test]
fn a_crlf_skill_file_lists_the_body_first_line() {
    let root = Scratch::new().with(&[
        ("skills/crlf.md", "---\r\nname: crlf\r\n---\r\n# CRLF rules\r\n\r\nKeep them.\r\n"),
        ("skills/lf.md", "---\nname: lf\n---\n# LF rules\n"),
    ]);
    let skills = list(&root);
    let shown: Vec<(&str, &str)> = skills.iter().map(|s| (s.name.as_str(), s.line.as_str())).collect();
    assert_eq!(shown, vec![("crlf", "CRLF rules"), ("lf", "LF rules")]);
}

/// D-27: the `skills` key names the folder, and the default is `skills/`.
#[test]
fn the_skills_key_overrides_the_default_folder() {
    let root = Scratch::new().with(&[
        ("solace.toml", "skills = \"house\"\n"),
        ("house/x.md", "---\ndescription: X rules.\n---\n"),
        ("skills/ignored.md", "Not listed.\n"),
    ]);
    let v = lockdown::settings(&root).unwrap();
    assert_eq!(lockdown::skills_dir(&v), "house");
    assert_eq!(lockdown::skills_dir(&toml::Value::Table(Default::default())), "skills");
    let skills = list(&root);
    let shown: Vec<&str> = skills.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(shown, vec!["house/x.md"]);
    let shown = section_for(&[&root], Some(&root), "work");
    assert!(shown.contains("- x: X rules. (`read house/x.md`)"), "{shown}");
}

/// D-27: a skill with roles reaches only that role's brief, in full; a skill
/// with no roles stays one line for every role.
#[test]
fn role_skills_reach_only_their_role_in_full() {
    let root = Scratch::new().with(&[
        ("skills/review.md", "---\nname: review\ndescription: Review UI.\nroles: reviewer\n---\nCheck contrast.\n"),
        ("skills/work.md", "---\nname: work\ndescription: Work style.\nroles: worker, reviewer\n---\nKeep it small.\n"),
        ("skills/folder/SKILL.md", "---\nname: folder\ndescription: A folder skill.\nroles: reviewer\n---\nFolder rules.\n"),
        ("skills/general.md", "General rules\nKeep it simple.\n"),
        ("skills/nobody.md", "---\nname: nobody\ndescription: Nobody's rules.\nroles:\n---\nNothing.\n"),
    ]);
    let review = section_for(&[&root], Some(&root), "review");
    assert!(review.contains("### review") && review.contains("Check contrast."), "{review}");
    assert!(review.contains("### work") && review.contains("Keep it small."), "{review}");
    assert!(review.contains("- general: General rules (`read skills/general.md`)"), "{review}");
    assert!(review.contains("- nobody: Nobody's rules. (`read skills/nobody.md`)"), "{review}");
    let work = section_for(&[&root], Some(&root), "work");
    assert!(work.contains("### work") && !work.contains("### review") && !work.contains("Check contrast."), "{work}");
    assert!(work.contains("- general: General rules"), "{work}");
    let investigation = section_for(&[&root], Some(&root), "investigate");
    assert!(!investigation.contains("### work") && !investigation.contains("### review"), "{investigation}");
    assert!(investigation.contains("- general: General rules"), "{investigation}");
    let api = api(&root);
    assert_eq!(api.as_array().unwrap().len(), 2, "Settings manages only skills with roles: {api}");
    assert!(
        api.as_array().unwrap().iter().all(|skill| skill["name"] != "general" && skill["name"] != "folder" && skill["name"] != "nobody"),
        "Settings manages only single-file skills with roles: {api}"
    );
}
