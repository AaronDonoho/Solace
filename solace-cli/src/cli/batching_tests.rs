use crate::cli::batching::looks_like_code;
use super::*;
/// A trait impl's spaces stay in one word, while ordinary words still split.
#[test]
fn command_words_keep_a_trait_impl_together() {
    assert_eq!(
        command_words("view crate.<Failure as From<std::io::Error>>::from --context"),
        ["view", "crate.<Failure as From<std::io::Error>>::from", "--context"]
    );
    assert_eq!(command_words("view crate.<Type as From<fn() -> u8>>::from"), ["view", "crate.<Type as From<fn() -> u8>>::from"]);
    assert_eq!(command_words("view crate.Foo<Vec<u8>>::new"), ["view", "crate.Foo<Vec<u8>>::new"]);
    assert_eq!(
        command_words("view crate.<Failure  as From<std::io::Error>>::from"),
        ["view", "crate.<Failure as From<std::io::Error>>::from"]
    );
    assert_eq!(command_words("view  a   b"), ["view", "a", "b"]);
    assert_eq!(command_words("grep a < b"), ["grep", "a", "<", "b"]);
}

#[test]
fn code_is_told_from_commands() {
    assert!(looks_like_code("let assert Ok(q) = checkout.quote(db, o.id, Standard)") && looks_like_code("echo q"));
    assert!(!looks_like_code("probe --from checkout_test.quote_test --trace pricing.quote"));
}

#[test]
fn a_question_is_told_from_a_command() {
    for line in ["task ask What next?", "strategy vision Add a page"] {
        assert!(crate::cli::is_question(line), "{line}");
    }
    for line in ["view app.main", "probe --from tests.one"] {
        assert!(!crate::cli::is_question(line), "{line}");
    }
}

