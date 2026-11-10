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
    for line in ["task ask What next?", "strategy vision Add a page", "design propose 0123456789abcdef"] {
        assert!(crate::cli::is_question(line), "{line}");
    }
    for line in ["view app.main", "probe --from tests.one", "design note A calm design"] {
        assert!(!crate::cli::is_question(line), "{line}");
    }
}

#[test]
fn session_remember_keeps_its_own_reply_room() {
    let budget = 500;
    let inspect_header = "==== solace inspect app.main ====\n".len();
    let mut rooms = vec![];
    let (out, _) = batch(&["session remember", "inspect app.main"], budget, &mut |w: &[String], room: usize| -> crate::failure::Out {
        rooms.push((w[0].clone(), room));
        Ok(if w[0] == "session" { "r".repeat(room) } else { "i".repeat(room) })
    });
    assert_eq!(rooms, [("session".to_string(), budget), ("inspect".to_string(), budget - inspect_header)]);
    assert!(out.contains(&"r".repeat(budget)) && out.contains(&"i".repeat(budget - inspect_header)), "{out}");
    rooms.clear();
    let (out, _) = batch(&["inspect app.main", "session remember"], budget, &mut |w: &[String], room: usize| -> crate::failure::Out {
        rooms.push((w[0].clone(), room));
        Ok("x".repeat(room))
    });
    assert_eq!(rooms, [("inspect".to_string(), budget - inspect_header), ("session".to_string(), budget)]);
    assert_eq!(out.matches('x').count(), budget - inspect_header + budget);
}

#[test]
fn every_line_gets_the_room_the_headers_leave() {
    let budget = 500;
    let commands = ["inspect app.main", "view app.main", "find app.main"];
    let headers = ["==== solace inspect app.main ====\n", "==== solace view app.main ====\n", "==== solace find app.main ====\n"];
    let mut rooms = vec![];
    let (out, _) = batch(&commands, budget, &mut |w: &[String], room: usize| -> crate::failure::Out {
        rooms.push((w.join(" "), room));
        Ok("x".repeat(room))
    });
    let mut used = 0;
    for (at, header) in headers.iter().enumerate() {
        used += header.len();
        let room = budget.saturating_sub(used);
        assert_eq!(rooms[at], (commands[at].to_string(), room));
        used += room;
        if room > 0 {
            used += 1;
        }
    }
    assert!(out.contains(&"x".repeat(budget - headers[0].len())), "{out}");
}

