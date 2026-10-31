use super::*;

#[test]
fn percent_encode_keeps_unreserved_and_escapes_the_rest() {
    assert_eq!(percent_encode("a b/é~-_.9"), "a%20b%2F%C3%A9~-_.9");
    assert_eq!(percent_encode(""), "");
}

