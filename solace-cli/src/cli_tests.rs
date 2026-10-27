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

