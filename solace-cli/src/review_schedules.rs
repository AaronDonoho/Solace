//! Scheduled system reviews: the schedules a human sets up on the System
//! Review page, each starting a review of its kinds when it comes due. They
//! belong to this computer, kept under `.solace/` and never shared (D-261),
//! and run while the board is served.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use chrono::{Datelike, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, TimeZone};
use serde_json::{Value, json};

use crate::failure::Failure;

const FILE: &str = ".solace/review-schedules.json";

/// How long after its due time a schedule may still start its review; a later
/// one was missed, as when the app was closed.
const LATE_LIMIT: u64 = 600;

/// Held while the schedules are read and written back, so a page's change
/// and a due review never overwrite each other.
static WRITING: Mutex<()> = Mutex::new(());

fn file(top: &Path) -> PathBuf {
    top.join(FILE)
}

/// Every schedule, in the order they were added.
fn list(top: &Path) -> Vec<Value> {
    crate::jsonx::read_opt(&file(top)).and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

fn save(top: &Path, schedules: &[Value]) -> Result<(), Failure> {
    std::fs::create_dir_all(top.join(".solace"))?;
    crate::jsonx::write_pretty(&file(top), &json!(schedules))?;
    // Each page showing the notices hears the list they now make, and a page that connects later is given it.
    crate::tasks::store::log(top, "", &json!({"missedReviews": missed(schedules)}));
    Ok(())
}

fn refused(message: &str) -> Failure {
    Failure(format!("E7001 {message}"), 1)
}

/// `/system-reviews/schedules`, and `/system-reviews/schedules/{id}` to change or
/// remove one: each answers with every schedule.
pub(crate) fn api(top: &Path, method: &str, path: &str, body: &Value) -> Result<Value, Failure> {
    let id = path.strip_prefix("/system-reviews/schedules").unwrap_or("").trim_start_matches('/');
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let mut schedules = list(top);
    let now = crate::now();
    match (method, id) {
        ("GET", "") => return Ok(json!(schedules)),
        ("POST", "") => schedules.push(schedule(&crate::unique_id(), body, now, &chrono::Local)?),
        ("PUT", id) if !id.is_empty() => {
            let at = position(&schedules, id)?;
            let mut changed = schedule(id, body, now, &chrono::Local)?;
            for key in ["missed", "missed_read"] {
                if let Some(kept) = schedules[at].get(key) {
                    changed[key] = kept.clone();
                }
            }
            schedules[at] = changed;
        }
        ("DELETE", id) if !id.is_empty() => {
            let at = position(&schedules, id)?;
            schedules.remove(at);
        }
        _ => return Err(refused("unknown route")),
    }
    save(top, &schedules)?;
    Ok(json!(schedules))
}

fn position(schedules: &[Value], id: &str) -> Result<usize, Failure> {
    schedules.iter().position(|s| s["id"] == id).ok_or_else(|| Failure(format!("E7023 unknown schedule {id}"), 1))
}

/// The schedule `body` describes, checked, as `id`, with the first time it comes
/// due after `now`: its review kinds, `repeat` (daily, weekly, monthly or once),
/// `time` (HH:MM), and `weekday` (1 for Monday to 7), `day` (of the month) or
/// `date` (YYYY-MM-DD) as `repeat` needs.
fn schedule<Tz: TimeZone>(id: &str, body: &Value, now: u64, tz: &Tz) -> Result<Value, Failure> {
    let kinds = crate::system_reviews::requested_kinds(body)?;
    let time = body["time"]
        .as_str()
        .and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok())
        .ok_or_else(|| refused("a schedule needs `time`, a time of day as HH:MM"))?;
    let mut s = json!({"id": id, "kinds": kinds, "repeat": body["repeat"], "time": time.format("%H:%M").to_string()});
    match body["repeat"].as_str() {
        Some("daily") => {}
        Some("weekly") => s["weekday"] = json!(number(body, "weekday", 7)?),
        Some("monthly") => s["day"] = json!(number(body, "day", 31)?),
        Some("once") => {
            let date = body["date"]
                .as_str()
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                .ok_or_else(|| refused("a schedule that runs once needs `date`, as YYYY-MM-DD"))?;
            s["date"] = json!(date.format("%Y-%m-%d").to_string());
        }
        _ => return Err(refused("a schedule's `repeat` is one of: daily, weekly, monthly, once")),
    }
    let Some(next) = next_after(&s, now, tz) else {
        return Err(refused("that date and time has passed; choose a later one"));
    };
    s["next"] = json!(next);
    Ok(s)
}

/// `body[key]`, a whole number from 1 to `most`.
fn number(body: &Value, key: &str, most: u64) -> Result<u64, Failure> {
    body[key]
        .as_u64()
        .filter(|n| (1..=most).contains(n))
        .ok_or_else(|| refused(&format!("`{key}` must be a whole number from 1 to {most}")))
}

/// The first time after `after` that schedule `s` comes due in `tz`, in seconds;
/// none for a date that has passed.
fn next_after<Tz: TimeZone>(s: &Value, after: u64, tz: &Tz) -> Option<u64> {
    let time = NaiveTime::parse_from_str(s["time"].as_str()?, "%H:%M").ok()?;
    let today = tz.timestamp_opt(i64::try_from(after).ok()?, 0).single()?.date_naive();
    let at = |date: NaiveDate| instant(date.and_time(time), tz).filter(|t| *t > after);
    match s["repeat"].as_str()? {
        "once" => at(NaiveDate::parse_from_str(s["date"].as_str()?, "%Y-%m-%d").ok()?),
        "daily" => [0, 1].into_iter().find_map(|n| at(today + TimeDelta::days(n))),
        "weekly" => {
            let weekday = i64::try_from(s["weekday"].as_u64()?).ok()?;
            let ahead = (weekday - i64::from(today.weekday().number_from_monday())).rem_euclid(7);
            [ahead, ahead + 7].into_iter().find_map(|n| at(today + TimeDelta::days(n)))
        }
        "monthly" => {
            let day = u32::try_from(s["day"].as_u64()?).ok()?;
            [0, 1].into_iter().find_map(|n| at(day_in_month(today, n, day)?))
        }
        _ => None,
    }
}

/// Day `day` of the month `months` after `date`'s, or that month's last day when it has fewer.
fn day_in_month(date: NaiveDate, months: u32, day: u32) -> Option<NaiveDate> {
    let first = date.with_day(1)?.checked_add_months(Months::new(months))?;
    let last = first.checked_add_months(Months::new(1))?.pred_opt()?.day();
    first.with_day(day.min(last))
}

/// The time `local` names in `tz`, in seconds: the first of a repeated hour, and
/// an hour on when the clocks skip it.
fn instant<Tz: TimeZone>(local: NaiveDateTime, tz: &Tz) -> Option<u64> {
    tz.from_local_datetime(&local)
        .earliest()
        .or_else(|| tz.from_local_datetime(&(local + TimeDelta::hours(1))).earliest())
        .and_then(|t| u64::try_from(t.timestamp()).ok())
}

/// Starts, with `start`, the review of each schedule due by `now`, and moves each
/// on to its next due time. Only the latest due time passed counts: one more than
/// `LATE_LIMIT` past was missed and starts nothing, and `missed` keeps it, unread,
/// until the human answers it; `last` keeps the due time whose review started.
fn start_due<Tz: TimeZone>(
    top: &Path,
    now: u64,
    tz: &Tz,
    start: &mut dyn FnMut(&[String]) -> Result<Value, Failure>,
) -> Result<(), Failure> {
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let mut schedules = list(top);
    let mut changed = false;
    for s in schedules.iter_mut() {
        let Some(due) = s["next"].as_u64().filter(|due| *due <= now) else { continue };
        let mut latest = due;
        while let Some(later) = next_after(s, latest, tz).filter(|later| *later <= now) {
            latest = later;
        }
        if now - latest <= LATE_LIMIT {
            // A review that can't start, such as with no reviewer model set up, is not retried.
            if let Err(e) = start(&kinds(s)) {
                eprintln!("scheduled system review {}: {}", s["id"], e.0);
            }
            s["last"] = json!(latest);
        } else {
            s["missed"] = json!(latest);
            s["missed_read"] = json!(false);
        }
        s["next"] = json!(next_after(s, now, tz));
        changed = true;
    }
    if changed {
        save(top, &schedules)?;
    }
    Ok(())
}

/// Starts each schedule's reviews as they come due, for as long as the board is served.
pub(crate) fn watch(top: PathBuf) {
    std::thread::spawn(move || {
        loop {
            let mut start = |kinds: &[String]| crate::system_reviews::start(&top, &json!({"kinds": kinds}));
            if let Err(e) = start_due(&top, crate::now(), &chrono::Local, &mut start) {
                eprintln!("system review schedules: {}", e.0);
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Utc};

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
        let daily =
            schedule("a", &json!({"kinds": ["security"], "repeat": "daily", "time": "09:00"}), at("2026-11-23 08:00"), &Utc).unwrap();
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
        let saved = list(&root);
        assert_eq!(
            saved.iter().map(|s| s["next"].clone()).collect::<Vec<_>>(),
            [json!(at("2026-11-24 09:00")), json!(at("2026-11-30 09:02")), json!(null)]
        );
        assert_eq!(saved[0]["last"], json!(at("2026-11-23 09:00")));
    }

    #[test]
    fn a_due_time_long_past_starts_nothing_and_moves_on() {
        let root = crate::testing::Scratch::new();
        let daily =
            schedule("a", &json!({"kinds": ["security"], "repeat": "daily", "time": "09:00"}), at("2026-11-23 08:00"), &Utc).unwrap();
        save(&root, &[daily]).unwrap();
        let mut started = 0;
        let mut start = |_: &[String]| {
            started += 1;
            Ok(json!({}))
        };
        start_due(&root, at("2026-11-23 12:00"), &Utc, &mut start).unwrap();
        assert_eq!(started, 0);
        assert_eq!(list(&root)[0]["next"], json!(at("2026-11-24 09:00")));
    }

    #[test]
    fn a_review_missed_while_closed_is_kept_once_until_answered() {
        let root = crate::testing::Scratch::new();
        let daily =
            schedule("a", &json!({"kinds": ["security"], "repeat": "daily", "time": "09:00"}), at("2026-11-23 08:00"), &Utc).unwrap();
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
        assert_eq!(list(&root)[0]["next"], json!(at("2026-12-02 09:00")));
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
        assert!(list(&root)[0]["next"].is_u64() && list(&root)[0].get("missed").is_none());
    }

    #[test]
    fn run_now_starts_one_review_and_a_failed_start_keeps_the_notice() {
        let root = crate::testing::Scratch::new();
        let mut s = schedule(
            "a",
            &json!({"kinds": ["security", "user-experience"], "repeat": "daily", "time": "09:00"}),
            at("2026-11-23 08:00"),
            &Utc,
        )
        .unwrap();
        s["missed"] = json!(at("2026-11-23 09:00"));
        s["missed_read"] = json!(true);
        save(&root, &[s]).unwrap();
        let mut failing = |_: &[String]| Err(Failure("E7001 no reviewer model is set up".into(), 1));
        let e = answer(&root, "a", Some(&mut failing)).unwrap_err();
        assert!(e.0.contains("no reviewer model"), "{}", e.0);
        assert_eq!(missed(&list(&root)).as_array().unwrap().len(), 1);
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
        let mut schedules = list(&root);
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
}

/// The ids of schedule `s`'s review kinds.
fn kinds(s: &Value) -> Vec<String> {
    s["kinds"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect()
}

/// The missed reviews not yet answered, one per schedule: its id, kinds and
/// repeat, the latest due time it missed, and whether the human has read it.
fn missed(schedules: &[Value]) -> Value {
    let notices = schedules.iter().filter(|s| s["missed"].is_u64()).map(
        |s| json!({"schedule": s["id"], "kinds": s["kinds"], "repeat": s["repeat"], "due": s["missed"], "read": s["missed_read"] == true}),
    );
    json!(notices.collect::<Vec<_>>())
}

/// `/system-reviews/missed`: the missed reviews; `/read` marks them all read, and
/// `/{schedule}/run` or `/{schedule}/skip` answers one. Each answers with the
/// missed reviews left.
pub(crate) fn missed_api(top: &Path, method: &str, path: &str) -> Result<Value, Failure> {
    let rest = path.strip_prefix("/system-reviews/missed").unwrap_or("").trim_start_matches('/');
    let mut start = |kinds: &[String]| crate::system_reviews::start(top, &json!({"kinds": kinds}));
    match (method, rest) {
        ("GET", "") => Ok(missed(&list(top))),
        ("POST", "read") => {
            let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
            let mut schedules = list(top);
            for s in schedules.iter_mut().filter(|s| s["missed"].is_u64()) {
                s["missed_read"] = json!(true);
            }
            save(top, &schedules)?;
            Ok(missed(&schedules))
        }
        ("POST", rest) => match rest.split_once('/') {
            Some((id, "run")) => answer(top, id, Some(&mut start)),
            Some((id, "skip")) => answer(top, id, None),
            _ => Err(refused("unknown route")),
        },
        _ => Err(refused("unknown route")),
    }
}

/// Answers schedule `id`'s missed review: with `start`, its review starts first,
/// and a review that can't start leaves the notice; without, it is skipped.
fn answer(top: &Path, id: &str, start: Option<&mut dyn FnMut(&[String]) -> Result<Value, Failure>>) -> Result<Value, Failure> {
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let mut schedules = list(top);
    let at = schedules
        .iter()
        .position(|s| s["id"] == id && s["missed"].is_u64())
        .ok_or_else(|| Failure(format!("E7023 no missed review for schedule {id}"), 1))?;
    if let Some(start) = start {
        start(&kinds(&schedules[at]))?;
    }
    let s = &mut schedules[at];
    if let Some(fields) = s.as_object_mut() {
        fields.remove("missed");
        fields.remove("missed_read");
    }
    save(top, &schedules)?;
    Ok(missed(&schedules))
}
