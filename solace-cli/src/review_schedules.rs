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
#[cfg(test)]
#[path = "review_schedules_tests.rs"]
mod tests;

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

/// Every schedule, in the order they were added. A missing file is none; a
/// damaged one is a failure, so a save never replaces it with fewer schedules.
fn list(top: &Path) -> Result<Vec<Value>, Failure> {
    let path = file(top);
    if !path.exists() {
        return Ok(Vec::new());
    }
    crate::jsonx::read(&path).ok().and_then(|v| v.as_array().cloned()).ok_or_else(damaged)
}

fn save(top: &Path, schedules: &[Value]) -> Result<(), Failure> {
    std::fs::create_dir_all(top.join(".solace"))?;
    crate::jsonx::write_pretty(&file(top), &json!(schedules))?;
    // Pages already listening hear the list the notices now make; a page that connects later
    // gets it in its stream's first state (`missed_reviews`).
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
    let mut schedules = list(top)?;
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
/// The schedules are saved before any review starts, so a failed save starts none
/// and a due time never starts its review twice.
fn start_due<Tz: TimeZone>(
    top: &Path,
    now: u64,
    tz: &Tz,
    start: &mut dyn FnMut(&[String]) -> Result<Value, Failure>,
) -> Result<(), Failure> {
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let mut schedules = list(top)?;
    let mut due_kinds = Vec::new();
    let mut changed = false;
    for s in schedules.iter_mut() {
        let Some(due) = s["next"].as_u64().filter(|due| *due <= now) else { continue };
        let mut latest = due;
        while let Some(later) = next_after(s, latest, tz).filter(|later| *later <= now) {
            latest = later;
        }
        if now - latest <= LATE_LIMIT {
            due_kinds.push((s["id"].clone(), kinds(s)));
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
    for (id, kinds) in due_kinds {
        // A review that can't start, such as with no reviewer model set up, is not retried.
        if let Err(e) = start(&kinds) {
            eprintln!("scheduled system review {id}: {}", e.0);
        }
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
        ("GET", "") => Ok(missed_reviews(top)),
        ("POST", "read") => {
            let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
            let mut schedules = list(top)?;
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
    let mut schedules = list(top)?;
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

/// The missed reviews not yet answered, as `GET /system-reviews/missed` lists
/// them; a damaged schedules file has none, and its routes say why.
pub(crate) fn missed_reviews(top: &Path) -> Value {
    list(top).map(|schedules| missed(&schedules)).unwrap_or_else(|_| json!([]))
}

fn damaged() -> Failure {
    Failure("E7030 the review schedules file is unreadable; it was left as it is".into(), 1)
}
