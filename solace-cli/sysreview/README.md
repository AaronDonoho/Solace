# Solace system review

The System Review page (`/system-review`), a Lustre element named
`<solace-system-review>`, with four tabs. **Run review** runs a review of the
whole workspace by hand: tick
the review kinds (Security, Code quality, User experience; all ticked at
first) and press **Run review**. The button is disabled while no kind is
ticked or a start is being sent.

A start is `POST /system-reviews` with `{"kinds": [...]}`, the kind ids
`security`, `code-quality` and `user-experience`. A started review is
confirmed under the controls; a refusal shows the server's message.

**Schedules** lists the reviews this computer runs by itself, in a table:
Kinds, Repeats (for example "Weekly, Mondays 09:00") and Next run, "None"
once a one-off time has passed. **Add schedule** opens a form: the kind
checkboxes, Repeats (Daily, Weekly, Monthly or Once on a date), a time, and
the weekday, day of the month or date the repeat needs. Clicking a row opens
the same form filled in, with **Delete**. Save is disabled while no kind is
ticked, no time or (for Once) no date is set, or a save is being sent; a
refusal shows the server's message in the form.

The list is `GET /system-reviews/schedules`, asked each time the tab is shown.
A new schedule is `POST /system-reviews/schedules`, a change is
`PUT /system-reviews/schedules/{id}` and a removal is
`DELETE /system-reviews/schedules/{id}`; each answers with every schedule, an
array of `{"id", "kinds", "repeat", "time", "weekday"?, "day"?, "date"?,
"next"}`. `repeat` is `daily`, `weekly`, `monthly` or `once`; `time` is
`HH:MM` in the computer's local time; `weekday` is 1 (Monday) to 7, `day` 1 to
31 (a shorter month runs on its last day) and `date` `YYYY-MM-DD`; `next` is in
seconds, null once a one-off time has passed. While the app is open, the
server starts each schedule's review when it comes due, as a start from the
Run review tab does.

A schedule whose latest due time passed more than ten minutes before the
server saw it, as when the app was closed, has a missed review instead: one
per schedule, its latest missed time. Missed reviews show in a yellow banner
above the tabs, "Weekly security review was due <time> and didn't run.", each
with **Run now**, which starts that review, and **Skip**, which drops it; the
schedule keeps its next time either way. The same banner shows on Delivery
until the notice is read. A notice is read once this page shows (the shell
sets its `shown` attribute to `true`); while the page is hidden, an unread
notice flags its menu tab.

The list is `GET /system-reviews/missed`, and the `missedReviews` event
carries it on every change: an array of `{"schedule", "kinds", "repeat",
"due", "read"}`, `due` in seconds. `POST /system-reviews/missed/read` marks
them all read, and `POST /system-reviews/missed/{schedule}/run` or
`/skip` answers one; each answers with those left. Answering one already
answered is refused, and a review that can't start keeps its notice.

**Past reviews** lists the reviews that completed on this computer, newest
first, in a table: Started, Ended, Kinds, Version and Findings (how many).
Clicking a row opens a dialog with the same details and a link to each finding
task. The list is `GET /system-reviews`, asked each time the tab is shown: an
array of `{"id", "started", "ended", "kinds", "version", "findings"}`, times
in seconds, `version` null when none was named.

**Log** shows `SYSTEMREVIEWS.md`, the log only Solace writes (D-265), as
Markdown, each `task:<id>` a link. It is `GET /system-reviews/log`, asked each
time the tab is shown: `{"text"}`; a refusal shows the server's message.

## Tests

```sh
gleam test --target javascript
```

The Gleam tests cover the kind choice, the disabled button, the request body
and both replies, the schedules table, its form for adding, changing and
deleting a schedule and its disabled Save, each repeat's request body, the
past reviews table, its dialog and its empty state, and the log as Markdown
and its refusal, and the missed-review banner with its refusal.
