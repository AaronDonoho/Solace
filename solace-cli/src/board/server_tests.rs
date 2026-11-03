use super::*;

#[test]
fn percent_encode_keeps_unreserved_and_escapes_the_rest() {
    assert_eq!(percent_encode("a b/é~-_.9"), "a%20b%2F%C3%A9~-_.9");
    assert_eq!(percent_encode(""), "");
}

/// A role model request stays at the root when the root is itself a project:
/// the project has no such route, so routing it there would refuse both a
/// save and a read.
#[test]
fn role_models_stay_at_the_root_when_the_root_is_a_project() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let board = Board::new(p.root.clone(), 0);
    assert!(!board.explicit(), "the root is one project");
    assert_eq!(board.route("/role-models").unwrap(), None);
    let config = p.root.join("config.toml");
    crate::board::setup::with_config(&config, || {
        let board = std::sync::Arc::new(Board::new(p.root.clone(), 0));
        let saved = board.projects_api("POST", "/role-models", &json!({"role": "worker", "default": "cc"})).unwrap();
        let worker = saved.as_array().unwrap().iter().find(|role| role["role"] == "worker").unwrap();
        assert_eq!(worker["default"].as_str(), Some("cc"));
        assert_eq!(board.projects_api("GET", "/role-models", &json!({})).unwrap(), saved);
    });
}

