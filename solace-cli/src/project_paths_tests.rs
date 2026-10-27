use super::*;
#[test]
fn project_dir_cleans_a_folder_name() {
    assert_eq!(project_dir(""), ".");
    assert_eq!(project_dir("./web/"), "web");
    assert_eq!(project_dir("web///"), "web");
    assert_eq!(project_dir("./"), ".");
}

#[test]
fn project_folder_resolves_a_relative_member() {
    let root = crate::testing::Scratch::new();
    let here = root.join("web");
    let members = vec!["web".to_string(), "r".to_string()];
    assert_eq!(project_folder(&root, Some(&here), "../r", &members), "r");
    assert_eq!(project_folder(&root, Some(&here), "./r", &members), "r");
    assert_eq!(project_folder(&root, Some(&here), "../nope", &members), "../nope");
    assert_eq!(project_folder(&root, Some(&here), "../../r", &members), "../../r");
    assert_eq!(project_folder(&root, None, "./r", &members), "r");
}

#[test]
fn same_project_compares_folders() {
    let root = crate::testing::Scratch::new();
    std::fs::create_dir_all(root.join("web")).unwrap();
    assert!(same_project(&root, &root));
    assert!(same_project(&root.join("web"), &root.join("./web")));
    assert!(!same_project(&root.join("web"), &root.join("other")));
}
