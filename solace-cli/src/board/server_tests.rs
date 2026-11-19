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

/// A stream that fails on the server ends the response, so the browser sees
/// the stream close and reconnects instead of waiting forever.
#[test]
fn a_failing_stream_ends_the_connection() {
    use std::io::{Read, Write};
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    std::fs::write(p.root.join("solace.toml"), "invalid = [").unwrap();
    let board = std::sync::Arc::new(Board::new(scope::top(&p.root), 0));
    let reader = Events::new(board, None, Some("{\"tasks\":0}"));
    let server = tiny_http::Server::http(("127.0.0.1", 0)).unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let worker = std::thread::spawn(move || {
        let request = server.recv().unwrap();
        respond_events(request, reader);
    });
    let mut socket = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    socket.write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut response = Vec::new();
    let mut buf = [0; 1024];
    loop {
        let n = socket.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        response.extend_from_slice(&buf[..n]);
        if response.ends_with(b"0\r\n\r\n") {
            break;
        }
    }
    worker.join().unwrap();
    let text = String::from_utf8_lossy(&response);
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    assert!(response.ends_with(b"0\r\n\r\n"), "{text}");
}

/// The stream's headers arrive at once, not only with its first event: a
/// client waits for them before it starts reading the stream.
#[test]
fn a_stream_sends_its_headers_before_the_first_event() {
    use std::io::{Read, Write};
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let board = std::sync::Arc::new(Board::new(scope::top(&p.root), 0));
    let reader = Events::new(board, Some(p.root.clone()), Some("{\"tasks\":0}"));
    let server = tiny_http::Server::http(("127.0.0.1", 0)).unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    std::thread::spawn(move || {
        let request = server.recv().unwrap();
        respond_events(request, reader);
    });
    let mut socket = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
    socket.write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\nLast-Event-ID: 0\r\n\r\n").unwrap();
    let mut head = Vec::new();
    let mut byte = [0; 1];
    while !head.ends_with(b"\r\n\r\n") {
        socket.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 200"), "{head:?}");
}

/// A panic in the stream reaches the browser as the end of the response, so
/// it reconnects instead of waiting on a connection that stays open.
#[test]
fn a_panicking_stream_ends_the_connection() {
    use std::io::{Read, Write};
    struct Boom;
    impl Read for Boom {
        fn read(&mut self, _out: &mut [u8]) -> std::io::Result<usize> {
            panic!("stream panic");
        }
    }
    let server = tiny_http::Server::http(("127.0.0.1", 0)).unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let worker = std::thread::spawn(move || {
        let request = server.recv().unwrap();
        respond_events(request, Boom);
    });
    let mut socket = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    socket.write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut response = Vec::new();
    let mut buf = [0; 1024];
    loop {
        let n = socket.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        response.extend_from_slice(&buf[..n]);
        if response.ends_with(b"0\r\n\r\n") {
            break;
        }
    }
    worker.join().unwrap();
    assert!(response.ends_with(b"0\r\n\r\n"), "{}", String::from_utf8_lossy(&response));
}

/// A reconnect's position comes from the header or, when a page opens a new
/// stream, from the query; the header wins if both name one.
#[test]
fn a_reconnect_position_comes_from_the_header_or_the_query() {
    assert_eq!(events_position("/events?last=%7B%22tasks%22%3A0%7D", None).as_deref(), Some("{\"tasks\":0}"));
    assert_eq!(events_position("/events", Some("7")).as_deref(), Some("7"));
    assert_eq!(events_position("/events?last=1", Some("2")).as_deref(), Some("2"));
    assert_eq!(events_position("/events", None), None);
}

/// A browser reconnects about every 3 seconds while the server keeps failing:
/// the same reason must not print a line on every attempt.
#[test]
fn a_repeating_stream_error_is_logged_once() {
    let now = std::time::Instant::now();
    let quiet = std::time::Duration::from_secs(60);
    let mut last = Vec::new();
    assert!(note_event_error(&mut last, "invalid solace.toml", now, quiet));
    assert!(!note_event_error(&mut last, "invalid solace.toml", now + std::time::Duration::from_secs(3), quiet));
    assert!(!note_event_error(&mut last, "invalid solace.toml", now + std::time::Duration::from_secs(30), quiet));
    let changed = now + std::time::Duration::from_secs(31);
    assert!(note_event_error(&mut last, "another reason", changed, quiet));
    assert!(!note_event_error(&mut last, "another reason", changed + std::time::Duration::from_secs(59), quiet));
    assert!(note_event_error(&mut last, "another reason", changed + quiet, quiet));
}

/// Two streams failing for different reasons take turns: neither is news again
/// until its own quiet period has passed.
#[test]
fn alternating_stream_errors_each_print_once() {
    let now = std::time::Instant::now();
    let quiet = std::time::Duration::from_secs(60);
    let mut last = Vec::new();
    assert!(note_event_error(&mut last, "database", now, quiet));
    assert!(note_event_error(&mut last, "config", now + std::time::Duration::from_secs(3), quiet));
    assert!(!note_event_error(&mut last, "database", now + std::time::Duration::from_secs(6), quiet));
    assert!(!note_event_error(&mut last, "config", now + std::time::Duration::from_secs(9), quiet));
    assert!(note_event_error(&mut last, "database", now + quiet, quiet));
    assert!(note_event_error(&mut last, "config", now + quiet + std::time::Duration::from_secs(3), quiet));
}

/// A stream that keeps failing with a new reason must not grow the remembered
/// reasons without end.
#[test]
fn many_stream_error_reasons_stay_bounded() {
    let now = std::time::Instant::now();
    let quiet = std::time::Duration::from_secs(60);
    let mut last = Vec::new();
    for i in 0..100 {
        assert!(note_event_error(&mut last, &format!("reason {i}"), now + std::time::Duration::from_secs(i), quiet,));
    }
    assert!(last.len() <= 8, "{last:?}");
}

/// An unreadable settings file leaves the snapshot without a role model
/// event, so open pickers keep their defaults instead of being cleared.
#[test]
fn a_bad_role_model_config_leaves_open_pickers_alone() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let config = p.root.join("config.toml");
    std::fs::write(&config, "invalid = [").unwrap();
    crate::board::setup::with_config(&config, || {
        let board = std::sync::Arc::new(Board::new(p.root.clone(), 0));
        let events = Events::new(board, None, Some("{\"tasks\":0}"));
        let state = events.strategy_state();
        assert!(!state.contains("roleModels"), "{state}");
    });
}

/// A readable settings file puts the role models in the snapshot, so open
/// pickers receive the saved defaults.
#[test]
fn a_readable_role_model_config_sends_the_snapshot_role_models() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let config = p.root.join("config.toml");
    std::fs::write(&config, "[role_models.strategist]\ndefault = \"m1\"\n").unwrap();
    crate::board::setup::with_config(&config, || {
        let board = std::sync::Arc::new(Board::new(p.root.clone(), 0));
        let events = Events::new(board, None, Some("{\"tasks\":0}"));
        let state = events.strategy_state();
        let role_models = state
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .map(|data| serde_json::from_str::<serde_json::Value>(data).unwrap())
            .find_map(|event| event.get("roleModels").cloned())
            .unwrap_or_else(|| panic!("no roleModels event in {state}"));
        let strategist = role_models.as_array().unwrap().iter().find(|role| role["role"] == "strategist").unwrap();
        assert_eq!(strategist["default"].as_str(), Some("m1"), "{state}");
    });
}

#[test]
fn about_stays_at_the_root_when_the_root_is_a_project() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let board = Board::new(p.root.clone(), 0);
    assert!(!board.explicit(), "the root is one project");
    assert_eq!(board.route("/about").unwrap(), None);
    let board = std::sync::Arc::new(board);
    assert_eq!(board.projects_api("GET", "/about", &json!({})).unwrap(), json!({"version": env!("CARGO_PKG_VERSION")}));
}

fn design_value(id: &str, title: &str, tab: &str) -> Value {
    json!({
        "schemaVersion": crate::design::SCHEMA_VERSION,
        "id": id,
        "title": title,
        "tab": tab,
        "elements": [{
            "id": "screen",
            "type": "screen",
            "properties": {"width": "fill", "height": "fill"}
        }]
    })
}

#[test]
fn designs_at_a_project_root_list_and_read() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let board = Board::new(p.root.clone(), 0);
    assert!(!board.explicit(), "the root is one project");
    assert_eq!(board.route("/designs").unwrap(), None);
    assert_eq!(board.route("/designs/0123456789abcdef").unwrap(), None);
    let board = std::sync::Arc::new(board);
    assert_eq!(board.projects_api("GET", "/designs", &json!({})).unwrap(), json!({"designs": [], "invalid": []}));
    let dir = p.root.join("designs");
    std::fs::create_dir_all(dir.join("folder")).unwrap();
    std::fs::write(dir.join("notes.txt"), "not a design").unwrap();
    let home = design_value("aaaaaaaaaaaaaaaa", "Home", "architecture");
    let settings = design_value("bbbbbbbbbbbbbbbb", "Settings", "ui");
    std::fs::write(dir.join("aaaaaaaaaaaaaaaa.json"), home.to_string()).unwrap();
    std::fs::write(dir.join("bbbbbbbbbbbbbbbb.json"), settings.to_string()).unwrap();
    std::fs::write(dir.join("cccccccccccccccc.json"), "{").unwrap();
    let mismatch = design_value("dddddddddddddddd", "Mismatch", "ui");
    std::fs::write(dir.join("eeeeeeeeeeeeeeee.json"), mismatch.to_string()).unwrap();

    let listed = board.projects_api("GET", "/designs", &json!({})).unwrap();
    assert_eq!(
        listed["designs"],
        json!([
            {"id": "aaaaaaaaaaaaaaaa", "title": "Home", "tab": "architecture", "version": 0},
            {"id": "bbbbbbbbbbbbbbbb", "title": "Settings", "tab": "ui", "version": 0}
        ])
    );
    assert_eq!(listed["invalid"].as_array().unwrap().len(), 2, "{listed}");
    assert_eq!(listed["invalid"][0]["file"], "cccccccccccccccc.json");
    assert!(listed["invalid"][0]["reason"].as_str().unwrap().starts_with("E7033"), "{listed}");
    assert_eq!(listed["invalid"][1]["file"], "eeeeeeeeeeeeeeee.json");
    assert!(listed["invalid"][1]["reason"].as_str().unwrap().contains("does not match"), "{listed}");

    assert_eq!(board.projects_api("GET", "/designs/bbbbbbbbbbbbbbbb", &json!({})).unwrap(), settings);
    let bad = board.projects_api("GET", "/designs/cccccccccccccccc", &json!({})).unwrap_err();
    assert!(bad.0.starts_with("E7033"), "{}", bad.0);
    let missing = board.projects_api("GET", "/designs/ffffffffffffffff", &json!({})).unwrap_err();
    assert!(missing.0.starts_with("E7023"), "{}", missing.0);
    let traversal = board.projects_api("GET", "/designs/..%2F..%2Fsecret", &json!({})).unwrap_err();
    assert!(traversal.0.starts_with("E7023"), "{}", traversal.0);
}

#[test]
fn designs_list_when_the_root_is_not_a_project() {
    let root = crate::testing::Scratch::new();
    let board = std::sync::Arc::new(Board::new(root.to_path_buf(), 0));
    assert!(board.explicit(), "the root is not one project");
    assert_eq!(board.route("/designs").unwrap(), None);
    assert_eq!(board.route("/designs/0123456789abcdef").unwrap(), None);
    assert_eq!(board.projects_api("GET", "/designs", &json!({})).unwrap(), json!({"designs": [], "invalid": []}));
    let design = design_value("0123456789abcdef", "Home", "ui");
    std::fs::create_dir_all(root.join("designs")).unwrap();
    std::fs::write(root.join("designs/0123456789abcdef.json"), design.to_string()).unwrap();
    assert_eq!(
        board.projects_api("GET", "/designs", &json!({})).unwrap(),
        json!({"designs": [{"id": "0123456789abcdef", "title": "Home", "tab": "ui", "version": 0}], "invalid": []})
    );
    assert_eq!(board.projects_api("GET", "/designs/0123456789abcdef", &json!({})).unwrap(), design);
}

#[test]
fn a_missing_design_is_a_404() {
    use std::io::{Read, Write};
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let board = std::sync::Arc::new(Board::new(p.root.clone(), 0));
    let server = tiny_http::Server::http(("127.0.0.1", 0)).unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let worker = std::thread::spawn(move || {
        let request = server.recv().unwrap();
        let url = request.url().to_string();
        let result = board.projects_api("GET", &url, &json!({}));
        respond_json(request, result);
    });
    let mut socket = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
    socket.write_all(b"GET /designs/0123456789abcdef HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut head = Vec::new();
    let mut byte = [0; 1];
    while !head.ends_with(b"\r\n\r\n") {
        socket.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    worker.join().unwrap();
    assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 404"), "{head:?}");
}

#[test]
fn a_broken_design_is_a_409() {
    use std::io::{Read, Write};
    let root = crate::testing::Scratch::new();
    std::fs::create_dir_all(root.join("designs")).unwrap();
    std::fs::write(root.join("designs/0123456789abcdef.json"), "{").unwrap();
    let board = std::sync::Arc::new(Board::new(root.to_path_buf(), 0));
    let server = tiny_http::Server::http(("127.0.0.1", 0)).unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let worker = std::thread::spawn(move || {
        let request = server.recv().unwrap();
        let url = request.url().to_string();
        let result = board.projects_api("GET", &url, &json!({}));
        respond_json(request, result);
    });
    let mut socket = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
    socket.write_all(b"GET /designs/0123456789abcdef HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut head = Vec::new();
    let mut byte = [0; 1];
    while !head.ends_with(b"\r\n\r\n") {
        socket.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    worker.join().unwrap();
    assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 409"), "{head:?}");
}

#[test]
fn design_versions_list_read_and_restore() {
    let root = crate::testing::Scratch::new();
    let board = std::sync::Arc::new(Board::new(root.to_path_buf(), 0));
    let first = design_value("0123456789abcdef", "First", "architecture");
    let second = design_value("0123456789abcdef", "Second", "architecture");
    let third = design_value("0123456789abcdef", "Third", "architecture");
    for design in [&first, &second, &third] {
        crate::design::save(&root, design, crate::design::Source::Human).unwrap();
    }
    let listed = board.projects_api("GET", "/designs/0123456789abcdef/versions", &json!({})).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 3, "{listed}");
    assert_eq!(listed[0]["number"], json!(3));
    assert!(listed[0].get("design").is_none(), "{listed}");
    let one = board.projects_api("GET", "/designs/0123456789abcdef/versions/2", &json!({})).unwrap();
    assert_eq!(one["design"], second);
    let restored = board.projects_api("POST", "/designs/0123456789abcdef/restore", &json!({"version": 1})).unwrap();
    assert_eq!(restored.as_array().unwrap().len(), 4, "{restored}");
    assert_eq!(board.projects_api("GET", "/designs/0123456789abcdef", &json!({})).unwrap(), first);
    let unknown = board.projects_api("POST", "/designs/0123456789abcdef/restore", &json!({"version": 9})).unwrap_err();
    assert!(unknown.0.starts_with("E7023"), "{}", unknown.0);
    assert_eq!(board.projects_api("GET", "/designs/0123456789abcdef", &json!({})).unwrap(), first);
    let bad = board.projects_api("GET", "/designs/0123456789abcdef/versions/two", &json!({})).unwrap_err();
    assert!(bad.0.starts_with("E7001"), "{}", bad.0);
    let missing = board.projects_api("POST", "/designs/0123456789abcdef/restore", &json!({})).unwrap_err();
    assert!(missing.0.starts_with("E7001"), "{}", missing.0);
    assert_eq!(board.projects_api("GET", "/designs/0123456789abcdef/versions/2", &json!({})).unwrap()["design"], second);
    assert_eq!(board.projects_api("GET", "/designs/0123456789abcdef/versions/3", &json!({})).unwrap()["design"], third);
}

#[test]
fn design_version_routes_refuse_an_address_without_an_id() {
    let root = crate::testing::Scratch::new();
    let board = std::sync::Arc::new(Board::new(root.to_path_buf(), 0));
    let listed = board.projects_api("GET", "/designs/versions", &json!({})).unwrap_err();
    assert!(listed.0.starts_with("E7023"), "{}", listed.0);
    let restored = board.projects_api("POST", "/designs/restore", &json!({"version": 1})).unwrap_err();
    assert!(restored.0.starts_with("E7001"), "{}", restored.0);
    let plus = board.projects_api("GET", "/designs/0123456789abcdef/versions/+3", &json!({})).unwrap_err();
    assert!(plus.0.starts_with("E7001"), "{}", plus.0);
}

/// D-240: a new connection's replay carries design runs on their own, so the
/// Strategy page never sees them.
#[test]
fn design_runs_replay_apart_from_strategy_runs() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let top = crate::scope::top(&p.root);
    std::fs::create_dir_all(top.join(".solace/design/runs")).unwrap();
    std::fs::write(
        top.join(".solace/design/runs/d1.json"),
        json!({"id":"d1","kind":"design","state":"running","context":{"kind":"page"},"notes":[]}).to_string(),
    )
    .unwrap();
    let board = std::sync::Arc::new(Board::new(p.root.clone(), 0));
    let events = Events::new(board, None, Some("{\"tasks\":0}"));
    let state = events.strategy_state();
    let data: Vec<Value> = state.lines().filter_map(|line| line.strip_prefix("data: ")).map(|d| serde_json::from_str(d).unwrap()).collect();
    let runs = data.iter().find_map(|event| event.get("designRuns").cloned()).unwrap_or_else(|| panic!("no designRuns in {state}"));
    assert_eq!(runs[0]["id"], "d1", "{state}");
    let strategy = data.iter().find_map(|event| event.get("strategyRuns").cloned()).unwrap_or_else(|| panic!("no strategyRuns in {state}"));
    assert_eq!(strategy, json!([]), "{state}");
}

/// A new stream's task lists carry only the run fields a page reads, while
/// opening the task still returns them whole.
#[test]
fn a_snapshot_trims_task_runs_but_a_task_opens_whole() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let mut t = crate::board::task_model::new_task("Sized", "", &json!("backlog"), 1.);
    let id = t["id"].as_str().unwrap().to_string();
    t["runs"] = json!([{"id":"r1","kind":"work","state":"running","changes":["app.main"]}]);
    crate::tasks::put(&p.store, &t).unwrap();
    let board = std::sync::Arc::new(Board::new(p.root.clone(), 0));
    let mut events = Events::new(board, None, Some("{\"tasks\":0}"));
    let state = events.snapshot().unwrap();
    let tasks = state
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str::<Value>(data).unwrap())
        .find_map(|event| event.get("tasks").cloned())
        .unwrap_or_else(|| panic!("no tasks event in {state}"));
    let listed = tasks.as_array().unwrap().iter().find(|t| t["id"] == id.as_str()).unwrap();
    assert!(listed["runs"][0].get("changes").is_none(), "{listed}");
    let opened = crate::board::task_model::read_task_api(&p.root, &["tasks", id.as_str()]).unwrap();
    assert_eq!(opened["runs"][0]["changes"], json!(["app.main"]), "{opened}");
}

/// A task with no project still rides the root's stream, trimmed.
#[test]
fn homeless_tasks_on_the_root_stream_keep_only_listed_run_fields() {
    let root = crate::testing::Scratch::new().with(&[
        ("solace.toml", "projects = [\"sub\"]\n"),
        (
            ".solace/tasks/t1.json",
            concat!(
                r#"{"id":"t1","column":"backlog","home":"web","scope":["sub","web"],"#,
                r#""runs":[{"id":"r1","kind":"work","state":"running","changes":["app.main"]}],"notes":[]}"#,
            ),
        ),
        ("sub/.solace/keep", ""),
    ]);
    let board = std::sync::Arc::new(Board::new(root.to_path_buf(), 0));
    let events = Events::new(board, None, Some("{\"tasks\":0}"));
    let state = events.strategy_state();
    let homeless = state
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str::<Value>(data).unwrap())
        .find(|event| event["noProject"] == json!(true))
        .unwrap_or_else(|| panic!("no homeless task in {state}"));
    let t = homeless["tasks"].as_array().unwrap().iter().find(|t| t["id"] == "t1").unwrap();
    assert!(t["runs"][0].get("changes").is_none(), "{t}");
}

/// Answered questions leave a run list only once the run ended; an open run
/// keeps them, and design runs follow the same rule.
#[test]
fn run_lists_drop_questions_only_from_ended_runs() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let top = crate::scope::top(&p.root);
    let strategy = top.join(".solace/strategy/runs");
    std::fs::create_dir_all(&strategy).unwrap();
    std::fs::write(
        strategy.join("ended.json"),
        json!({"id":"ended","kind":"strategy","state":"ended","started":1,"questions":[{"id":"q1"}],"notes":[]}).to_string(),
    )
    .unwrap();
    std::fs::write(
        strategy.join("open.json"),
        json!({"id":"open","kind":"strategy","state":"running","started":2,"questions":[{"id":"q2"}],"notes":[]}).to_string(),
    )
    .unwrap();
    let design_dir = top.join(".solace/design/runs");
    std::fs::create_dir_all(&design_dir).unwrap();
    std::fs::write(
        design_dir.join("d1.json"),
        json!({"id":"d1","kind":"design","state":"ended","started":3,"questions":[{"id":"q3"}],"notes":[]}).to_string(),
    )
    .unwrap();
    let board = std::sync::Arc::new(Board::new(p.root.clone(), 0));
    let events = Events::new(board, None, Some("{\"tasks\":0}"));
    let state = events.strategy_state();
    let data: Vec<Value> =
        state.lines().filter_map(|line| line.strip_prefix("data: ")).map(|data| serde_json::from_str(data).unwrap()).collect();
    let runs = data.iter().find_map(|event| event.get("strategyRuns").cloned()).unwrap();
    let ended = runs.as_array().unwrap().iter().find(|run| run["id"] == "ended").unwrap();
    let open = runs.as_array().unwrap().iter().find(|run| run["id"] == "open").unwrap();
    assert!(ended.get("questions").is_none(), "{ended}");
    assert_eq!(open["questions"], json!([{"id":"q2"}]), "{open}");
    let design = data.iter().find_map(|event| event.get("designRuns").cloned()).unwrap();
    assert!(design[0].get("questions").is_none(), "{design}");
}

#[test]
fn design_rename_delete_and_undo_routes() {
    let root = crate::testing::Scratch::new();
    let board = std::sync::Arc::new(Board::new(root.to_path_buf(), 0));
    let first = design_value("0123456789abcdef", "First", "architecture");
    crate::design::save(&root, &first, crate::design::Source::Human).unwrap();
    let renamed = board.projects_api("POST", "/designs/0123456789abcdef/rename", &json!({"title": "Renamed"})).unwrap();
    assert_eq!(renamed["title"], json!("Renamed"));
    assert_eq!(board.projects_api("GET", "/designs/0123456789abcdef", &json!({})).unwrap()["title"], json!("Renamed"));
    assert_eq!(board.projects_api("GET", "/designs/0123456789abcdef/versions", &json!({})).unwrap().as_array().unwrap().len(), 2);
    let listed = board.projects_api("DELETE", "/designs/0123456789abcdef", &json!({})).unwrap();
    assert!(listed["designs"].as_array().unwrap().is_empty(), "{listed}");
    assert!(board.projects_api("GET", "/designs/0123456789abcdef", &json!({})).unwrap_err().0.starts_with("E7023"));
    assert!(board.projects_api("GET", "/designs/0123456789abcdef/versions", &json!({})).unwrap_err().0.starts_with("E7023"));
    let back = board.projects_api("POST", "/designs/0123456789abcdef/undo", &json!({})).unwrap();
    assert_eq!(back["designs"][0]["title"], json!("Renamed"));
    assert_eq!(board.projects_api("GET", "/designs/0123456789abcdef", &json!({})).unwrap()["title"], json!("Renamed"));
    let blank = board.projects_api("POST", "/designs/0123456789abcdef/rename", &json!({"title": " "})).unwrap_err();
    assert!(blank.0.starts_with("E7033"), "{}", blank.0);
}

#[test]
fn design_edit_route_saves_each_operation_as_a_human_version() {
    let root = crate::testing::Scratch::new();
    let board = std::sync::Arc::new(Board::new(root.to_path_buf(), 0));
    let original = json!({
        "schemaVersion": crate::design::SCHEMA_VERSION,
        "id": "0123456789abcdef",
        "title": "Home",
        "tab": "ui",
        "elements": [{
            "id": "screen",
            "type": "screen",
            "properties": {"width": "fill", "height": "fill"},
            "children": [
                {"id": "heading", "type": "text", "properties": {"width": "hug", "height": "hug", "text": "Home"}},
                {"id": "note", "type": "text", "properties": {"width": "hug", "height": "hug", "text": "Note"}}
            ]
        }]
    });
    crate::design::save(&root, &original, crate::design::Source::Human).unwrap();
    let edit =
        |operations: Value| board.projects_api("POST", "/designs/0123456789abcdef/edit", &json!({"operations": operations})).unwrap();
    let mut expected = original.clone();
    let add = json!([{
        "op": "add",
        "parent": "screen",
        "index": 0,
        "element": {"id": "body", "type": "text", "properties": {"width": "hug", "height": "hug", "text": "Body"}}
    }]);
    let added = edit(add.clone());
    crate::design::apply(&mut expected, add.as_array().unwrap()).unwrap();
    assert_eq!(added, expected);
    let set = json!([{"op": "set", "id": "heading", "field": "properties", "property": "text", "value": "Welcome"}]);
    let changed = edit(set.clone());
    crate::design::apply(&mut expected, set.as_array().unwrap()).unwrap();
    assert_eq!(changed, expected);
    let moved = json!([{"op": "move", "id": "note", "parent": "screen", "index": 0}]);
    let moved_design = edit(moved.clone());
    crate::design::apply(&mut expected, moved.as_array().unwrap()).unwrap();
    assert_eq!(moved_design, expected);
    let removed = json!([{"op": "remove", "id": "body"}]);
    let removed_design = edit(removed.clone());
    crate::design::apply(&mut expected, removed.as_array().unwrap()).unwrap();
    assert_eq!(removed_design, expected);
    assert_eq!(board.projects_api("GET", "/designs/0123456789abcdef", &json!({})).unwrap(), expected);
    let versions = board.projects_api("GET", "/designs/0123456789abcdef/versions", &json!({})).unwrap();
    assert_eq!(versions.as_array().unwrap().len(), 5, "{versions}");
    for version in versions.as_array().unwrap() {
        assert_eq!(version["source"]["kind"], "human", "{version}");
    }
}

#[test]
fn a_refused_design_edit_saves_nothing() {
    let root = crate::testing::Scratch::new();
    let board = std::sync::Arc::new(Board::new(root.to_path_buf(), 0));
    let original = json!({
        "schemaVersion": crate::design::SCHEMA_VERSION,
        "id": "0123456789abcdef",
        "title": "Home",
        "tab": "ui",
        "elements": [{
            "id": "screen",
            "type": "screen",
            "properties": {"width": "fill", "height": "fill"},
            "children": [
                {"id": "heading", "type": "text", "properties": {"width": "hug", "height": "hug", "text": "Home"}}
            ]
        }]
    });
    crate::design::save(&root, &original, crate::design::Source::Human).unwrap();
    let invalid = board
        .projects_api(
            "POST",
            "/designs/0123456789abcdef/edit",
            &json!({"operations": [{"op": "set", "id": "heading", "field": "properties", "property": "width", "value": "sideways"}]}),
        )
        .unwrap_err();
    assert!(invalid.0.starts_with("E7033") && invalid.0.contains("\"heading\""), "{}", invalid.0);
    assert_eq!(board.projects_api("GET", "/designs/0123456789abcdef", &json!({})).unwrap(), original);
    let missing = board
        .projects_api("POST", "/designs/0123456789abcdef/edit", &json!({"operations": [{"op": "remove", "id": "ghost"}]}))
        .unwrap_err();
    assert!(missing.0.starts_with("E7033") && missing.0.contains("\"ghost\""), "{}", missing.0);
    assert_eq!(board.projects_api("GET", "/designs/0123456789abcdef", &json!({})).unwrap(), original);
    let unknown = board.projects_api("POST", "/designs/fedcba9876543210/edit", &json!({"operations": []})).unwrap_err();
    assert!(unknown.0.starts_with("E7023"), "{}", unknown.0);
    let malformed = board.projects_api("POST", "/designs/0123456789abcdef/edit", &json!({})).unwrap_err();
    assert!(malformed.0.starts_with("E7001"), "{}", malformed.0);
    let unchanged = board.projects_api("POST", "/designs/0123456789abcdef/edit", &json!({"operations": []})).unwrap();
    assert_eq!(unchanged, original);
    assert_eq!(board.projects_api("GET", "/designs/0123456789abcdef/versions", &json!({})).unwrap().as_array().unwrap().len(), 1);
}

fn release_draft(root: &std::path::Path, version: &str, text: &str) -> std::path::PathBuf {
    let dir = root.join(crate::releases::folder(root)).join(version);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("release.json"), json!({"version": version, "time": 1, "artifacts": []}).to_string()).unwrap();
    std::fs::write(dir.join("notes.json"), json!({"version": version, "state": "draft", "text": text}).to_string()).unwrap();
    dir
}

#[test]
fn approving_edited_release_notes_keeps_them_with_the_release() {
    let root = crate::testing::Scratch::new();
    release_draft(&root, "1.0.0", "old draft");
    let board = std::sync::Arc::new(Board::new(root.to_path_buf(), 0));
    let saved = board.projects_api("POST", "/releases/1.0.0/notes/approve", &json!({"text": "# Notes\nEdited by hand"})).unwrap();
    assert_eq!(saved["notes"]["state"], json!("approved"));
    assert_eq!(saved["notes"]["text"], json!("# Notes\nEdited by hand"));
    let restarted = std::sync::Arc::new(Board::new(root.to_path_buf(), 0));
    let read = restarted.projects_api("GET", "/releases/1.0.0", &json!({})).unwrap();
    assert_eq!(read["notes"]["state"], json!("approved"));
    assert_eq!(read["notes"]["text"], json!("# Notes\nEdited by hand"));
    assert!(crate::releases::save_draft(&root, "1.0.0", "late draft").is_err());
    let read = restarted.projects_api("GET", "/releases/1.0.0", &json!({})).unwrap();
    assert_eq!(read["notes"]["text"], json!("# Notes\nEdited by hand"));
}

#[test]
fn rejecting_release_notes_stores_none_and_ends_the_open_draft_run() {
    let root = crate::testing::Scratch::new();
    let dir = release_draft(&root, "1.0.0", "old draft");
    let runs = root.join(".solace/release-notes");
    std::fs::create_dir_all(&runs).unwrap();
    std::fs::write(
        runs.join("open.json"),
        json!({
            "id": "open",
            "kind": crate::board::roles::NOTES_RUN,
            "state": "running",
            "context": {"kind": "release", "id": "1.0.0"},
            "notes": [],
            "questions": []
        })
        .to_string(),
    )
    .unwrap();
    let board = std::sync::Arc::new(Board::new(root.to_path_buf(), 0));
    let rejected = board.projects_api("POST", "/releases/1.0.0/notes/reject", &json!({})).unwrap();
    assert_eq!(rejected["notes"], Value::Null);
    assert!(!dir.join("notes.json").exists());
    let read = board.projects_api("GET", "/releases/1.0.0", &json!({})).unwrap();
    assert_eq!(read["notes"], Value::Null);
    assert!(crate::tasks::strategy::notes_runs(&root).into_iter().all(|run| run["state"] == "ended"));
}

#[test]
fn rejecting_approved_notes_keeps_them_with_the_release() {
    let root = crate::testing::Scratch::new();
    release_draft(&root, "1.0.0", "old draft");
    let board = std::sync::Arc::new(Board::new(root.to_path_buf(), 0));
    board.projects_api("POST", "/releases/1.0.0/notes/approve", &json!({"text": "# Approved"})).unwrap();
    let refused = board.projects_api("POST", "/releases/1.0.0/notes/reject", &json!({}));
    let e = refused.unwrap_err();
    assert!(e.0.contains("E7023"), "{}", e.0);
    let read = board.projects_api("GET", "/releases/1.0.0", &json!({})).unwrap();
    assert_eq!(read["notes"]["state"], json!("approved"));
    assert_eq!(read["notes"]["text"], json!("# Approved"));
}

/// A workflow row the workflow stream has not sent keeps tasks from
/// moving past it, so a later poll can send it first (D-78).
#[test]
fn a_workflow_row_waits_for_the_workflow_stream_before_tasks_advance() {
    let p = crate::testing::project(&[("src/app.gleam", "pub fn main() { 1 }\n")]);
    let top = crate::scope::top(&p.root);
    let board = std::sync::Arc::new(Board::new(top, 0));
    let last = json!({"tasks": 0, ".": 0}).to_string();
    let mut events = Events::new(board, Some(p.root.clone()), Some(&last));
    let store = crate::board::task_model::open(&p.root).unwrap();
    let workflow = crate::board::task_model::flow(&p.root).unwrap();
    crate::board::task_model::event(&store, json!({"workflow": workflow.clone()})).unwrap();
    assert_eq!(events.next_task().unwrap(), None);
    assert_eq!(events.tasks, 0);
    let text = events.next_workflow().unwrap().expect("no workflow event");
    let data = text.lines().find_map(|line| line.strip_prefix("data: ")).expect("no data line");
    let event: Value = serde_json::from_str(data).unwrap();
    assert_eq!(event["workflow"], workflow);
}

