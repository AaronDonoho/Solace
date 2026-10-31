use super::*;

/// `%xx` decodes; a byte that isn't UTF-8 is replaced, and a non-hex
/// escape stays as written.
#[test]
fn percent_decoding_replaces_invalid_utf8() {
    assert_eq!(percent_decode("%41"), "A");
    assert_eq!(percent_decode("a%41b"), "aAb");
    assert_eq!(percent_decode("%E9"), "\u{FFFD}");
    assert_eq!(percent_decode("%zz"), "%zz");
}

/// A view request clears a parse from before it: the next Rust parse starts
/// at byte 1.
#[test]
fn a_view_request_leaves_no_rust_parser_spans() {
    std::thread::spawn(|| {
        let (_root, store) = crate::testing::rust_project("#[test]\nfn go() {}\n");
        let _: syn::Ident = syn::parse_str("earlier").expect("earlier is an identifier");
        super::route(&store, "GET", &["files", "Cargo.toml"], &std::collections::HashMap::new(), &serde_json::json!({})).unwrap();
        assert_eq!(crate::testing::rust_span_start(), "bytes(1..2)");
    })
    .join()
    .unwrap();
}
