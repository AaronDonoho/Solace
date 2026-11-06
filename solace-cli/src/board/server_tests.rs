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
        assert!(state.contains("roleModels"), "{state}");
    });
}

