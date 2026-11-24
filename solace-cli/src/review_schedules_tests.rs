use super::*;
use chrono::{FixedOffset, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};

/// 2026-11-23 is a Monday.
fn at(local: &str) -> u64 {
    instant(NaiveDateTime::parse_from_str(local, "%Y-%m-%d %H:%M").unwrap(), &Utc).unwrap()
}

fn due(body: Value, now: &str) -> Option<u64> {
    next_after(&schedule("s", &body, at(now), &Utc).unwrap(), at(now), &Utc)
}

fn board(top: &Path) -> std::sync::Arc<crate::board::server::Board> {
    std::sync::Arc::new(crate::board::server::Board::new(top.to_path_buf(), 0))
}

/// A time zone whose clocks change once, at `at` in UTC seconds, from the
/// `before` offset to the `after` one.
#[derive(Clone)]
struct ClockChange {
    at: i64,
    before: FixedOffset,
    after: FixedOffset,
}

impl ClockChange {
    /// One hour ahead of UTC, then two from 01:00 UTC on `day`: 02:00 to 03:00 local is skipped.
    fn forward(day: &str) -> ClockChange {
        ClockChange { at: change_at(day), before: hours(1), after: hours(2) }
    }

    /// Two hours ahead of UTC, then one from 01:00 UTC on `day`: 02:00 to 03:00 local comes twice.
    fn back(day: &str) -> ClockChange {
        ClockChange { at: change_at(day), before: hours(2), after: hours(1) }
    }

    fn offset_at(&self, utc: i64) -> FixedOffset {
        if utc < self.at { self.before } else { self.after }
    }
}

fn hours(n: i32) -> FixedOffset {
    FixedOffset::east_opt(n * 3600).unwrap()
}

fn change_at(day: &str) -> i64 {
    i64::try_from(at(&format!("{day} 01:00"))).unwrap()
}

impl TimeZone for ClockChange {
    type Offset = FixedOffset;

    fn from_offset(offset: &FixedOffset) -> ClockChange {
        ClockChange { at: i64::MAX, before: *offset, after: *offset }
    }

    fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<FixedOffset> {
        self.offset_from_local_datetime(&local.and_hms_opt(0, 0, 0).unwrap())
    }

    fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<FixedOffset> {
        let mut fits: Vec<(i64, FixedOffset)> = [self.before, self.after]
            .into_iter()
            .map(|offset| (local.and_utc().timestamp() - i64::from(offset.local_minus_utc()), offset))
            .filter(|(utc, offset)| self.offset_at(*utc) == *offset)
            .collect();
        fits.sort_by_key(|(utc, _)| *utc);
        fits.dedup_by_key(|(utc, _)| *utc);
        match fits[..] {
            [] => LocalResult::None,
            [(_, one)] => LocalResult::Single(one),
            [(_, earlier), (_, later), ..] => LocalResult::Ambiguous(earlier, later),
        }
    }

    fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
        self.offset_from_utc_datetime(&utc.and_hms_opt(0, 0, 0).unwrap())
    }

    fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
        self.offset_at(utc.and_utc().timestamp())
    }
}

#[test]
fn each_repeat_comes_due_at_its_next_time() {
    let daily = json!({"kinds": ["security"], "repeat": "daily", "time": "09:00"});
    assert_eq!(due(daily.clone(), "2026-11-23 08:59"), Some(at("2026-11-23 09:00")));
    assert_eq!(due(daily, "2026-11-23 09:00"), Some(at("2026-11-24 09:00")));
    let weekly = json!({"kinds": ["security"], "repeat": "weekly", "weekday": 1, "time": "09:00"});
    assert_eq!(due(weekly.clone(), "2026-11-22 12:00"), Some(at("2026-11-23 09:00")));
    assert_eq!(due(weekly, "2026-11-23 09:30"), Some(at("2026-11-30 09:00")));
    let sunday = json!({"kinds": ["security"], "repeat": "weekly", "weekday": 7, "time": "18:00"});
    assert_eq!(due(sunday, "2026-11-23 09:00"), Some(at("2026-11-29 18:00")));
    let monthly = json!({"kinds": ["security"], "repeat": "monthly", "day": 15, "time": "07:30"});
    assert_eq!(due(monthly.clone(), "2026-11-14 23:00"), Some(at("2026-11-15 07:30")));
    assert_eq!(due(monthly, "2026-11-15 07:30"), Some(at("2026-12-15 07:30")));
}

#[test]
fn a_monthly_day_past_a_short_months_end_comes_due_on_its_last_day() {
    let monthly = json!({"kinds": ["security"], "repeat": "monthly", "day": 31, "time": "09:00"});
    assert_eq!(due(monthly.clone(), "2026-11-23 09:00"), Some(at("2026-11-30 09:00")));
    assert_eq!(due(monthly, "2027-02-01 09:00"), Some(at("2027-02-28 09:00")));
}

#[test]
fn a_one_off_time_comes_due_once() {
    let once = json!({"kinds": ["security"], "repeat": "once", "date": "2026-12-01", "time": "10:00"});
    let s = schedule("s", &once, at("2026-11-23 09:00"), &Utc).unwrap();
    assert_eq!(s["next"], json!(at("2026-12-01 10:00")));
    assert_eq!(next_after(&s, at("2026-12-01 10:00"), &Utc), None);
}

#[test]
fn due_times_follow_the_local_time_zone() {
    let tz = FixedOffset::east_opt(2 * 3600).unwrap();
    let daily = json!({"kinds": ["security"], "repeat": "daily", "time": "09:00"});
    let s = schedule("s", &daily, at("2026-11-23 06:00"), &tz).unwrap();
    assert_eq!(s["next"], json!(at("2026-11-23 07:00")));
}

/// 02:30 doesn't happen the day the clocks go forward: the review comes due an
/// hour on, at 03:30 local, and at 02:30 again the day after.
#[test]
fn a_time_the_clocks_skip_comes_due_an_hour_later() {
    let tz = ClockChange::forward("2026-03-29");
    let daily = json!({"kinds": ["security"], "repeat": "daily", "time": "02:30"});
    let s = schedule("s", &daily, at("2026-03-29 00:00"), &tz).unwrap();
    assert_eq!(s["next"], json!(at("2026-03-29 01:30")));
    assert_eq!(next_after(&s, at("2026-03-29 01:30"), &tz), Some(at("2026-03-30 00:30")));
}

/// 02:30 happens twice the day the clocks go back: the review comes due at the
/// first, and not again until the next day.
#[test]
fn a_time_the_clocks_repeat_comes_due_once_at_the_first() {
    let tz = ClockChange::back("2026-10-25");
    let daily = json!({"kinds": ["security"], "repeat": "daily", "time": "02:30"});
    let s = schedule("s", &daily, at("2026-10-24 23:00"), &tz).unwrap();
    assert_eq!(s["next"], json!(at("2026-10-25 00:30")));
    assert_eq!(next_after(&s, at("2026-10-25 00:30"), &tz), Some(at("2026-10-26 01:30")));
}

#[test]
fn schedules_are_added_changed_and_removed_through_their_routes() {
    let root = crate::testing::Scratch::new();
    let board = board(&root);
    let route = |method: &str, path: &str, body: Value| board.projects_api(method, path, &body);
    assert_eq!(route("GET", "/system-reviews/schedules", json!(null)).unwrap(), json!([]));
    let daily = json!({"kinds": ["security", "code-quality"], "repeat": "daily", "time": "09:00"});
    let weekly = json!({"kinds": ["user-experience"], "repeat": "weekly", "weekday": 3, "time": "17:45"});
    route("POST", "/system-reviews/schedules", daily).unwrap();
    let added = route("POST", "/system-reviews/schedules", weekly).unwrap();
    assert_eq!(added.as_array().unwrap().len(), 2);
    let first = added[0]["id"].as_str().unwrap().to_string();
    let second = added[1]["id"].as_str().unwrap().to_string();
    assert_eq!((added[0]["kinds"].clone(), added[0]["time"].as_str()), (json!(["security", "code-quality"]), Some("09:00")));
    assert_eq!((added[1]["repeat"].as_str(), added[1]["weekday"].as_u64()), (Some("weekly"), Some(3)));
    assert!(added.as_array().unwrap().iter().all(|s| s["next"].as_u64().is_some_and(|n| n > crate::now())));
    let monthly = json!({"kinds": ["security"], "repeat": "monthly", "day": 1, "time": "08:00"});
    route("PUT", &format!("/system-reviews/schedules/{first}"), monthly).unwrap();
    route("DELETE", &format!("/system-reviews/schedules/{second}"), json!({})).unwrap();
    let read = super::tests::board(&root).projects_api("GET", "/system-reviews/schedules", &json!(null)).unwrap();
    assert_eq!(read.as_array().unwrap().len(), 1);
    assert_eq!(
        (read[0]["id"].as_str(), read[0]["repeat"].as_str(), read[0]["day"].as_u64()),
        (Some(first.as_str()), Some("monthly"), Some(1))
    );
    assert!(read[0].get("time").is_some() && read[0].get("weekday").is_none());
    let e = route("DELETE", &format!("/system-reviews/schedules/{second}"), json!({})).unwrap_err();
    assert!(e.0.starts_with("E7023"), "{}", e.0);
}

#[test]
fn an_incomplete_schedule_is_refused_and_not_saved() {
    let root = crate::testing::Scratch::new();
    let board = board(&root);
    for body in [
        json!({"kinds": [], "repeat": "daily", "time": "09:00"}),
        json!({"kinds": ["security"], "repeat": "daily"}),
        json!({"kinds": ["security"], "repeat": "daily", "time": "25:00"}),
        json!({"kinds": ["security"], "repeat": "hourly", "time": "09:00"}),
        json!({"kinds": ["security"], "repeat": "weekly", "time": "09:00"}),
        json!({"kinds": ["security"], "repeat": "weekly", "weekday": 8, "time": "09:00"}),
        json!({"kinds": ["security"], "repeat": "monthly", "day": 0, "time": "09:00"}),
        json!({"kinds": ["security"], "repeat": "once", "time": "09:00"}),
        json!({"kinds": ["security"], "repeat": "once", "date": "2020-01-01", "time": "09:00"}),
    ] {
        let e = board.projects_api("POST", "/system-reviews/schedules", &body).unwrap_err();
        assert!(e.0.starts_with("E7001"), "{body}: {}", e.0);
    }
    assert_eq!(board.projects_api("GET", "/system-reviews/schedules", &json!(null)).unwrap(), json!([]));
}

#[test]
fn each_due_schedule_starts_its_review_once() {
    let root = crate::testing::Scratch::new();
    let daily = schedule("a", &json!({"kinds": ["security"], "repeat": "daily", "time": "09:00"}), at("2026-11-23 08:00"), &Utc).unwrap();
    let weekly = json!({"kinds": ["code-quality", "user-experience"], "repeat": "weekly", "weekday": 1, "time": "09:02"});
    let weekly = schedule("b", &weekly, at("2026-11-23 08:00"), &Utc).unwrap();
    let once = json!({"kinds": ["user-experience"], "repeat": "once", "date": "2026-11-23", "time": "09:00"});
    let once = schedule("c", &once, at("2026-11-23 08:00"), &Utc).unwrap();
    save(&root, &[daily, weekly, once]).unwrap();
    let mut started: Vec<Vec<String>> = vec![];
    let mut start = |kinds: &[String]| {
        started.push(kinds.to_vec());
        Ok(json!({}))
    };
    start_due(&root, at("2026-11-23 08:59"), &Utc, &mut start).unwrap();
    start_due(&root, at("2026-11-23 09:00"), &Utc, &mut start).unwrap();
    start_due(&root, at("2026-11-23 09:01"), &Utc, &mut start).unwrap();
    start_due(&root, at("2026-11-23 09:02"), &Utc, &mut start).unwrap();
    assert_eq!(started, [vec!["security"], vec!["user-experience"], vec!["code-quality", "user-experience"]]);
    let saved = list(&root).unwrap();
    assert_eq!(
        saved.iter().map(|s| s["next"].clone()).collect::<Vec<_>>(),
        [json!(at("2026-11-24 09:00")), json!(at("2026-11-30 09:02")), json!(null)]
    );
    assert_eq!(saved[0]["last"], json!(at("2026-11-23 09:00")));
}

#[test]
fn a_due_time_long_past_starts_nothing_and_moves_on() {
    let root = crate::testing::Scratch::new();
    let daily = schedule("a", &json!({"kinds": ["security"], "repeat": "daily", "time": "09:00"}), at("2026-11-23 08:00"), &Utc).unwrap();
    save(&root, &[daily]).unwrap();
    let mut started = 0;
    let mut start = |_: &[String]| {
        started += 1;
        Ok(json!({}))
    };
    start_due(&root, at("2026-11-23 12:00"), &Utc, &mut start).unwrap();
    assert_eq!(started, 0);
    assert_eq!(list(&root).unwrap()[0]["next"], json!(at("2026-11-24 09:00")));
}

#[test]
fn a_review_missed_while_closed_is_kept_once_until_answered() {
    let root = crate::testing::Scratch::new();
    let daily = schedule("a", &json!({"kinds": ["security"], "repeat": "daily", "time": "09:00"}), at("2026-11-23 08:00"), &Utc).unwrap();
    let weekly = json!({"kinds": ["code-quality"], "repeat": "weekly", "weekday": 1, "time": "10:00"});
    let weekly = schedule("b", &weekly, at("2026-11-23 08:00"), &Utc).unwrap();
    save(&root, &[daily, weekly]).unwrap();
    let started: std::cell::RefCell<Vec<Vec<String>>> = Default::default();
    let mut start = |kinds: &[String]| {
        started.borrow_mut().push(kinds.to_vec());
        Ok(json!({}))
    };
    // Closed from Monday 08:00 to Thursday 09:05: the daily review is due again
    // within the limit and runs; the weekly one was missed.
    start_due(&root, at("2026-11-26 09:05"), &Utc, &mut start).unwrap();
    assert_eq!(*started.borrow(), [vec!["security"]]);
    let board = board(&root);
    let notices = board.projects_api("GET", "/system-reviews/missed", &json!(null)).unwrap();
    assert_eq!(
        notices,
        json!([{"schedule": "b", "kinds": ["code-quality"], "repeat": "weekly", "due": at("2026-11-23 10:00"), "read": false}])
    );
    // Closed again until the next Tuesday: the daily review's latest due time is the one missed.
    start_due(&root, at("2026-12-01 12:00"), &Utc, &mut start).unwrap();
    assert_eq!(started.borrow().len(), 1);
    let notices = board.projects_api("GET", "/system-reviews/missed", &json!(null)).unwrap();
    assert_eq!(
        notices.as_array().unwrap().iter().map(|n| (n["schedule"].clone(), n["due"].clone())).collect::<Vec<_>>(),
        [(json!("a"), json!(at("2026-12-01 09:00"))), (json!("b"), json!(at("2026-11-30 10:00")))]
    );
    assert_eq!(list(&root).unwrap()[0]["next"], json!(at("2026-12-02 09:00")));
}

#[test]
fn a_missed_review_is_read_then_answered_through_its_routes() {
    let root = crate::testing::Scratch::new();
    let daily = json!({"kinds": ["security"], "repeat": "daily", "time": "09:00"});
    let mut a = schedule("a", &daily, at("2026-11-23 08:00"), &Utc).unwrap();
    a["missed"] = json!(at("2026-11-23 09:00"));
    a["missed_read"] = json!(false);
    let mut b = schedule("b", &daily, at("2026-11-23 08:00"), &Utc).unwrap();
    b["missed"] = json!(at("2026-11-23 09:00"));
    b["missed_read"] = json!(false);
    save(&root, &[a, b]).unwrap();
    let board = board(&root);
    let route = |method: &str, path: &str| board.projects_api(method, path, &json!({}));
    let read = route("POST", "/system-reviews/missed/read").unwrap();
    assert!(read.as_array().unwrap().iter().all(|n| n["read"] == true), "{read}");
    let left = route("POST", "/system-reviews/missed/a/skip").unwrap();
    assert_eq!(left.as_array().unwrap().iter().map(|n| n["schedule"].clone()).collect::<Vec<_>>(), [json!("b")]);
    let e = route("POST", "/system-reviews/missed/a/skip").unwrap_err();
    assert!(e.0.starts_with("E7023"), "{}", e.0);
    let e = route("POST", "/system-reviews/missed/a/run").unwrap_err();
    assert!(e.0.starts_with("E7023"), "{}", e.0);
    // The fresh read of a restarted board still holds the one left, and the schedule keeps its times.
    let again = super::tests::board(&root).projects_api("GET", "/system-reviews/missed", &json!(null)).unwrap();
    assert_eq!(again, left);
    let saved = list(&root).unwrap();
    assert!(saved[0]["next"].is_u64() && saved[0].get("missed").is_none());
}

#[test]
fn run_now_starts_one_review_and_a_failed_start_keeps_the_notice() {
    let root = crate::testing::Scratch::new();
    let mut s =
        schedule("a", &json!({"kinds": ["security", "user-experience"], "repeat": "daily", "time": "09:00"}), at("2026-11-23 08:00"), &Utc)
            .unwrap();
    s["missed"] = json!(at("2026-11-23 09:00"));
    s["missed_read"] = json!(true);
    save(&root, &[s]).unwrap();
    let mut failing = |_: &[String]| Err(Failure("E7001 no reviewer model is set up".into(), 1));
    let e = answer(&root, "a", Some(&mut failing)).unwrap_err();
    assert!(e.0.contains("no reviewer model"), "{}", e.0);
    assert_eq!(missed(&list(&root).unwrap()).as_array().unwrap().len(), 1);
    let mut started: Vec<Vec<String>> = vec![];
    let mut start = |kinds: &[String]| {
        started.push(kinds.to_vec());
        Ok(json!({}))
    };
    assert_eq!(answer(&root, "a", Some(&mut start)).unwrap(), json!([]));
    assert!(answer(&root, "a", Some(&mut start)).is_err());
    assert_eq!(started, [vec!["security", "user-experience"]]);
}

#[test]
fn a_changed_schedule_keeps_its_missed_review() {
    let root = crate::testing::Scratch::new();
    let board = board(&root);
    let daily = json!({"kinds": ["security"], "repeat": "daily", "time": "09:00"});
    let added = board.projects_api("POST", "/system-reviews/schedules", &daily).unwrap();
    let id = added[0]["id"].as_str().unwrap().to_string();
    let mut schedules = list(&root).unwrap();
    schedules[0]["missed"] = json!(at("2026-11-23 09:00"));
    schedules[0]["missed_read"] = json!(false);
    save(&root, &schedules).unwrap();
    let weekly = json!({"kinds": ["security"], "repeat": "weekly", "weekday": 2, "time": "09:00"});
    board.projects_api("PUT", &format!("/system-reviews/schedules/{id}"), &weekly).unwrap();
    let notices = board.projects_api("GET", "/system-reviews/missed", &json!(null)).unwrap();
    assert_eq!(notices[0]["due"], json!(at("2026-11-23 09:00")));
    board.projects_api("DELETE", &format!("/system-reviews/schedules/{id}"), &json!({})).unwrap();
    assert_eq!(board.projects_api("GET", "/system-reviews/missed", &json!(null)).unwrap(), json!([]));
}

/// The app was closed when a review was due: the notice is logged before any
/// page connects, and a page connecting afterwards is sent it.
#[test]
fn a_notice_logged_before_a_page_connects_is_in_its_first_state() {
    use std::io::Read;
    let root = crate::testing::Scratch::new();
    let weekly = json!({"kinds": ["code-quality"], "repeat": "weekly", "weekday": 1, "time": "10:00"});
    save(&root, &[schedule("b", &weekly, at("2026-11-23 08:00"), &Utc).unwrap()]).unwrap();
    start_due(&root, at("2026-11-26 09:05"), &Utc, &mut |_: &[String]| Ok(json!({}))).unwrap();
    let mut stream = crate::board::server::Events::new(board(&root), None, None);
    let (mut text, mut buf) = (String::new(), [0; 8192]);
    while !text.contains("event: ready") {
        let n = stream.read(&mut buf).unwrap();
        assert!(n > 0);
        text.push_str(std::str::from_utf8(&buf[..n]).unwrap());
    }
    let missed = text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str::<Value>(data).ok())
        .find_map(|event| event.get("missedReviews").cloned())
        .unwrap_or_else(|| panic!("no missedReviews in {text}"));
    assert_eq!(
        missed,
        json!([{"schedule": "b", "kinds": ["code-quality"], "repeat": "weekly", "due": at("2026-11-23 10:00"), "read": false}])
    );
}

#[test]
fn a_damaged_schedules_file_is_refused_and_left_as_it_is() {
    let root = crate::testing::Scratch::new();
    let board = board(&root);
    let route = |method: &str, path: &str, body: Value| board.projects_api(method, path, &body);
    assert_eq!(route("GET", "/system-reviews/schedules", json!(null)).unwrap(), json!([]));
    std::fs::create_dir_all(root.join(".solace")).unwrap();
    let daily = json!({"kinds": ["security"], "repeat": "daily", "time": "09:00"});
    for text in ["[{\"id\": \"a\"", "{}"] {
        std::fs::write(file(&root), text).unwrap();
        let refusals = [
            route("GET", "/system-reviews/schedules", json!(null)),
            route("POST", "/system-reviews/schedules", daily.clone()),
            route("PUT", "/system-reviews/schedules/a", daily.clone()),
            route("DELETE", "/system-reviews/schedules/a", json!({})),
            route("POST", "/system-reviews/missed/read", json!({})),
            route("POST", "/system-reviews/missed/a/skip", json!({})),
        ];
        for refusal in refusals {
            let e = refusal.unwrap_err();
            assert!(e.0.starts_with("E7030"), "{}", e.0);
        }
        let mut started = 0;
        let mut start = |_: &[String]| {
            started += 1;
            Ok(json!({}))
        };
        assert!(answer(&root, "a", Some(&mut start)).unwrap_err().0.starts_with("E7030"));
        assert!(start_due(&root, u64::MAX / 2, &Utc, &mut start).unwrap_err().0.starts_with("E7030"));
        assert_eq!(started, 0);
        assert_eq!(missed_reviews(&root), json!([]));
        assert_eq!(std::fs::read_to_string(file(&root)).unwrap(), text);
    }
}

#[test]
fn a_failed_save_starts_no_review() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let root = crate::testing::Scratch::new();
        let daily =
            schedule("a", &json!({"kinds": ["security"], "repeat": "daily", "time": "09:00"}), at("2026-11-23 08:00"), &Utc).unwrap();
        save(&root, &[daily]).unwrap();
        let folder = root.join(".solace");
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        // A user who can write anyway, such as root, can't make the save fail this way.
        let writable = std::fs::write(folder.join("probe"), "").is_ok();
        let mut started = 0;
        let mut start = |_: &[String]| {
            started += 1;
            Ok(json!({}))
        };
        let result = start_due(&root, at("2026-11-23 09:00"), &Utc, &mut start);
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        if !writable {
            assert!(result.is_err());
            assert_eq!(started, 0);
            assert_eq!(list(&root).unwrap()[0]["next"], json!(at("2026-11-23 09:00")));
        }
    }
}

#[test]
fn a_review_that_fails_to_start_is_not_retried() {
    let root = crate::testing::Scratch::new();
    let daily = schedule("a", &json!({"kinds": ["security"], "repeat": "daily", "time": "09:00"}), at("2026-11-23 08:00"), &Utc).unwrap();
    save(&root, &[daily]).unwrap();
    let mut tries = 0;
    let mut failing = |_: &[String]| {
        tries += 1;
        Err(Failure("E7001 no reviewer model is set up".into(), 1))
    };
    start_due(&root, at("2026-11-23 09:00"), &Utc, &mut failing).unwrap();
    start_due(&root, at("2026-11-23 09:01"), &Utc, &mut failing).unwrap();
    assert_eq!(tries, 1);
    let saved = list(&root).unwrap();
    assert_eq!((saved[0]["next"].clone(), saved[0]["last"].clone()), (json!(at("2026-11-24 09:00")), json!(at("2026-11-23 09:00"))));
}
