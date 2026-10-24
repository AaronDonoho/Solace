import features
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{None, Some}
import gleam/result
import gleam/string
import gleeunit
import lustre/element
import plan
import roadmap
import solace_task
import strategy

pub fn main() -> Nil {
  gleeunit.main()
}

pub fn tab_from_url_test() {
  assert strategy.tab_from("?tab=tasks") == strategy.Tasks
  assert strategy.tab_from("x=1&tab=roadmap") == strategy.Roadmap
  assert strategy.tab_from("") == strategy.Vision
  assert strategy.tab_from("?tab=nope") == strategy.Vision
}

pub fn tab_names_round_trip_test() {
  list.each(strategy.tabs(), fn(tab) {
    assert strategy.tab_from("?tab=" <> strategy.tab_name(tab)) == tab
  })
}

pub fn one_tab_is_selected_test() {
  let html = element.to_string(strategy.view(strategy.new(strategy.Tasks)))
  assert list.length(string.split(html, "aria-selected=\"true\"")) == 2
  assert string.contains(html, "id=\"tab-features\"")
  assert list.length(string.split(html, " hidden")) == 4
}

fn step(model: strategy.Model, msg: strategy.Msg) -> strategy.Model {
  strategy.update(model, msg).0
}

fn task(
  id: String,
  title: String,
  body: String,
  column: String,
  feature: String,
  closed: Int,
) -> json.Json {
  json.object([
    #("id", json.string(id)),
    #("title", json.string(title)),
    #("body", json.string(body)),
    #("column", json.string(column)),
    #("feature", case feature {
      "" -> json.null()
      f -> json.string(f)
    }),
    #("milestone", json.null()),
    #("dependsOn", json.array(["t0"], json.string)),
    #("createdTime", json.int(60)),
    #("closedTime", case closed {
      0 -> json.null()
      c -> json.int(c)
    }),
  ])
}

fn message(project: String, fields: List(#(String, json.Json))) -> String {
  json.to_string(json.object([#("project", json.string(project)), ..fields]))
}

/// A workspace with the root's project and `web`, caught up.
fn workspace() -> strategy.Model {
  strategy.new(strategy.Tasks)
  |> step(strategy.ProjectsLoaded(
    "{\"workspace\":true,\"projects\":[{\"dir\":\".\",\"label\":\"root\",\"name\":\"r\",\"base\":\"\"},{\"dir\":\"web\",\"label\":\"web\",\"name\":\"web\",\"base\":\"/p/web\"}]}",
  ))
  |> step(strategy.CaughtUp)
}

fn ids(model: strategy.Model) -> List(String) {
  list.map(strategy.visible(model), fn(pair) { { pair.1 }.id })
}

pub fn row_decoder_reads_links_and_closed_time_test() {
  let assert Ok(row) =
    json.parse(
      json.to_string(task("t1", "A", "b", "done", "f1", 70)),
      strategy.row_decoder(),
    )
  assert row
    == strategy.Row(
      "t1",
      "A",
      "b",
      "done",
      Some("f1"),
      None,
      Some(60),
      Some(70),
      "",
      "",
      [],
      ["t0"],
    )
  // A task saved before D-87 has none of the fields.
  let assert Ok(old) =
    json.parse(
      "{\"id\":\"t2\",\"title\":\"Old\",\"body\":\"\",\"column\":\"backlog\"}",
      strategy.row_decoder(),
    )
  assert #(
      old.feature,
      old.milestone,
      old.created,
      old.closed,
      old.paused,
      old.finished,
      old.runs,
      old.depends,
    )
    == #(None, None, None, None, "", "", [], [])
  // The server sends null for a field it has no value for.
  let assert Ok(nulls) =
    json.parse(
      "{\"id\":\"t3\",\"title\":\"Nulls\",\"body\":\"\",\"column\":\"backlog\",\"paused_from\":null,\"finished\":null,\"runs\":[]}",
      strategy.row_decoder(),
    )
  assert #(nulls.paused, nulls.finished, nulls.runs) == #("", "", [])
}

pub fn closed_tasks_come_newest_first_then_open_ones_test() {
  let row = fn(id, title, closed) {
    #(
      ".",
      strategy.Row(
        id,
        title,
        "",
        "done",
        None,
        None,
        None,
        closed,
        "",
        "",
        [],
        [],
      ),
    )
  }
  let order =
    strategy.sorted([
      row("a", "Zeta", None),
      row("b", "Old", Some(10)),
      row("c", "Alpha", None),
      row("d", "New", Some(30)),
    ])
    |> list.map(fn(pair) { { pair.1 }.id })
  assert order == ["d", "b", "c", "a"]
}

pub fn filters_keep_column_project_links_and_words_test() {
  let row =
    strategy.Row(
      "t1",
      "Fix checkout",
      "The Tax rate is wrong",
      "done",
      Some("f1"),
      None,
      Some(1),
      Some(5),
      "",
      "",
      [],
      [],
    )
  let all = strategy.Filters("", "", "", "", "")
  assert strategy.keeps(all, "web", row)
  assert strategy.keeps(strategy.Filters(..all, column: "done"), "web", row)
  assert !strategy.keeps(strategy.Filters(..all, column: "backlog"), "web", row)
  assert strategy.keeps(strategy.Filters(..all, project: "web"), "web", row)
  assert !strategy.keeps(strategy.Filters(..all, project: "."), "web", row)
  assert strategy.keeps(strategy.Filters(..all, feature: "f1"), "web", row)
  assert !strategy.keeps(strategy.Filters(..all, feature: "f2"), "web", row)
  assert !strategy.keeps(strategy.Filters(..all, feature: "-"), "web", row)
  assert strategy.keeps(strategy.Filters(..all, milestone: "-"), "web", row)
  assert !strategy.keeps(strategy.Filters(..all, milestone: "m1"), "web", row)
  // Every word, in the title or the body, any case.
  assert strategy.keeps(
    strategy.Filters(..all, query: "checkout tax"),
    "web",
    row,
  )
  assert !strategy.keeps(
    strategy.Filters(..all, query: "checkout refund"),
    "web",
    row,
  )
  // A word that is the task's id, bare, as task:<id>, in any case, or
  // beside a word in the title.
  assert strategy.keeps(strategy.Filters(..all, query: "t1"), "web", row)
  assert strategy.keeps(strategy.Filters(..all, query: "task:t1"), "web", row)
  assert strategy.keeps(strategy.Filters(..all, query: "TASK:T1"), "web", row)
  assert strategy.keeps(
    strategy.Filters(..all, query: "checkout t1"),
    "web",
    row,
  )
  // Only the whole id: a part of it, another task's id, and `task:` alone
  // name no task.
  assert !strategy.keeps(strategy.Filters(..all, query: "1"), "web", row)
  assert !strategy.keeps(strategy.Filters(..all, query: "task:t2"), "web", row)
  assert !strategy.keeps(strategy.Filters(..all, query: "task:"), "web", row)
}

pub fn live_messages_merge_every_project_test() {
  let model =
    workspace()
    |> step(
      strategy.Live(
        message(".", [
          #(
            "tasks",
            json.array([task("t1", "Root", "", "done", "", 10)], fn(t) { t }),
          ),
        ]),
      ),
    )
    |> step(
      strategy.Live(
        message("web", [
          #(
            "tasks",
            json.array(
              [
                task("t2", "Web", "", "done", "", 20),
                task("t3", "Open", "", "backlog", "", 0),
              ],
              fn(t) { t },
            ),
          ),
        ]),
      ),
    )
  assert ids(model) == ["t2", "t1", "t3"]
  // An update replaces the task in its own project.
  let model =
    step(
      model,
      strategy.Live(
        message("web", [#("task", task("t3", "Open", "", "done", "", 30))]),
      ),
    )
  assert ids(model) == ["t3", "t2", "t1"]
  let model =
    step(model, strategy.Live(message(".", [#("removed", json.string("t1"))])))
  assert ids(model) == ["t3", "t2"]
}

pub fn features_and_milestones_follow_the_stream_test() {
  let model =
    workspace()
    |> step(strategy.Live(
      "{\"features\":[{\"id\":\"f1\",\"title\":\"Search\"}]}",
    ))
    |> step(strategy.Live(
      "{\"milestones\":[{\"id\":\"m1\",\"title\":\"Launch\"}]}",
    ))
    |> step(strategy.Live("{\"feature\":{\"id\":\"f2\",\"title\":\"Export\"}}"))
    |> step(strategy.Live(
      "{\"milestone\":{\"id\":\"m1\",\"title\":\"Launch 1\",\"body\":\"b\",\"targetDate\":\"2027-01-01\",\"state\":\"active\"}}",
    ))
    |> step(strategy.Live("{\"removedFeature\":\"f1\"}"))
  assert model.features
    == [features.Feature("f2", "Export", "", None, 0.0, "proposed", None)]
  assert model.milestones
    == [
      features.Milestone(
        "m1",
        "Launch 1",
        0.0,
        "b",
        Some("2027-01-01"),
        "active",
      ),
    ]
}

pub fn replay_waits_for_ready_and_the_project_list_test() {
  let snapshot =
    message("web", [
      #(
        "tasks",
        json.array([task("t1", "Web", "", "done", "", 1)], fn(t) { t }),
      ),
    ])
  let model =
    strategy.new(strategy.Tasks)
    |> step(strategy.Live(snapshot))
    |> step(strategy.CaughtUp)
  // Not listed yet: held.
  assert ids(model) == []
  let model =
    step(
      model,
      strategy.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\"web\",\"label\":\"web\",\"name\":\"web\",\"base\":\"/p/web\"}]}",
      ),
    )
  assert ids(model) == ["t1"]
  assert model.replay == []
}

pub fn the_open_task_gets_its_own_messages_test() {
  let model =
    workspace()
    |> step(strategy.Open("web", "t1"))
    |> step(
      strategy.Live(
        message("web", [#("task", task("t1", "A", "", "done", "", 1))]),
      ),
    )
    |> step(
      strategy.Live(
        message(".", [#("task", task("t1", "B", "", "done", "", 1))]),
      ),
    )
    |> step(
      strategy.Live(
        message("web", [#("task", task("t2", "C", "", "done", "", 1))]),
      ),
    )
    |> step(
      strategy.Live(
        message("web", [
          #(
            "workflow",
            json.object([#("columns", json.array([], fn(c) { c }))]),
          ),
        ]),
      ),
    )
  // Its update under another project counts too (`<solace-task>` fetches
  // itself again on it), and so does its home's workflow; another task's
  // update doesn't.
  assert list.map(model.live, fn(m) { m.0 }) == [1, 2, 3]
  let html = element.to_string(strategy.view(model))
  assert string.contains(html, "<solace-task")
  assert string.contains(html, "base=\"/p/web\"")
  assert string.contains(html, "task=\"t1\"")
}

pub fn an_inline_link_patches_one_field_test() {
  assert json.to_string(strategy.link_body("feature", "f1"))
    == "{\"feature\":\"f1\"}"
  assert json.to_string(strategy.link_body("milestone", ""))
    == "{\"milestone\":null}"
  // The reply is the task as it is now.
  let model =
    workspace()
    |> step(strategy.Patched(
      "web",
      "t1",
      json.to_string(task("t1", "A", "", "done", "f1", 3)),
    ))
  let assert [#("web", row)] = strategy.visible(model)
  assert row.feature == Some("f1")
}

pub fn the_vision_follows_the_stream_and_a_draft_keeps_its_base_test() {
  let model =
    workspace()
    |> step(strategy.Live(
      "{\"vision\":{\"text\":\"# Why\\nFor teams.\",\"revision\":\"r1\"}}",
    ))
  assert model.vision == strategy.Document("# Why\nFor teams.", "r1")
  let html =
    element.to_string(strategy.view(
      strategy.Model(..model, tab: strategy.Vision),
    ))
  // Headings render at one size, as in task notes.
  assert string.contains(html, "<h4>Why</h4><p>For teams.</p>")
  // An edit begins from the revision shown; another save meanwhile leaves the
  // draft as typed, and the page offers Reload.
  let model =
    model
    |> step(strategy.EditVision)
    |> step(strategy.Draft("Mine"))
    |> step(strategy.Live(
      "{\"vision\":{\"text\":\"Theirs\",\"revision\":\"r2\"}}",
    ))
  assert model.draft == Some(#("Mine", "r1"))
  let html =
    element.to_string(strategy.view(
      strategy.Model(..model, tab: strategy.Vision),
    ))
  assert string.contains(html, "changed since you began editing")
  assert string.contains(html, ">Reload<")
  let model = step(model, strategy.ReloadVision)
  assert model.draft == Some(#("Theirs", "r2"))
}

pub fn a_vision_save_names_its_base_and_its_reply_ends_the_edit_test() {
  assert json.to_string(strategy.vision_body("Text", "r1"))
    == "{\"text\":\"Text\",\"base\":\"r1\"}"
  let editing =
    workspace()
    |> step(strategy.Live("{\"vision\":{\"text\":\"\",\"revision\":\"r0\"}}"))
    |> step(strategy.EditVision)
    |> step(strategy.Draft("New"))
  // A refusal keeps the draft and says why.
  let refused =
    step(
      editing,
      strategy.VisionSaved(
        "{\"code\":\"E7026\",\"message\":\"the vision changed since this edit began; reload it\"}",
      ),
    )
  assert refused.draft == Some(#("New", "r0"))
  assert refused.vision_error
    == "the vision changed since this edit began; reload it"
  let saved =
    step(
      editing,
      strategy.VisionSaved("{\"text\":\"New\",\"revision\":\"r1\"}"),
    )
  assert saved.draft == None
  assert saved.vision == strategy.Document("New", "r1")
}

pub fn the_vision_count_is_in_code_points_and_holds_save_past_the_limit_test() {
  // As Rust's chars().count(): "é" is one, and so is each code point of an emoji.
  assert strategy.count_text(string.repeat("é", 973))
    == "973 / 1000 character limit"
  assert strategy.vision_count("👍🏽") == 2
  let editing =
    workspace()
    |> step(strategy.Live("{\"vision\":{\"text\":\"\",\"revision\":\"r0\"}}"))
    |> step(strategy.EditVision)
  let html = fn(model) {
    element.to_string(strategy.view(
      strategy.Model(..model, tab: strategy.Vision),
    ))
  }
  let at_limit = step(editing, strategy.Draft(string.repeat("a", 1000)))
  assert string.contains(html(at_limit), "1000 / 1000 character limit")
  assert !string.contains(
    html(at_limit),
    "<button disabled type=\"button\">Save",
  )
  assert step(at_limit, strategy.SaveVision).saving
  // Past it the count is an alert, Save is disabled, and a save sends nothing.
  let over = step(editing, strategy.Draft(string.repeat("a", 1001)))
  assert string.contains(
    html(over),
    "<p class=\"vision-count\" role=\"alert\">1001 / 1000 character limit</p>",
  )
  assert string.contains(html(over), "<button disabled type=\"button\">Save")
  assert !step(over, strategy.SaveVision).saving
}

pub fn open_from_the_features_tab_shows_the_task_on_the_tasks_tab_test() {
  let model =
    strategy.new(strategy.Features)
    |> step(strategy.Open("web", "t1"))
  assert model.tab == strategy.Tasks
  assert model.selected == Some(#("web", "t1"))
}

pub fn a_feature_save_refused_keeps_the_draft_and_a_saved_one_ends_the_edit_test() {
  let editing =
    workspace()
    |> step(strategy.Live(
      "{\"features\":[{\"id\":\"f1\",\"title\":\"Search\"}]}",
    ))
    |> step(strategy.EditFeature("f1"))
    |> step(strategy.FeatureMilestone("m9"))
  assert editing.feature_draft
    == Some(features.Draft(Some("f1"), "Search", "", "proposed", Some("m9")))
  // A refusal keeps the draft and says why.
  let refused =
    step(
      editing,
      strategy.FeatureSaved(
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown milestone m9\"}",
      ),
    )
  assert refused.feature_draft == editing.feature_draft
  assert refused.feature_error == "E7023 unknown milestone m9"
  let saved =
    step(
      editing,
      strategy.FeatureSaved(
        "{\"id\":\"f1\",\"title\":\"Search\",\"state\":\"active\"}",
      ),
    )
  assert saved.feature_draft == None
  assert saved.feature_error == ""
  assert saved.features
    == [features.Feature("f1", "Search", "", None, 0.0, "active", None)]
  // A new feature's form starts proposed, with no milestone, and a request id.
  let new = step(workspace(), strategy.NewFeature)
  assert new.feature_draft == Some(features.new_draft())
  assert new.feature_request != ""
}

pub fn a_feature_removed_while_confirming_frees_the_list_test() {
  let model =
    workspace()
    |> step(strategy.Live(
      "{\"features\":[{\"id\":\"f1\",\"title\":\"Search\"},{\"id\":\"f2\",\"title\":\"Export\"}]}",
    ))
    |> step(strategy.Select(strategy.Features))
    |> step(strategy.DeleteFeature("f1"))
  let html = fn(m) { element.to_string(strategy.view(m)) }
  assert string.contains(html(model), "alertdialog")
  let model = step(model, strategy.Live("{\"removedFeature\":\"f1\"}"))
  assert !string.contains(html(model), "alertdialog")
  assert string.contains(
    html(model),
    "<button aria-label=\"Delete Export\" class=\"edit\" type=\"button\">",
  )
}

pub fn a_new_task_reports_each_step_test() {
  // A blank title sends nothing.
  let model = workspace() |> step(strategy.TaskTitle("f1", "  "))
  assert step(model, strategy.AddTask("f1")).task_saving == None
  // A refused create keeps the title and says which step failed.
  let model =
    model
    |> step(strategy.TaskTitle("f1", "Index"))
    |> step(strategy.TaskCreated(
      "web",
      "f1",
      "{\"code\":\"E7001\",\"message\":\"E7001 no title\"}",
    ))
  assert model.task_errors
    == [#("f1", "Couldn't create the task: E7001 no title")]
  assert model.task_titles == [#("f1", "Index")]
  // A create clears the form and sends the link; a refused link says the
  // task exists without it.
  let model =
    model
    |> step(strategy.TaskCreated(
      "web",
      "f1",
      json.to_string(task("t9", "Index", "", "backlog", "", 0)),
    ))
  assert model.task_linking == Some(#("web", "t9", "f1"))
  assert model.task_titles == []
  assert model.task_errors == []
  let model =
    model
    |> step(strategy.Patched(
      "web",
      "t9",
      "{\"code\":\"E7023\",\"message\":\"E7023 unknown feature f1\"}",
    ))
  assert model.task_errors
    == [
      #(
        "f1",
        "The task was created without the link to this feature: E7023 unknown feature f1",
      ),
    ]
  assert model.task_linking == None
  assert model.task_saving == None
  assert model.error == ""
}

pub fn a_milestone_save_refused_keeps_the_draft_and_a_saved_one_ends_the_edit_test() {
  let editing =
    workspace()
    |> step(strategy.Live(
      "{\"milestones\":[{\"id\":\"m1\",\"title\":\"1.0\",\"targetDate\":\"2027-01-01\"}]}",
    ))
    |> step(strategy.EditMilestone("m1"))
    |> step(strategy.MilestoneTargetDate(""))
    |> step(strategy.MilestoneState("active"))
  assert editing.milestone_draft
    == Some(roadmap.Draft(Some("m1"), "1.0", "", "active", None))
  let refused =
    step(
      editing,
      strategy.MilestoneSaved(
        "{\"code\":\"E7001\",\"message\":\"E7001 title is required\"}",
      ),
    )
  assert refused.milestone_draft == editing.milestone_draft
  assert refused.milestone_error == "E7001 title is required"
  let saved =
    step(
      editing,
      strategy.MilestoneSaved(
        "{\"id\":\"m1\",\"title\":\"1.0\",\"rank\":1,\"state\":\"active\"}",
      ),
    )
  assert saved.milestone_draft == None
  assert saved.milestones
    == [features.Milestone("m1", "1.0", 1.0, "", None, "active")]
  let new = step(workspace(), strategy.NewMilestone)
  assert new.milestone_draft == Some(roadmap.new_draft())
  assert new.milestone_request != ""
  // A delete asks first, and not while a form is open.
  assert step(editing, strategy.DeleteMilestone("m1")).milestone_deleting
    == None
  assert step(saved, strategy.DeleteMilestone("m1")).milestone_deleting
    == Some("m1")
}

pub fn a_dependency_change_shows_its_refusal_or_the_task_test() {
  let model = workspace()
  let cycle =
    step(
      model,
      strategy.DependsPatched(
        "web",
        "{\"code\":\"E7025\",\"message\":\"E7025 these dependencies would make a cycle\"}",
      ),
    )
  assert cycle.milestone_error == "E7025 these dependencies would make a cycle"
  let done =
    step(
      cycle,
      strategy.DependsPatched(
        "web",
        json.to_string(task("t1", "A", "", "backlog", "", 0)),
      ),
    )
  assert done.milestone_error == ""
  let assert [#("web", row)] = strategy.visible(done)
  assert row.depends == ["t0"]
}

/// A strategy run as the stream or a reply carries it.
fn run(
  id: String,
  state: String,
  context: #(String, String),
  started: Int,
  extra: List(#(String, json.Json)),
) -> json.Json {
  json.object([
    #("id", json.string(id)),
    #("kind", json.string("strategy")),
    #("model", json.string("fake")),
    #("state", json.string(state)),
    #("started", json.int(started)),
    #("context", case context {
      #("page", _) -> json.object([#("kind", json.string("page"))])
      #(kind, id) ->
        json.object([#("kind", json.string(kind)), #("id", json.string(id))])
    }),
    ..extra
  ])
}

/// A workspace with task t1 in the root's project, feature f1, milestone m1,
/// and an open run on each, plus an ended one on the page.
fn planned() -> strategy.Model {
  workspace()
  |> step(
    strategy.Live(
      message(".", [
        #(
          "tasks",
          json.array([task("t1", "Index", "", "backlog", "", 0)], fn(t) { t }),
        ),
      ]),
    ),
  )
  |> step(strategy.Live(
    "{\"features\":[{\"id\":\"f1\",\"title\":\"Search\",\"milestone\":\"m1\"}]}",
  ))
  |> step(strategy.Live(
    "{\"milestones\":[{\"id\":\"m1\",\"title\":\"Launch\"}]}",
  ))
  |> step(
    strategy.Live(
      json.to_string(
        json.object([
          #(
            "strategyRuns",
            json.preprocessed_array([
              run("r0", "ended", #("page", ""), 5, [
                #("outcome", json.string("planned")),
              ]),
              run("r1", "waiting", #("task", "t1"), 10, []),
              run("r2", "running", #("feature", "f1"), 20, []),
              run("r3", "waiting", #("milestone", "m1"), 30, []),
            ]),
          ),
        ]),
      ),
    ),
  )
}

/// Start plans the task open on the Tasks tab, else the whole page (D-93).
pub fn start_takes_its_context_from_the_selection_test() {
  let model = planned()
  assert strategy.plan_context(model) == plan.page
  let model = step(model, strategy.Open(".", "t1"))
  assert strategy.plan_context(model) == plan.Context("task", "t1")
  assert strategy.plan_context(step(model, strategy.Select(strategy.Vision)))
    == plan.page
  assert json.to_string(plan.start_body(
      "fake",
      "",
      "Plan it",
      plan.Context("task", "t1"),
    ))
    == "{\"model\":\"fake\",\"text\":\"Plan it\",\"context\":{\"kind\":\"task\",\"id\":\"t1\"}}"
  assert json.to_string(plan.start_body("", "high", "Go", plan.page))
    == "{\"effort\":\"high\",\"text\":\"Go\",\"context\":{\"kind\":\"page\"}}"
  // A task with an open run offers it instead of a new start: New run
  // shows it.
  let shown = element.to_string(strategy.view(model))
  assert string.contains(shown, solace_task.ui_label("plan_open_here"))
  assert !string.contains(shown, "id=\"plan-text\"")
  assert plan.new_run(model.runs, strategy.plan_context(model)) == "r1"
  assert plan.new_run(model.runs, plan.page) == ""
}

/// A task row, a feature and a milestone with an open run show a marker; it
/// goes when the run ends.
pub fn open_runs_mark_their_records_until_they_end_test() {
  let model = planned()
  let page = fn(model) { element.to_string(strategy.view(model)) }
  let markers = fn(model) {
    list.length(string.split(page(model), ">Plan</span>")) - 1
  }
  assert plan.open_ids(model.runs, "task") == ["t1"]
  assert markers(model) == 3
  let marker =
    "<span class=\"planning\" title=\""
    <> solace_task.ui_label("plan_marker")
    <> "\">"
  assert string.contains(page(model), "Index</button>" <> marker)
  assert string.contains(
    page(model),
    "<h4>Search <span class=\"state state-proposed\">proposed</span>" <> marker,
  )
  assert string.contains(
    page(model),
    "<h3 data-milestone-id=\"m1\"><button aria-expanded=\"false\" class=\"toggle\" type=\"button\"><span class=\"title\">Launch</span><span class=\"facts\"><span class=\"state state-proposed\">proposed</span>"
      <> marker,
  )
  let ended =
    model
    |> step(
      strategy.Live(
        json.to_string(
          json.object([
            #("strategyRun", run("r1", "ended", #("task", "t1"), 10, [])),
          ]),
        ),
      ),
    )
    |> step(
      strategy.Live(
        json.to_string(
          json.object([
            #("strategyRun", run("r2", "ended", #("feature", "f1"), 20, [])),
          ]),
        ),
      ),
    )
  assert plan.open_ids(ended.runs, "task") == []
  assert markers(ended) == 1
  assert string.contains(
    page(ended),
    "<h3 data-milestone-id=\"m1\"><button aria-expanded=\"false\" class=\"toggle\" type=\"button\"><span class=\"title\">Launch</span><span class=\"facts\"><span class=\"state state-proposed\">proposed</span>"
      <> marker,
  )
}

/// The shown run stays in the panel across tab switches; the panel lists
/// the runs on its context newest first under Earlier runs, and New run
/// returns to the start form.
pub fn the_panel_stays_across_tabs_and_lists_past_runs_test() {
  let model =
    planned()
    |> step(
      strategy.Live(
        json.to_string(
          json.object([
            #(
              "strategyRun",
              run("r4", "ended", #("page", ""), 50, [
                #("outcome", json.string("stopped")),
              ]),
            ),
          ]),
        ),
      ),
    )
    |> step(strategy.ShowPlan("r3"))
  let shown = fn(model) { element.to_string(strategy.view(model)) }
  assert string.contains(shown(model), "Waiting for you · fake")
  assert string.contains(shown(model), "<em>Launch</em>")
  let model = step(model, strategy.Select(strategy.Features))
  assert model.plan_open == "r3"
  assert string.contains(shown(model), "Waiting for you · fake")
  assert string.contains(shown(model), "id=\"plan-text\"")
  let model = step(model, strategy.ShowPlan(""))
  let page = shown(model)
  assert string.contains(page, "id=\"plan-text\"")
  assert string.contains(page, ">Start</button>")
  assert list.map(plan.past(model.runs, plan.page), fn(r) { r.id })
    == ["r4", "r0"]
  let assert Ok(#(_, after)) = string.split_once(page, "Earlier runs")
  assert string.contains(after, "Ended: stopped")
  assert string.contains(after, "Ended: planned")
  // An ended run opens read-only.
  let ended = shown(step(model, strategy.ShowPlan("r4")))
  assert !string.contains(ended, "id=\"plan-text\"")
  // Collapsed, the panel keeps one line.
  let collapsed = shown(step(model, strategy.PlanCollapse))
  assert string.contains(collapsed, "No run open")
  assert !string.contains(collapsed, "id=\"plan-text\"")
}

/// A reply's run is shown with its notes and proposals; an answer is sent
/// once, and a refusal shows.
pub fn a_proposal_shows_and_one_answer_is_sent_at_a_time_test() {
  let reply =
    json.to_string(
      run("r5", "waiting", #("page", ""), 60, [
        #(
          "notes",
          json.preprocessed_array([
            json.object([
              #("author", json.string("human")),
              #("text", json.string("Plan search")),
            ]),
            json.object([
              #("author", json.string("agent")),
              #("text", json.string("Here is a task")),
            ]),
          ]),
        ),
        #(
          "questions",
          json.preprocessed_array([
            json.object([
              #("id", json.string("q1")),
              #("text", json.string("Create task \"Index\"")),
              #("options", json.array(["Approve", "Reject"], json.string)),
              #("answer", json.null()),
              #(
                "proposal",
                json.object([
                  #("kind", json.string("task")),
                  #(
                    "fields",
                    json.object([
                      #("title", json.string("Index")),
                      #("feature", json.string("f1")),
                    ]),
                  ),
                  #(
                    "reopens",
                    json.preprocessed_array([
                      json.object([
                        #("kind", json.string("feature")),
                        #("id", json.string("f1")),
                      ]),
                    ]),
                  ),
                ]),
              ),
            ]),
            json.object([
              #("id", json.string("q2")),
              #("text", json.string("Set feature f1 active")),
              #("options", json.array(["Approve", "Reject"], json.string)),
              #("answer", json.null()),
              #(
                "proposal",
                json.object([
                  #("kind", json.string("feature")),
                  #("id", json.string("f1")),
                  #(
                    "fields",
                    json.object([
                      #("title", json.string("Search")),
                      #("state", json.string("active")),
                    ]),
                  ),
                  #(
                    "before",
                    json.object([
                      #("title", json.string("Search")),
                      #("state", json.string("upcoming")),
                    ]),
                  ),
                ]),
              ),
            ]),
          ]),
        ),
      ]),
    )
  let model =
    planned()
    |> step(strategy.PlanText("Plan search"))
    |> step(strategy.StartPlan)
  assert model.plan_sending
  let model = step(model, strategy.PlanReplied(True, reply))
  assert model.plan_open == "r5" && !model.plan_sending && model.plan_text == ""
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "Here is a task")
  assert string.contains(page, "Create task<em> Index</em>")
  assert string.contains(
    page,
    "Approving also sets the feature <a data-feature=\"f1\" href=\"/feature/f1\">Search</a> back to upcoming.",
  )
  assert string.contains(page, ">Approve</button>")
  // D-115: an edit shows only the fields that change, a short one as old →
  // new; a field that still matches the record's value isn't shown.
  assert string.contains(page, "<dt>state</dt><dd>upcoming → active</dd>")
  assert !string.contains(page, "<dt>title</dt><dd>Search</dd>")
  let sent = step(model, strategy.AnswerPlan("q1", "Approve"))
  assert sent.plan_sending
  assert step(sent, strategy.AnswerPlan("q1", "Reject")) == sent
  let refused =
    step(
      sent,
      strategy.PlanReplied(
        False,
        "{\"code\":\"E7003\",\"message\":\"E7003 the question is already answered\"}",
      ),
    )
  assert !refused.plan_sending
  assert string.contains(
    element.to_string(strategy.view(refused)),
    "E7003 the question is already answered",
  )
  // The replayed list holds no notes: the run keeps the ones it has.
  let replayed =
    step(
      refused,
      strategy.Live(
        json.to_string(
          json.object([
            #(
              "strategyRuns",
              json.preprocessed_array([
                run("r5", "waiting", #("page", ""), 60, []),
              ]),
            ),
          ]),
        ),
      ),
    )
  let assert [r5] = replayed.runs
  assert list.length(r5.notes) == 2
}

/// The case reported, on the Tasks tab: a task stored in `web` and covering
/// the root's project too, its detail handed a running investigation and then
/// the task completed, in order; this task's update under the other project
/// too, a change to the list of projects, and a `resync` once the stream
/// catches up again.
pub fn a_completed_task_reaches_its_detail_test() {
  let one = fn(project, column, state) {
    "{\"project\":\""
    <> project
    <> "\",\"task\":{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\""
    <> column
    <> "\",\"rank\":1,\"scope\":[\"web\",\".\"],\"runs\":[{\"id\":\"r\",\"kind\":\"investigate\",\"model\":\"m\",\"state\":\""
    <> state
    <> "\",\"started\":1,\"log\":\"\"}]}}"
  }
  let running = one("web", "assigned", "running")
  let done = one("web", "complete", "ended")
  let elsewhere = one(".", "complete", "ended")
  let model =
    workspace()
    |> step(strategy.Open("web", "1"))
    |> step(strategy.Live(running))
    |> step(strategy.Live(done))
    |> step(strategy.Live(elsewhere))
    |> step(strategy.Live("{\"project\":\".\",\"workflow\":{\"columns\":[]}}"))
    |> step(strategy.Live("{\"projects\":[\".\",\"web\",\"api\"]}"))
  assert model.live
    == [
      #(1, running),
      #(2, done),
      #(3, elsewhere),
      #(4, "{\"projects\":[\".\",\"web\",\"api\"]}"),
    ]
  let model = step(model, strategy.CaughtUp)
  assert model.live
    == [
      #(1, running),
      #(2, done),
      #(3, elsewhere),
      #(4, "{\"projects\":[\".\",\"web\",\"api\"]}"),
      #(5, "{\"resync\":true}"),
    ]
}

/// The panel's width: a separator on its left edge, kept between the
/// minimum and the window's bound; Left widens it. Collapsed, no handle.
pub fn the_panel_is_resized_within_its_bounds_test() {
  assert plan.clamp(100, 1000) == 280
  assert plan.clamp(900, 1000) == 600
  assert plan.clamp(500, 1000) == 500
  assert plan.clamp(500, 0) == 500
  assert plan.max_width(300) == 280
  let model = planned()
  assert model.plan_width == 380
  let html = element.to_string(strategy.view(model))
  assert string.contains(html, "role=\"separator\"")
  assert string.contains(html, "aria-valuenow=\"380\"")
  assert string.contains(html, "tabindex=\"0\"")
  assert string.contains(html, "--plan-width:380px")
  assert step(model, strategy.PlanKey("ArrowLeft")).plan_width == 396
  assert step(model, strategy.PlanKey("ArrowRight")).plan_width == 364
  assert step(model, strategy.PlanKey("Home")).plan_width == 280
  assert step(model, strategy.PlanKey("Tab")).plan_width == 380
  assert step(model, strategy.PlanWidth(100)).plan_width == 280
  assert step(model, strategy.PlanWidth(512)).plan_width == 512
  let collapsed =
    element.to_string(strategy.view(step(model, strategy.PlanCollapse)))
  assert !string.contains(collapsed, "role=\"separator\"")
}

/// The panel says what it plans: the whole project, or the record by kind.
pub fn the_panel_names_its_planning_context_test() {
  let shown = fn(model) { element.to_string(strategy.view(model)) }
  let model = planned()
  assert string.contains(
    shown(model),
    solace_task.ui_label("plan_context") <> ": Whole Project",
  )
  let model = step(model, strategy.Open(".", "t1"))
  assert string.contains(
    shown(model),
    solace_task.ui_label("plan_context") <> ": task <em>Index</em>",
  )
  assert string.contains(
    shown(step(model, strategy.ShowPlan("r2"))),
    solace_task.ui_label("plan_context") <> ": feature <em>Search</em>",
  )
  assert string.contains(
    shown(step(model, strategy.ShowPlan("r3"))),
    solace_task.ui_label("plan_context") <> ": milestone <em>Launch</em>",
  )
  assert !string.contains(shown(model), "On the whole page")
}

/// D-98: a task link opens the task on the Tasks tab, from any tab, and asks
/// `<solace-task>` to show it; asking again for the same task is news too.
pub fn a_task_link_opens_its_task_on_the_tasks_tab_test() {
  let model = strategy.new(strategy.Roadmap)
  let #(model, _) = strategy.update(model, strategy.Focus("task", "a", "abc"))
  assert model.tab == strategy.Tasks
  assert model.selected == Some(#("a", "abc"))
  assert model.reveal == #("abc", 1)
  let #(model, _) = strategy.update(model, strategy.Focus("task", "a", "abc"))
  assert model.reveal == #("abc", 2)
  let #(same, _) = strategy.update(model, strategy.Focus("task", "a", ""))
  assert same.reveal == #("abc", 2)
}

/// A run that goes waiting has replied, and a new unanswered proposal asks
/// the human; an answer, a run starting, or nothing new does not.
pub fn attention_when_a_run_replies_or_proposes_test() {
  let run = fn(state, questions) {
    plan.Run(
      id: "r1",
      model: "m",
      effort: "",
      state: state,
      started: 1,
      ended: 0,
      outcome: "",
      context: plan.page,
      notes: [],
      questions: questions,
      used: "",
    )
  }
  let ask = fn(id, answer) {
    plan.Question(id, "Add a task?", ["Yes", "No"], answer, None, 0, -1)
  }
  assert strategy.needs_attention([run("running", [])], [run("waiting", [])])
  assert !strategy.needs_attention([run("waiting", [])], [run("waiting", [])])
  assert !strategy.needs_attention([], [run("running", [])])
  assert !strategy.needs_attention([run("waiting", [])], [run("running", [])])
  assert strategy.needs_attention([run("running", [])], [
    run("running", [ask("q1", "")]),
  ])
  assert !strategy.needs_attention([run("running", [ask("q1", "")])], [
    run("running", [ask("q1", "Yes")]),
  ])
  assert !strategy.needs_attention([run("running", [ask("q1", "")])], [
    run("running", [ask("q1", "")]),
  ])
}

/// The open task's Depends on line updates live: the update of a task it
/// depends on, in another project, goes to its details; another task's
/// doesn't.
pub fn a_dependency_update_goes_to_the_open_task_test() {
  let model =
    workspace()
    |> step(
      strategy.Live(
        message("web", [#("task", task("t1", "A", "", "done", "", 1))]),
      ),
    )
    |> step(strategy.Open("web", "t1"))
    |> step(
      strategy.Live(
        message(".", [#("task", task("t0", "Dep", "", "done", "", 1))]),
      ),
    )
    |> step(
      strategy.Live(
        message(".", [#("task", task("t9", "Other", "", "done", "", 1))]),
      ),
    )
  // `task` makes every task depend on t0.
  assert list.map(model.live, fn(m) { m.0 }) == [1]
  let assert [#(_, handed)] = model.live
  assert string.contains(handed, "\"t0\"")
}

/// A started feature's edge has the color its tasks' cards show on the board,
/// by its kept slot (D-114); a feature not started has none.
pub fn a_started_feature_shows_its_color_test() {
  let html =
    workspace()
    |> step(strategy.Live(
      "{\"features\":[{\"id\":\"f1\",\"title\":\"Late\",\"colorSlot\":1},{\"id\":\"f2\",\"title\":\"Early\",\"colorSlot\":0},{\"id\":\"f3\",\"title\":\"Idle\"}]}",
    ))
    |> step(strategy.Select(strategy.Features))
    |> strategy.view
    |> element.to_string
  let article = fn(title) {
    let assert Ok(a) =
      string.split(html, "<article")
      |> list.find(fn(s) { string.contains(s, "<h4>" <> title) })
    a
  }
  assert string.contains(
    article("Early"),
    "8px solid " <> solace_task.feature_color(0),
  )
  assert string.contains(
    article("Late"),
    "8px solid " <> solace_task.feature_color(1),
  )
  assert !string.contains(article("Idle"), "8px solid")
}

/// Whether each of `parts` is in `page`, each after the one before.
fn in_order(page: String, parts: List(String)) -> Bool {
  case parts {
    [] -> True
    [part, ..rest] ->
      case string.split_once(page, part) {
        Ok(#(_, after)) -> in_order(after, rest)
        Error(_) -> False
      }
  }
}

/// A run's note and question as the stream or a reply carries them; a
/// question with time 0 has none, as one saved before D-118.
fn said(text: String, time: Int) -> json.Json {
  json.object([
    #("author", json.string("agent")),
    #("text", json.string(text)),
    #("time", json.int(time)),
  ])
}

fn asked(id: String, text: String, time: Int) -> json.Json {
  json.object([
    #("id", json.string(id)),
    #("text", json.string(text)),
    #("options", json.array(["Yes", "No"], json.string)),
    #("answer", json.null()),
    ..case time {
      0 -> []
      t -> [#("time", json.int(t))]
    }
  ])
}

/// D-118: a run's notes and proposals are drawn as one conversation by time:
/// a proposal between the notes around it, a note after a proposal after it,
/// and a question with no time after every note.
pub fn notes_and_proposals_are_one_conversation_in_order_test() {
  let reply =
    json.to_string(
      run("r5", "waiting", #("page", ""), 60, [
        #(
          "notes",
          json.preprocessed_array([
            said("First note", 10),
            said("Second note", 30),
            said("Last note", 40),
          ]),
        ),
        #(
          "questions",
          json.preprocessed_array([
            asked("q1", "Between question", 20),
            asked("q2", "Untimed question", 0),
          ]),
        ),
      ]),
    )
  let model =
    planned()
    |> step(strategy.PlanText("Plan search"))
    |> step(strategy.StartPlan)
    |> step(strategy.PlanReplied(True, reply))
  let page = element.to_string(strategy.view(model))
  assert in_order(page, [
    "First note",
    "Between question",
    "Second note",
    "Last note",
    "Untimed question",
    "plan-end",
  ])
  assert !in_order(page, ["Second note", "Between question"])
}

/// D-118: opening a run starts at its newest message; a new one follows
/// while the human is at the run's end, and while they read further up it
/// shows "New message ↓" instead, which scrolls down.
pub fn new_messages_follow_the_human_or_wait_test() {
  let with_notes = fn(texts: List(String)) {
    run("r5", "waiting", #("page", ""), 60, [
      #(
        "notes",
        json.preprocessed_array(
          list.index_map(texts, fn(t, i) { said(t, 10 + i) }),
        ),
      ),
      #("questions", json.preprocessed_array([])),
    ])
  }
  let arrive = fn(model, run) {
    step(
      model,
      strategy.Live(json.to_string(json.object([#("strategyRun", run)]))),
    )
  }
  let model =
    planned()
    |> step(strategy.PlanText("Hi"))
    |> step(strategy.StartPlan)
    |> step(strategy.PlanReplied(True, json.to_string(with_notes(["Hi"]))))
  assert model.plan_jump
  let loaded =
    step(
      model,
      strategy.PlanLoaded(
        model.plan_poll,
        "r5",
        json.to_string(
          json.object([
            #("id", json.string("r5")),
            #("kind", json.string("strategy")),
            #("model", json.string("fake")),
            #("state", json.string("waiting")),
            #("started", json.int(60)),
            #("context", json.object([#("kind", json.string("page"))])),
            #("notes", json.preprocessed_array([said("Hi", 10)])),
            #("questions", json.preprocessed_array([])),
            #("transcript", json.preprocessed_array([])),
          ]),
        ),
      ),
    )
  assert !loaded.plan_jump && loaded.plan_at_bottom && !loaded.plan_new
  // At the end: the reply follows, with no button.
  let followed = arrive(loaded, with_notes(["Hi", "Reply"]))
  assert followed.plan_at_bottom && !followed.plan_new
  assert !string.contains(
    element.to_string(strategy.view(followed)),
    "New message ↓",
  )
  // Scrolled up to read: no jump, the button shows.
  let reading = step(followed, strategy.PlanScrolled(False))
  let waiting = arrive(reading, with_notes(["Hi", "Reply", "Another"]))
  assert waiting.plan_new && !waiting.plan_at_bottom
  let page = element.to_string(strategy.view(waiting))
  assert in_order(page, ["Another", "New message ↓", "plan-end"])
  // The same run again adds nothing new.
  assert !arrive(reading, with_notes(["Hi", "Reply"])).plan_new
  let back = step(waiting, strategy.PlanNewest)
  assert back.plan_at_bottom && !back.plan_new
  assert !string.contains(
    element.to_string(strategy.view(back)),
    "New message ↓",
  )
  // Scrolling back to the end clears the button too.
  assert !step(waiting, strategy.PlanScrolled(True)).plan_new
}

/// The run bar sits above the conversation, the message box below it. An
/// open run's bar has End, Stop, Earlier runs and New run, and its box is the
/// markdown editor, which styles as it is written, with Send; an ended run's
/// bar has no End or Stop and there is no box; with no run shown, the box
/// starts one.
pub fn the_run_bar_holds_the_run_controls_test() {
  let shown = fn(model) { element.to_string(strategy.view(model)) }
  let open = shown(step(planned(), strategy.ShowPlan("r3")))
  assert in_order(open, [
    "class=\"plan-bar\"",
    "Waiting for you · fake",
    ">End</button>",
    ">Stop</button>",
    "Earlier runs",
    ">New run</button>",
    "class=\"plan-body\"",
    "class=\"plan-compose\"",
    "class=\"composer\"",
    "<solace-md-editor",
    "id=\"plan-text\"",
    ">Send</button>",
  ])
  assert !string.contains(open, ">Preview</button>")
  assert !string.contains(open, "Runs on this")
  let ended = shown(step(planned(), strategy.ShowPlan("r0")))
  assert in_order(ended, [
    "class=\"plan-bar\"",
    "Ended: planned",
    "Earlier runs",
    ">New run</button>",
    "class=\"plan-body\"",
  ])
  assert !string.contains(ended, ">End</button>")
  assert !string.contains(ended, ">Stop</button>")
  assert !string.contains(ended, "id=\"plan-text\"")
  let none = shown(planned())
  assert in_order(none, [
    "class=\"plan-bar\"",
    "No run open",
    ">New run</button>",
    "class=\"plan-body\"",
    "class=\"plan-compose\"",
    "class=\"composer\"",
    "id=\"plan-text\"",
    ">Start</button>",
  ])
  assert !string.contains(none, ">End</button>")
  // The box keeps the typed message; a sent one clears it.
  let typed = planned() |> step(strategy.PlanText("**Plan** search"))
  assert typed.plan_text == "**Plan** search"
}

/// D-122: a feature link opens the Features tab, a milestone link the
/// Roadmap tab; neither opens a task.
pub fn a_feature_or_milestone_link_opens_its_tab_test() {
  let model = strategy.new(strategy.Tasks)
  let #(model, _) = strategy.update(model, strategy.Focus("feature", "", "f1"))
  assert model.tab == strategy.Features
  let #(model, _) =
    strategy.update(model, strategy.Focus("milestone", "", "m1"))
  assert model.tab == strategy.Roadmap
  assert model.milestone_open == ["m1"]
  assert model.selected == None
  assert model.reveal == #("", 0)
  let #(same, _) = strategy.update(model, strategy.Focus("feature", "", ""))
  assert same.tab == strategy.Roadmap
}

/// The Tasks tab's Column filter shows the workflow's labels, not the column
/// ids, as the board's column bar does.
pub fn the_column_filter_shows_the_workflow_labels_test() {
  let model =
    workspace()
    |> step(strategy.WorkflowLoaded(
      ".",
      "{\"columns\":[{\"id\":\"backlog\",\"label\":\"Backlog\"},{\"id\":\"planning\",\"label\":\"Planning\"},{\"id\":\"assigned\",\"label\":\"Assigned\"},{\"id\":\"needs_review\",\"label\":\"Review\"},{\"id\":\"complete\",\"label\":\"Complete\",\"final\":true},{\"id\":\"cancelled\",\"label\":\"Canceled\",\"final\":true}]}",
    ))
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, ">Backlog</option>")
  assert string.contains(page, ">Planning</option>")
  assert string.contains(page, ">Assigned</option>")
  assert string.contains(page, ">Review</option>")
  assert string.contains(page, ">Complete</option>")
  assert string.contains(page, ">Canceled</option>")
  assert !string.contains(page, ">needs_review</option>")
}

/// Every page names its controls with the shared words from `ui_labels`, so
/// a rename is one edit in the label map.
pub fn shared_labels_on_the_pages_test() {
  let shown = fn(model) { element.to_string(strategy.view(model)) }
  let button = fn(name) { ">" <> solace_task.ui_label(name) <> "</button>" }
  let model =
    planned()
    |> step(strategy.ModelsLoaded(
      "[{\"id\":\"m\",\"label\":\"M\",\"available\":true}]",
    ))
  // The panel is Plan, and says what it plans with the same word.
  assert string.contains(
    shown(model),
    ">" <> solace_task.ui_label("plan") <> "</h2>",
  )
  assert string.contains(
    shown(model),
    "aria-label=\"" <> solace_task.ui_label("plan") <> "\"",
  )
  assert string.contains(
    shown(model),
    solace_task.ui_label("plan_context") <> ": Whole Project",
  )
  assert string.contains(
    shown(model),
    ">" <> solace_task.ui_label("plan") <> "</span>",
  )
  assert string.contains(
    shown(model),
    "aria-label=\"" <> solace_task.ui_label("model") <> "\"",
  )
  // The open run on the selection is offered by name.
  let model = step(model, strategy.Open(".", "t1"))
  assert string.contains(shown(model), button("open_run"))
  // Features: New feature, its form's Create task, and the project picker.
  let model = step(model, strategy.Select(strategy.Features))
  // The new-task form shows on the focused feature only.
  assert !string.contains(shown(model), button("create_task"))
  let model = step(model, strategy.FocusFeature("f1"))
  assert string.contains(shown(model), button("new_feature"))
  assert string.contains(shown(model), button("create_task"))
  assert string.contains(
    shown(model),
    "aria-label=\"" <> solace_task.ui_label("project") <> "\"",
  )
  // Roadmap: New milestone, and the dependency editor's words.
  // A task reaches the roadmap by its feature, so link t1 to f1.
  let model =
    step(
      model,
      strategy.Live(
        message(".", [
          #(
            "tasks",
            json.array([task("t1", "Index", "", "backlog", "f1", 0)], fn(t) {
              t
            }),
          ),
        ]),
      ),
    )
  let model = step(model, strategy.Select(strategy.Roadmap))
  assert string.contains(shown(model), button("new_milestone"))
  let model = step(model, strategy.ToggleMilestone("m1"))
  assert string.contains(shown(model), button("dependencies"))
  // Vision: Save and Cancel.
  let model = step(model, strategy.Select(strategy.Vision))
  let model = step(model, strategy.EditVision)
  assert string.contains(shown(model), button("save"))
  assert string.contains(shown(model), button("cancel"))
}

/// Text areas fill their container: the composer's markdown editor is a
/// block, so it fills the box; the plan panel's is shorter and scrolls past
/// 40dvh; and the composer box, the plan panel's composer, and the feature,
/// vision, feature-form and picker panels have no `max-width` of their
/// own.
pub fn textareas_fill_their_container_test() {
  let assert [css, ..] =
    element.to_string(strategy.view(planned())) |> string.split("</style>")
  assert string.contains(
    css,
    ".composer{border:1px solid var(--edge);border-radius:0;margin:8px 4px;background:var(--paper)}",
  )
  assert string.contains(
    css,
    ".composer solace-md-editor{display:block;border-top:1px solid var(--line)}",
  )
  assert string.contains(css, ".plan .composer{margin:0}")
  assert string.contains(
    css,
    ".plan .composer solace-md-editor{--md-min-height:5lh;--md-max-height:40dvh}",
  )
  // Each panel is checked for a cap of its own, so a `max-width` added
  // elsewhere in the page can't fail this test.
  assert !string.contains(css, ".composer{max-width")
  assert !string.contains(css, ".plan .composer{max-width")
  assert !string.contains(css, ".feature{max-width")
  assert !string.contains(css, ".vision{max-width")
  assert !string.contains(css, ".feature-form{max-width")
  assert !string.contains(css, ".picker{max-width")
  assert string.contains(
    css,
    ".feature{background:var(--paper);border:1px solid var(--edge);padding:var(--pad-card);margin:0 0 var(--space-3)}",
  )
  assert string.contains(
    css,
    ".vision{background:var(--paper);border:1px solid var(--edge);padding:var(--pad-card)}",
  )
  assert string.contains(
    css,
    ".feature-form{background:var(--paper);border:1px solid var(--edge);padding:var(--pad-card);margin:0 0 var(--space-4)}",
  )
  assert string.contains(
    css,
    ".picker{border:1px solid var(--line);margin:var(--space-1) 0 var(--space-2);padding:var(--space-1) var(--space-3)}",
  )
}

/// The tab bar names each panel, so no panel repeats its tab's name as a heading.
pub fn tabs_have_no_heading_of_their_own_test() {
  let page = element.to_string(strategy.view(planned()))
  // Any heading level counts: a tab's name must not return as one.
  assert !string.contains(page, ">Tasks</h")
  assert !string.contains(page, ">Vision</h")
  assert !string.contains(page, ">Features</h")
  assert !string.contains(page, ">Roadmap</h")
  // The plan panel's heading names the panel, not a tab, and stays.
  assert string.contains(page, ">" <> solace_task.ui_label("plan") <> "</h2>")
}

/// A feature card is compact (a "final / total final" count line)
/// until it is focused, by a link or its More button; then it shows its body
/// and task list. More again compacts it.
pub fn only_the_focused_feature_shows_its_body_and_tasks_test() {
  let model =
    workspace()
    |> step(
      strategy.Live(
        message(".", [
          #(
            "tasks",
            json.preprocessed_array([
              task("t1", "Index", "", "done", "f1", 10),
              task("t2", "Rank", "", "backlog", "f1", 0),
              task("t3", "Query", "", "backlog", "f1", 0),
            ]),
          ),
        ]),
      ),
    )
    |> step(strategy.Live(
      "{\"features\":[{\"id\":\"f1\",\"title\":\"Search\",\"body\":\"Find things fast\",\"milestone\":\"m1\"},{\"id\":\"f2\",\"title\":\"Other\",\"body\":\"Second body\"}]}",
    ))
    |> step(strategy.Live(
      "{\"milestones\":[{\"id\":\"m1\",\"title\":\"Launch\"}]}",
    ))
    |> step(strategy.Select(strategy.Features))
  let article = fn(model, title) {
    let html = element.to_string(strategy.view(model))
    let assert Ok(a) =
      string.split(html, "<article")
      |> list.find(fn(s) { string.contains(s, "<h4>" <> title) })
    a
  }
  let compact = article(model, "Search")
  assert string.contains(compact, "1 / 3 final")
  assert !string.contains(compact, "Find things fast")
  assert !string.contains(compact, "feature-tasks")
  let #(model, _) = strategy.update(model, strategy.Focus("feature", "", "f1"))
  let full = article(model, "Search")
  assert string.contains(full, "Find things fast")
  assert string.contains(full, "feature-tasks")
  assert string.contains(full, ">Rank</button>")
  assert !string.contains(full, " final</p>")
  let other = article(model, "Other")
  assert string.contains(other, "0 / 0 final")
  assert !string.contains(other, "Second body")
  let model = step(model, strategy.FocusFeature("f1"))
  assert !string.contains(article(model, "Search"), "Find things fast")
}

/// The page carries the shared control styles, so its controls and the
/// option lists of its selects follow the theme.
pub fn page_uses_the_shared_control_styles_test() {
  let page = element.to_string(strategy.view(planned()))
  assert string.contains(page, "option,optgroup{background:var(--paper)")
}

/// A filter input fills its label: `width:100%` with no side margins, so
/// it stays inside the label's edge.
pub fn filters_inputs_fill_their_labels_test() {
  let assert [css, ..] =
    element.to_string(strategy.view(planned())) |> string.split("</style>")
  assert string.contains(
    css,
    ".filters input,.filters select{width:100%;margin:0}",
  )
}

/// D-129: a task event marked `noProject` lands in the homeless bucket,
/// whatever home it names: it shows on the Tasks tab, the Project filter
/// keeps it alone, its row says "No project", and a link naming its home
/// folder opens it there. The root's homeless snapshot replaces the bucket.
pub fn a_task_with_no_project_shows_on_the_tasks_tab_test() {
  let homeless = fn(fields: List(#(String, json.Json))) {
    json.to_string(
      json.object([
        #("project", json.string("web")),
        #("noProject", json.bool(True)),
        ..fields
      ]),
    )
  }
  let model =
    workspace()
    |> step(
      strategy.Live(
        message(".", [
          #(
            "tasks",
            json.array([task("t1", "Root", "", "done", "", 10)], fn(t) { t }),
          ),
        ]),
      ),
    )
    |> step(
      strategy.Live(
        homeless([
          #(
            "tasks",
            json.array([task("t8", "Lost", "", "backlog", "", 0)], fn(t) { t }),
          ),
        ]),
      ),
    )
    |> step(
      strategy.Live(
        homeless([#("task", task("t9", "Orphan", "", "backlog", "", 0))]),
      ),
    )
  assert ids(model) == ["t1", "t8", "t9"]
  let filtered =
    step(
      model,
      strategy.Filter(strategy.Filters(..model.filters, project: "no-project")),
    )
  assert ids(filtered) == ["t8", "t9"]
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, ">No project</td>")
  // A link naming the task's home folder opens the homeless row.
  let #(opened, _) = strategy.update(model, strategy.Focus("task", "web", "t9"))
  assert opened.selected == Some(#("no-project", "t9"))
  // A task event without the mark stays in the project it names.
  let model =
    step(
      model,
      strategy.Live(
        message("web", [#("task", task("t2", "Web", "", "done", "", 5))]),
      ),
    )
  assert ids(model) == ["t1", "t2", "t8", "t9"]
}

/// D-129: a root with one project has no Project column, so a task with no
/// project says so beside its title.
pub fn a_single_project_marks_a_task_with_no_project_test() {
  let model =
    strategy.new(strategy.Tasks)
    |> step(strategy.ProjectsLoaded(
      "{\"workspace\":false,\"projects\":[{\"dir\":\".\",\"label\":\"root\",\"name\":\"r\",\"base\":\"\"}]}",
    ))
    |> step(strategy.CaughtUp)
    |> step(
      strategy.Live(
        json.to_string(
          json.object([
            #("project", json.string("web")),
            #("noProject", json.bool(True)),
            #("task", task("t9", "Orphan", "", "backlog", "", 0)),
          ]),
        ),
      ),
    )
  assert ids(model) == ["t9"]
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, ">No project</span>")
  assert !string.contains(page, ">No project</td>")
  // The mark is styled, set apart from the title.
  assert string.contains(page, ".tasks .homeless{")
}

/// D-118: in the same second, a proposal comes after the notes the server
/// had when it was asked and before the note after it; one saved without
/// that count comes after the notes of its second.
pub fn a_proposal_and_a_note_in_one_second_keep_their_order_test() {
  let reply =
    json.to_string(
      run("r5", "waiting", #("page", ""), 60, [
        #(
          "notes",
          json.preprocessed_array([
            said("Human asks", 20),
            said("Agent follows up", 20),
          ]),
        ),
        #(
          "questions",
          json.preprocessed_array([
            json.object([
              #("id", json.string("q1")),
              #("text", json.string("Counted proposal")),
              #("options", json.array(["Approve", "Reject"], json.string)),
              #("answer", json.null()),
              #("time", json.int(20)),
              #("after", json.int(1)),
            ]),
            asked("q2", "Uncounted proposal", 20),
          ]),
        ),
      ]),
    )
  let model =
    planned()
    |> step(strategy.PlanText("Plan search"))
    |> step(strategy.StartPlan)
    |> step(strategy.PlanReplied(True, reply))
  assert in_order(element.to_string(strategy.view(model)), [
    "Human asks",
    "Counted proposal",
    "Agent follows up",
    "Uncounted proposal",
    "plan-end",
  ])
}

import lustre/dev/query
import lustre/dev/simulate
import lustre/effect

/// With a task open that has an open run, the bar's New run shows that run.
pub fn the_bar_new_run_shows_the_open_run_test() {
  let model = step(planned(), strategy.Open(".", "t1"))
  let sim =
    simulate.application(
      init: fn(_) { #(model, effect.none()) },
      update: strategy.update,
      view: strategy.view,
    )
    |> simulate.start(Nil)
    |> simulate.click(on: query.element(
      matching: query.tag("button") |> query.and(query.text("New run")),
    ))
  assert simulate.model(sim).plan_open == "r1"
}

pub fn the_new_task_form_disables_create_while_the_title_is_blank_test() {
  let create_disabled = fn(model) {
    let assert Ok(#(before, _)) =
      element.to_string(strategy.view(model))
      |> string.split_once("Create task</button>")
    let assert Ok(button) = string.split(before, "<button") |> list.last
    string.contains(button, "disabled")
  }
  let model = workspace()
  assert model.new_task == None
  assert string.contains(element.to_string(strategy.view(model)), "New task")
  let model =
    model |> step(strategy.NewTaskOpen) |> step(strategy.NewTaskTitle("  "))
  // In a workspace the project defaults to the root's own.
  let assert Some(draft) = model.new_task
  assert draft.project == "."
  assert create_disabled(model)
  assert step(model, strategy.CreateTask).task_saving == None
  let model = step(model, strategy.NewTaskTitle("Index"))
  assert !create_disabled(model)
  let saving = step(model, strategy.CreateTask)
  assert saving.task_saving == Some("")
  // While its create is in flight, Cancel is disabled and does nothing.
  let assert Ok(#(before, _)) =
    element.to_string(strategy.view(saving))
    |> string.split_once("Cancel</button>")
  let assert Ok(cancel) = string.split(before, "<button") |> list.last
  assert string.contains(cancel, "disabled")
  assert step(saving, strategy.NewTaskCancel).new_task != None
  // Cancel closes the form.
  assert step(model, strategy.NewTaskCancel).new_task == None
}

pub fn the_new_task_request_is_kept_across_a_refused_create_test() {
  let model =
    workspace()
    |> step(strategy.NewTaskOpen)
    |> step(strategy.NewTaskTitle("Index"))
    |> step(strategy.NewTaskText("Some **text**"))
  let rid = model.new_task_request
  assert rid != ""
  let model =
    model
    |> step(strategy.CreateTask)
    |> step(strategy.TaskCreated(
      ".",
      "",
      "{\"code\":\"E7001\",\"message\":\"E7001 no title\"}",
    ))
  assert model.new_task_error == "Couldn't create the task: E7001 no title"
  assert model.task_saving == None
  assert model.new_task_request == rid
  let assert Some(draft) = model.new_task
  assert draft.title == "Index"
  assert draft.body == "Some **text**"
  // The retry sends the same request.
  let model = step(model, strategy.CreateTask)
  assert model.new_task_request == rid
  assert model.task_saving == Some("")
}

pub fn the_new_task_links_only_what_was_picked_test() {
  let draft = strategy.NewTask("Index", "", "web", "", "")
  assert strategy.new_task_links(draft) == []
  let loose = strategy.NewTask(..draft, milestone: "m1")
  assert strategy.new_task_links(loose) == [#("milestone", "m1")]
  assert json.to_string(strategy.links_body(strategy.new_task_links(loose)))
    == "{\"milestone\":\"m1\"}"
  assert strategy.new_task_links(strategy.NewTask(..draft, feature: "f1"))
    == [#("feature", "f1")]
  let both = strategy.NewTask(..draft, feature: "f1", milestone: "m1")
  assert strategy.new_task_links(both) == [#("feature", "f1")]
  assert json.to_string(strategy.links_body(strategy.new_task_links(both)))
    == "{\"feature\":\"f1\"}"
  let created = json.to_string(task("t9", "Index", "", "backlog", "", 0))
  // Nothing picked: no link, the form closes and the task opens.
  let model =
    workspace()
    |> step(strategy.NewTaskOpen)
    |> step(strategy.NewTaskProject("web"))
    |> step(strategy.NewTaskTitle("Index"))
    |> step(strategy.CreateTask)
    |> step(strategy.TaskCreated("web", "", created))
  assert model.task_linking == None
  assert model.task_saving == None
  assert model.new_task == None
  assert model.selected == Some(#("web", "t9"))
  // A feature picked: the link goes out, and the form waits for it.
  let model =
    workspace()
    |> step(strategy.NewTaskOpen)
    |> step(strategy.NewTaskProject("web"))
    |> step(strategy.NewTaskTitle("Index"))
    |> step(strategy.NewTaskFeature("f1"))
    |> step(strategy.CreateTask)
    |> step(strategy.TaskCreated("web", "", created))
  assert model.task_linking == Some(#("web", "t9", ""))
  assert model.new_task != None
  assert model.selected == None
  // The link's reply closes the form and opens the task.
  let linked =
    step(
      model,
      strategy.Patched(
        "web",
        "t9",
        json.to_string(task("t9", "Index", "", "backlog", "f1", 0)),
      ),
    )
  assert linked.new_task == None
  assert linked.task_linking == None
  assert linked.task_saving == None
  assert linked.selected == Some(#("web", "t9"))
  // A refused link says the task exists without it; the task still opens.
  let refused =
    step(
      model,
      strategy.Patched(
        "web",
        "t9",
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown feature f1\"}",
      ),
    )
  assert refused.new_task_error
    == "The task was created without its feature: E7023 unknown feature f1"
  assert refused.task_saving == None
  assert refused.task_linking == None
  assert refused.selected == Some(#("web", "t9"))
  assert refused.error == ""
  let assert Some(fresh) = refused.new_task
  assert fresh.title == ""
  // Another task's refused PATCH in the same project isn't the form's.
  let other =
    step(
      model,
      strategy.Patched(
        "web",
        "t1",
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown feature f1\"}",
      ),
    )
  assert other.task_linking == Some(#("web", "t9", ""))
  assert other.new_task_error == ""
  assert other.error == "E7023 unknown feature f1"
}

/// The numbers from `from` to `to`, both included.
fn numbers(from: Int, to: Int) -> List(Int) {
  case from > to {
    True -> []
    _ -> [from, ..numbers(from + 1, to)]
  }
}

/// A workspace with `n` open tasks in the root's project, titled so the
/// default order is the tasks' own.
fn many(n: Int) -> strategy.Model {
  workspace()
  |> step(
    strategy.Live(
      message(".", [#("tasks", json.array(task_list(n), fn(t) { t }))]),
    ),
  )
}

/// A title that sorts in the same order as the task's number.
fn title_of(i: Int) -> String {
  case i < 10 {
    True -> "task-0" <> int.to_string(i)
    False -> "task-" <> int.to_string(i)
  }
}

pub fn the_tasks_tab_pages_its_rows_test() {
  assert strategy.pages(0) == 1
  assert strategy.pages(1) == 1
  assert strategy.pages(10) == 1
  assert strategy.pages(11) == 2
  assert strategy.pages(25) == 3
  assert strategy.pages(26) == 3
  let ids_on = fn(total: Int, page: Int) {
    strategy.page_rows(strategy.visible(many(total)), page)
    |> list.map(fn(pair) { pair.1.id })
  }
  let named = fn(from: Int, to: Int) {
    numbers(from, to) |> list.map(fn(i) { "t" <> int.to_string(i) })
  }
  assert ids_on(0, 1) == []
  assert ids_on(1, 1) == named(1, 1)
  assert ids_on(10, 1) == named(1, 10)
  assert ids_on(11, 1) == named(1, 10)
  assert ids_on(11, 2) == named(11, 11)
  assert ids_on(25, 2) == named(11, 20)
  assert ids_on(25, 3) == named(21, 25)
  assert ids_on(26, 3) == named(21, 26)
  // Past the end: the last page, never an empty one.
  assert ids_on(26, 9) == named(21, 26)
  assert strategy.page_text(2, 26) == "page 2 of 3, 26 tasks"
  assert strategy.page_text(1, 1) == "page 1 of 1, 1 task"
  assert strategy.page_text(9, 0) == "page 1 of 1, 0 tasks"
}

pub fn the_tasks_tab_shows_one_page_with_its_count_test() {
  let model = many(30) |> step(strategy.Page(3))
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "task-30")
  assert !string.contains(page, "task-01")
  assert string.contains(page, "page 3 of 3, 30 tasks")
  // Prev is greyed out on the first page, Next on the last.
  let first = element.to_string(strategy.view(many(30)))
  assert string.contains(first, "page 1 of 3, 30 tasks")
  // A button's own tag: the `<button` opening its label.
  let tag_of = fn(html: String, label: String) {
    let assert Ok(#(before, _)) = string.split_once(html, label)
    let assert Ok(tag) = string.split(before, "<button") |> list.last
    tag
  }
  assert string.contains(tag_of(first, "← Prev"), "disabled")
  assert !string.contains(tag_of(first, "Next →"), "disabled")
  assert !string.contains(tag_of(page, "← Prev"), "disabled")
  assert string.contains(tag_of(page, "Next →"), "disabled")
}

pub fn a_filter_change_resets_to_the_first_page_test() {
  let model = many(30) |> step(strategy.Page(2))
  assert model.page == 2
  let model =
    step(model, strategy.Filter(strategy.Filters("", "", "", "", "task-05")))
  assert model.page == 1
  assert ids(model) == ["t5"]
}

pub fn the_task_detail_stays_below_the_table_and_pager_test() {
  let model =
    many(30)
    |> step(strategy.Open(".", "t3"))
    |> step(strategy.Page(2))
  let page = element.to_string(strategy.view(model))
  let at = fn(needle: String) {
    let assert Ok(#(before, _)) = string.split_once(page, needle)
    string.byte_size(before)
  }
  let table = at("</table>")
  let count = at("page 2 of 3, 30 tasks")
  let detail = at("<solace-task")
  assert table < count && count < detail
  // The detail shows even when its row is on another page.
  assert string.contains(page, "task-03")
}

pub fn a_task_link_opens_the_page_holding_the_task_test() {
  let model = many(30) |> step(strategy.Focus("task", ".", "t27"))
  assert model.selected == Some(#(".", "t27"))
  assert model.page == 3
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "task-27")
}

/// D-118: a longer transcript with the same conversation never shows "New
/// message ↓": at the run's end the human stays there, and scrolled up
/// nothing changes.
pub fn transcript_growth_keeps_the_model_test() {
  let entry = fn(t: String) {
    json.object([#("kind", json.string("text")), #("text", json.string(t))])
  }
  let reply = fn(entries: List(json.Json)) {
    json.to_string(
      json.object([
        #("id", json.string("r5")),
        #("kind", json.string("strategy")),
        #("model", json.string("fake")),
        #("state", json.string("waiting")),
        #("started", json.int(60)),
        #("context", json.object([#("kind", json.string("page"))])),
        #("notes", json.preprocessed_array([said("Hi", 10)])),
        #("questions", json.preprocessed_array([])),
        #("transcript", json.preprocessed_array(entries)),
      ]),
    )
  }
  let started =
    run("r5", "waiting", #("page", ""), 60, [
      #("notes", json.preprocessed_array([said("Hi", 10)])),
      #("questions", json.preprocessed_array([])),
    ])
  let model =
    planned()
    |> step(strategy.PlanText("Hi"))
    |> step(strategy.StartPlan)
    |> step(strategy.PlanReplied(True, json.to_string(started)))
  let loaded =
    step(model, strategy.PlanLoaded(model.plan_poll, "r5", reply([])))
  assert loaded.plan_at_bottom && !loaded.plan_new
  let more = reply([entry("Reading"), entry("Thinking")])
  let grown = step(loaded, strategy.PlanLoaded(loaded.plan_poll, "r5", more))
  assert list.length(grown.plan_entries) == 2
  assert grown.plan_at_bottom && !grown.plan_new
  let reading = step(loaded, strategy.PlanScrolled(False))
  let waited = step(reading, strategy.PlanLoaded(reading.plan_poll, "r5", more))
  assert list.length(waited.plan_entries) == 2
  assert !waited.plan_new && !waited.plan_at_bottom
}

/// A workspace whose tasks differ in every sortable column: `a`, `b` in the
/// root's project, `c`, `d` in `web` (`b` and `d` share a title and created
/// time), and `e` with no project, created at no known time.
fn sortable() -> strategy.Model {
  let t = fn(id, title, column, feature, milestone, created, closed) {
    let opt = fn(v) {
      case v {
        "" -> json.null()
        s -> json.string(s)
      }
    }
    let time = fn(v) {
      case v {
        0 -> json.null()
        n -> json.int(n)
      }
    }
    json.object([
      #("id", json.string(id)),
      #("title", json.string(title)),
      #("body", json.string("")),
      #("column", json.string(column)),
      #("feature", opt(feature)),
      #("milestone", opt(milestone)),
      #("dependsOn", json.array([], json.string)),
      #("createdTime", time(created)),
      #("closedTime", time(closed)),
    ])
  }
  let tasks = fn(list) { #("tasks", json.array(list, fn(x) { x })) }
  workspace()
  |> step(strategy.WorkflowLoaded(
    ".",
    "{\"columns\":[{\"id\":\"backlog\",\"label\":\"Backlog\"},{\"id\":\"doing\",\"label\":\"Doing\"},{\"id\":\"done\",\"label\":\"Done\",\"final\":true}]}",
  ))
  |> step(strategy.Live(
    "{\"features\":[{\"id\":\"f1\",\"title\":\"Beta\"},{\"id\":\"f2\",\"title\":\"Alpha\"}]}",
  ))
  |> step(strategy.Live(
    "{\"milestones\":[{\"id\":\"m1\",\"title\":\"Later\"},{\"id\":\"m2\",\"title\":\"Soon\"}]}",
  ))
  |> step(
    strategy.Live(
      message(".", [
        tasks([
          t("a", "Write docs", "done", "f1", "m1", 300, 400),
          t("b", "Add search", "backlog", "f2", "", 100, 0),
        ]),
      ]),
    ),
  )
  |> step(
    strategy.Live(
      message("web", [
        tasks([
          t("c", "Fix login", "doing", "f1", "m2", 200, 500),
          t("d", "Add search", "doing", "", "m1", 100, 0),
        ]),
      ]),
    ),
  )
  |> step(
    strategy.Live(
      json.to_string(
        json.object([
          #("project", json.string("web")),
          #("noProject", json.bool(True)),
          tasks([t("e", "Lost", "backlog", "", "", 0, 0)]),
        ]),
      ),
    ),
  )
}

/// The visible ids after clicking `key` once (ascending) and twice (descending).
fn sorted_by(key: strategy.SortKey) -> #(List(String), List(String)) {
  let once = step(sortable(), strategy.SortBy(key))
  #(ids(once), ids(step(once, strategy.SortBy(key))))
}

/// Each heading's order, both ways: the column by workflow order, the
/// project by label, the feature and milestone by title, the times by time.
/// Missing values (no project, feature, milestone or time) go last either
/// way, and ties fall back to the title, then the id.
pub fn each_column_sorts_by_its_own_order_test() {
  assert sorted_by(strategy.ByTitle)
    == #(["b", "d", "c", "e", "a"], ["a", "e", "c", "b", "d"])
  assert sorted_by(strategy.ByColumn)
    == #(["b", "e", "d", "c", "a"], ["a", "d", "c", "b", "e"])
  assert sorted_by(strategy.ByProject)
    == #(["b", "a", "d", "c", "e"], ["d", "c", "b", "a", "e"])
  assert sorted_by(strategy.ByFeature)
    == #(["b", "c", "a", "d", "e"], ["c", "a", "b", "d", "e"])
  assert sorted_by(strategy.ByMilestone)
    == #(["d", "a", "c", "b", "e"], ["c", "d", "a", "b", "e"])
  assert sorted_by(strategy.ByCreated)
    == #(["b", "d", "c", "a", "e"], ["a", "c", "b", "d", "e"])
  assert sorted_by(strategy.ByClosed)
    == #(["a", "c", "b", "d", "e"], ["c", "a", "b", "d", "e"])
}

/// The same heading flips the direction each click; another heading starts
/// ascending. A new sort goes back to page 1.
pub fn a_heading_click_flips_its_direction_test() {
  let model = step(sortable(), strategy.SortBy(strategy.ByTitle))
  assert model.sort == Some(#(strategy.ByTitle, True))
  let model = step(model, strategy.SortBy(strategy.ByTitle))
  assert model.sort == Some(#(strategy.ByTitle, False))
  let model = step(model, strategy.SortBy(strategy.ByTitle))
  assert model.sort == Some(#(strategy.ByTitle, True))
  let model =
    model
    |> step(strategy.SortBy(strategy.ByTitle))
    |> step(strategy.SortBy(strategy.ByClosed))
  assert model.sort == Some(#(strategy.ByClosed, True))
  let paged =
    many(30)
    |> step(strategy.Page(2))
    |> step(strategy.SortBy(strategy.ByTitle))
  assert paged.page == 1
}

/// Before any click the table keeps the default order: closed newest first,
/// then open ones by title.
pub fn the_default_order_holds_until_a_heading_is_clicked_test() {
  let model = sortable()
  assert model.sort == None
  assert ids(model) == ["c", "a", "b", "d", "e"]
}

/// Filters apply first, then the sort, which stays across filter changes and
/// tab switches; the open row stays open, its details below the table.
pub fn the_sort_stays_across_filters_tabs_and_the_open_row_test() {
  let model =
    sortable()
    |> step(strategy.Open("web", "c"))
    |> step(strategy.SortBy(strategy.ByCreated))
    |> step(strategy.SortBy(strategy.ByCreated))
    |> step(strategy.Filter(strategy.Filters("", "web", "", "", "")))
  assert ids(model) == ["c", "d"]
  assert model.sort == Some(#(strategy.ByCreated, False))
  let model =
    model
    |> step(strategy.Select(strategy.Features))
    |> step(strategy.Select(strategy.Tasks))
  assert model.sort == Some(#(strategy.ByCreated, False))
  assert model.selected == Some(#("web", "c"))
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "aria-current=\"true\"")
  let assert Ok(#(before, _)) = string.split_once(page, "<solace-task")
  assert string.contains(before, "</table>")
}

/// The sorted heading alone shows ▲ or ▼ and sets `aria-sort`; before any
/// click no heading does. Every heading is a button.
pub fn only_the_sorted_heading_shows_its_direction_test() {
  let count = fn(page, needle) { list.length(string.split(page, needle)) - 1 }
  let page = element.to_string(strategy.view(sortable()))
  assert count(page, "aria-sort") == 0
  assert count(page, "▲") == 0 && count(page, "▼") == 0
  assert count(page, "class=\"sort\"") == 7
  let model = step(sortable(), strategy.SortBy(strategy.ByColumn))
  let page = element.to_string(strategy.view(model))
  assert count(page, "aria-sort") == 1
  assert count(page, "▲") == 1 && count(page, "▼") == 0
  let assert Ok(#(_, after)) =
    string.split_once(page, "aria-sort=\"ascending\"")
  let assert Ok(#(heading, _)) = string.split_once(after, "</th>")
  assert string.contains(heading, "Column")
  let page =
    element.to_string(
      strategy.view(step(model, strategy.SortBy(strategy.ByColumn))),
    )
  assert count(page, "aria-sort=\"descending\"") == 1
  assert count(page, "▼") == 1 && count(page, "▲") == 0
}

/// D-132: a row whose project's workflow offers `plan` in its column shows
/// Plan; a Planning row, and a row whose project's workflow isn't loaded,
/// don't. The reply moves the row; a refusal shows. The open task's details
/// are told they are on the Strategy page.
pub fn a_backlog_row_offers_plan_test() {
  let t = fn(id, title, column) {
    json.object([
      #("id", json.string(id)),
      #("title", json.string(title)),
      #("body", json.string("")),
      #("column", json.string(column)),
    ])
  }
  let tasks = fn(list) { #("tasks", json.array(list, fn(x) { x })) }
  let model =
    workspace()
    |> step(strategy.WorkflowLoaded(
      ".",
      "{\"columns\":[{\"id\":\"backlog\",\"label\":\"Backlog\",\"actions\":[\"plan\",\"cancel\"]},{\"id\":\"planning\",\"label\":\"Planning\",\"actions\":[\"to_backlog\"]}],\"actions\":{\"plan\":{\"to\":\"planning\",\"label\":\"Plan\"},\"to_backlog\":{\"to\":\"backlog\",\"label\":\"Back to backlog\"}}}",
    ))
    |> step(
      strategy.Live(
        message(".", [
          tasks([t("a", "Draft spec", "backlog"), t("b", "Ship it", "planning")]),
        ]),
      ),
    )
    |> step(
      strategy.Live(message("web", [tasks([t("c", "Web task", "backlog")])])),
    )
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "aria-label=\"Plan Draft spec\"")
  assert string.contains(page, ">Plan</button>")
  assert !string.contains(page, "aria-label=\"Plan Ship it\"")
  assert !string.contains(page, "aria-label=\"Plan Web task\"")
  let planned =
    model
    |> step(strategy.Planned(
      ".",
      json.to_string(t("a", "Draft spec", "planning")),
    ))
  let page = element.to_string(strategy.view(planned))
  assert !string.contains(page, "aria-label=\"Plan Draft spec\"")
  let refused =
    model
    |> step(strategy.Planned(
      ".",
      "{\"code\":\"E7001\",\"message\":\"not allowed\"}",
    ))
  assert refused.error != ""
  let opened =
    element.to_string(strategy.view(step(model, strategy.Open(".", "b"))))
  assert string.contains(opened, "hosted")
  assert string.contains(opened, "strategy")
  assert string.contains(opened, "task=\"b\"")
}

pub fn titles_sort_without_regard_to_case_test() {
  let row = fn(id, title) {
    #(
      "web",
      strategy.Row(
        id,
        title,
        "",
        "backlog",
        None,
        None,
        None,
        None,
        "",
        "",
        [],
        [],
      ),
    )
  }
  let rows = [row("1", "Zebra"), row("2", "add x"), row("3", "Apple")]
  let titles = fn(rows: List(#(String, strategy.Row))) {
    list.map(rows, fn(r) { { r.1 }.title })
  }
  assert titles(strategy.sorted(rows)) == ["add x", "Apple", "Zebra"]
  let model =
    workspace()
    |> step(strategy.SortBy(strategy.ByTitle))
  assert titles(strategy.sort_rows(model, rows)) == ["add x", "Apple", "Zebra"]
}

pub fn a_single_project_new_task_form_has_no_project_picker_test() {
  let one = fn(workspace) {
    strategy.new(strategy.Tasks)
    |> step(strategy.ProjectsLoaded(
      "{\"workspace\":"
      <> workspace
      <> ",\"projects\":[{\"dir\":\".\",\"label\":\"root\",\"name\":\"r\",\"base\":\"\"}]}",
    ))
    |> step(strategy.CaughtUp)
    |> step(strategy.NewTaskOpen)
  }
  let page = fn(model) { element.to_string(strategy.view(model)) }
  list.each(["false", "true"], fn(flag) {
    let html = page(one(flag))
    assert !string.contains(html, "name=\"project\"")
    assert string.contains(html, "name=\"feature\"")
    assert string.contains(html, "name=\"milestone\"")
  })
  // Two projects in a workspace do show it.
  assert string.contains(
    page(step(workspace(), strategy.NewTaskOpen)),
    "name=\"project\"",
  )
}

/// Unavailable models aren't offered: a line under the pick says why, an
/// icon beside the pick tries again, and with none that can start, what to
/// set up.
pub fn the_plan_offers_only_models_that_can_start_test() {
  let shown = fn(model) { element.to_string(strategy.view(model)) }
  let sol =
    "{\"id\":\"sol\",\"label\":\"Sol\",\"agent\":\"codex\",\"available\":false,\"reason\":\"Codex usage limit reached\",\"retry\":true}"
  let model =
    planned()
    |> step(strategy.ModelsLoaded(
      "[{\"id\":\"m\",\"label\":\"M\",\"model\":\"gpt-6-astra\",\"available\":true},"
      <> sol
      <> "]",
    ))
  assert string.contains(shown(model), ">M</option>")
  assert !string.contains(shown(model), "value=\"sol\"")
  assert string.contains(
    shown(model),
    "<summary>1 more model unavailable</summary>",
  )
  assert string.contains(
    shown(model),
    "<li>Sol: Codex usage limit reached</li>",
  )
  assert string.contains(shown(model), "aria-label=\"Try again\"")
  assert string.contains(shown(model), "⟳</button><select")
  assert !string.contains(shown(model), "No model can start yet")
  // None can start, every one held by a usage limit: the limits line, not
  // the install steps the agents are already past.
  let none = step(model, strategy.ModelsLoaded("[" <> sol <> "]"))
  assert string.contains(shown(none), "Every model is held by a usage limit")
  assert !string.contains(shown(none), "No model can start yet")
  assert !string.contains(shown(none), "claude.ai/install.sh")
  assert string.contains(shown(none), "<summary>1 model unavailable</summary>")
  assert string.contains(shown(none), "⟳</button>")
  // A model that isn't installed: the install steps stand.
  let setup =
    step(
      none,
      strategy.ModelsLoaded(
        "["
        <> sol
        <> ",{\"id\":\"luna\",\"label\":\"Luna\",\"agent\":\"claude\",\"available\":false,\"reason\":\"claude is not installed\",\"retry\":false}]",
      ),
    )
  assert string.contains(shown(setup), "No model can start yet")
  assert string.contains(shown(setup), "claude.ai/install.sh")
  // Try again's reply is the list as it is now: the model is offered again.
  let again = step(none, strategy.TryAgainModel)
  assert again.plan_refreshing
  let again =
    again
    |> step(strategy.ModelsLoaded(
      "[{\"id\":\"sol\",\"label\":\"Sol\",\"agent\":\"codex\",\"available\":true}]",
    ))
  assert !again.plan_refreshing
  assert string.contains(shown(again), ">Sol</option>")
  assert !string.contains(shown(again), "unavailable</summary>")
}

/// A plan run shows the one label of the model its agent used, when its start
/// event named one; an alias says which model it resolved to. Without a used
/// model it names its requested model, and an id the list lacks stays raw.
pub fn a_plan_run_names_the_model_it_used_test() {
  let model =
    step(
      planned(),
      strategy.ModelsLoaded(
        "[{\"id\":\"claude-opus\",\"label\":\"Opus (latest)\"},{\"id\":\"claude-opus-5-5\",\"label\":\"Opus 5.5\"}]",
      ),
    )
  let run = fn(used) {
    plan.Run(
      id: "r1",
      model: "claude-opus",
      effort: "",
      state: "ended",
      started: 1,
      ended: 2,
      outcome: "planned",
      context: plan.page,
      notes: [],
      questions: [],
      used: used,
    )
  }
  assert plan.state_text(run(""), model.models)
    == "Ended: planned · Opus (latest)"
  assert plan.state_text(run("claude-opus-5-5"), model.models)
    == "Ended: planned · Opus 5.5"
  assert plan.state_text(run("unknown"), model.models)
    == "Ended: planned · unknown"
  assert !string.contains(
    plan.state_text(run("claude-opus-5-5"), model.models),
    "claude-opus",
  )
  let renamed =
    step(
      model,
      strategy.ModelsRefreshed(
        "[{\"id\":\"claude-opus-5-5\",\"label\":\"Opus Renamed\"}]",
      ),
    )
  assert plan.state_text(run("claude-opus-5-5"), renamed.models)
    == "Ended: planned · Opus Renamed"
}

/// Cards take the shared card padding, small controls the small height, and
/// the page spaces its filters, table, band and headings with the tokens.
pub fn strategy_uses_the_shared_spacing_test() {
  let page = element.to_string(strategy.view(planned()))
  assert string.contains(page, "--control-h:36px")
  assert string.contains(
    page,
    ".feature{background:var(--paper);border:1px solid var(--edge);padding:var(--pad-card)",
  )
  assert string.contains(
    page,
    ".vision{background:var(--paper);border:1px solid var(--edge);padding:var(--pad-card)}",
  )
  assert string.contains(page, ".plan-bar select{")
  assert string.contains(page, ".plan p{margin:var(--space-1) 0}")
  // The plan panel's selects take the shared small-control declarations, and
  // add the padding and width that are theirs.
  assert string.contains(
    page,
    solace_task.small_control
      <> ";padding:var(--space-1) var(--space-2);max-width:100%}",
  )
  assert string.contains(
    page,
    ".filters{display:flex;flex-wrap:wrap;gap:var(--space-2) var(--space-4)",
  )
  assert string.contains(
    page,
    ".tasks th{text-align:left;font-size:12px;text-transform:uppercase;letter-spacing:.08em;padding:var(--space-2) var(--space-3)",
  )
  assert string.contains(
    page,
    ".detail-bar{display:flex;flex-wrap:wrap;gap:var(--space-1) var(--space-3)",
  )
  assert string.contains(
    page,
    ".milestone h3 .facts{display:flex;flex-wrap:wrap;align-items:baseline;gap:var(--space-2)",
  )
  assert string.contains(
    page,
    ".tasks th:nth-last-child(-n+2){text-align:right}",
  )
  assert string.contains(
    page,
    ".tabs{display:flex;background:var(--sheet);border-bottom:2px solid var(--edge);min-height:var(--tabs-height);box-sizing:border-box}",
  )
  assert string.contains(
    page,
    ".tasks .created,.tasks .closed{color:var(--soft);text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap}",
  )
  assert string.contains(
    page,
    ".tasks .created span,.tasks .closed span{display:block}",
  )
  assert !string.contains(page, ".tasks .created,.tasks .closed{display:flex")
  assert string.contains(page, ".tasks .planning{margin-left:var(--space-1)}")
}

/// The Tasks tab's search keeps a task whose id a query word names, bare or
/// as task:<id>, in any case; a word no id names keeps nothing.
pub fn the_search_finds_a_task_by_its_id_test() {
  let by = fn(query: String) {
    let filters = strategy.Filters("", "", "", "", query)
    step(many(30), strategy.Filter(filters)) |> ids
  }
  assert by("task:t27") == ["t27"]
  assert by("t27") == ["t27"]
  assert by("TASK:T27") == ["t27"]
  assert by("task:t31") == []
}

// A model list fetched again keeps the plan's pick; its effort stays only
// while the model still offers it, so a config edit that drops the effort
// leaves the plan at the model's default.
pub fn the_plan_keeps_only_an_effort_the_model_still_offers_test() {
  let chosen =
    planned()
    |> step(strategy.ModelsLoaded(
      "[{\"id\":\"m\",\"label\":\"M\",\"efforts\":[\"low\",\"high\"]}]",
    ))
    |> step(strategy.PlanModel("m"))
    |> step(strategy.PlanEffort("high"))
  assert chosen.plan_effort == "high"
  let kept =
    step(
      chosen,
      strategy.ModelsLoaded(
        "[{\"id\":\"m\",\"label\":\"M\",\"efforts\":[\"low\",\"high\"]}]",
      ),
    )
  assert kept.plan_model == "m"
  assert kept.plan_effort == "high"
  let dropped =
    step(
      chosen,
      strategy.ModelsLoaded(
        "[{\"id\":\"m\",\"label\":\"M\",\"efforts\":[\"low\"]}]",
      ),
    )
  assert dropped.plan_model == "m"
  assert dropped.plan_effort == ""
}

/// Start shows on a proposed or upcoming feature (D-154), and on one with no
/// state, which reads as proposed; a feature started or closed has none.
pub fn only_a_feature_not_started_shows_start_test() {
  let html =
    workspace()
    |> step(strategy.Live(
      "{\"features\":[{\"id\":\"f1\",\"title\":\"Idea\",\"state\":\"proposed\"},{\"id\":\"f2\",\"title\":\"Next\",\"state\":\"upcoming\"},{\"id\":\"f3\",\"title\":\"Blank\"},{\"id\":\"f4\",\"title\":\"Busy\",\"state\":\"active\"},{\"id\":\"f5\",\"title\":\"Done\",\"state\":\"complete\"},{\"id\":\"f6\",\"title\":\"Dropped\",\"state\":\"canceled\"}]}",
    ))
    |> step(strategy.Select(strategy.Features))
    |> strategy.view
    |> element.to_string
  let start = fn(title) {
    string.contains(html, "aria-label=\"Start " <> title <> "\"")
  }
  assert start("Idea")
  assert start("Next")
  assert start("Blank")
  assert !start("Busy")
  assert !start("Done")
  assert !start("Dropped")
}

pub fn a_dropped_feature_moves_at_once_and_back_if_refused_test() {
  let model =
    workspace()
    |> step(strategy.Live(
      "{\"features\":[{\"id\":\"f1\",\"title\":\"Search\",\"rank\":1},{\"id\":\"f2\",\"title\":\"Export\",\"rank\":2}]}",
    ))
    |> step(strategy.Select(strategy.Features))
  let html = element.to_string(strategy.view(model))
  assert string.contains(html, "aria-label=\"Reorder Search\"")
  assert string.contains(html, "class=\"grip\"")
  assert !string.contains(html, "Move up")
  let ranks = fn(m: strategy.Model) {
    list.map(m.features, fn(f) { #(f.id, f.rank) })
    |> list.sort(fn(a, b) { string.compare(a.0, b.0) })
  }
  // A cancelled drag moves nothing.
  assert ranks(step(model, strategy.DropFeature("f1", -1)))
    == [#("f1", 1.0), #("f2", 2.0)]
  let dropped = step(model, strategy.DropFeature("f1", 1))
  assert ranks(dropped) == [#("f1", 3.0), #("f2", 2.0)]
  let refused =
    step(
      dropped,
      strategy.FeatureMoved(
        "{\"error\":{\"code\":\"E7003\",\"message\":\"A run is open.\"}}",
      ),
    )
  assert ranks(refused) == [#("f1", 1.0), #("f2", 2.0)]
  assert refused.feature_error != ""
}

/// A Start sends one PATCH to `active`, and its reply answers the start
/// alone: the accepted feature shows as the server has it, and a refusal
/// says why and leaves the order a drag is answering exactly as it was.
pub fn a_start_answers_only_itself_test() {
  let model =
    workspace()
    |> step(strategy.Live(
      "{\"features\":[{\"id\":\"f1\",\"title\":\"Search\",\"rank\":1,\"state\":\"proposed\"},{\"id\":\"f2\",\"title\":\"Export\",\"rank\":2,\"state\":\"proposed\"}]}",
    ))
    |> step(strategy.Select(strategy.Features))
  let html = element.to_string(strategy.view(model))
  assert string.contains(
    html,
    "aria-label=\"" <> solace_task.ui_label("start") <> " Search\"",
  )
  let ranks = fn(m: strategy.Model) {
    list.map(m.features, fn(f) { #(f.id, f.rank) })
    |> list.sort(fn(a, b) { string.compare(a.0, b.0) })
  }
  // A drag is answered by a move, which keeps a save from racing it.
  let dropped = step(model, strategy.DropFeature("f1", 1))
  assert dropped.feature_saving
  assert ranks(dropped) == [#("f1", 3.0), #("f2", 2.0)]
  let started = step(dropped, strategy.StartFeature("f1"))
  // The card shows the state once the server answers, not before.
  assert started.features == dropped.features
  let answered =
    step(
      started,
      strategy.FeatureStarted(
        "{\"id\":\"f1\",\"title\":\"Search\",\"rank\":3,\"state\":\"active\",\"slot\":1}",
      ),
    )
  let assert [f, ..] = answered.features
  assert f.state == "active"
  assert answered.feature_error == ""
  // A start keeps the rank the drag set, and answers no other reply.
  assert ranks(answered) == [#("f1", 3.0), #("f2", 2.0)]
  // A refused start changes nothing: the order the drag set stays, and the
  // drag's own PATCH is still the one being answered.
  let refused =
    step(
      step(model, strategy.DropFeature("f1", 1)),
      strategy.FeatureStarted(
        "{\"error\":{\"code\":\"E7003\",\"message\":\"A run is open.\"}}",
      ),
    )
  assert ranks(refused) == [#("f1", 3.0), #("f2", 2.0)]
  assert refused.feature_error != ""
  assert refused.feature_saving
}

/// A move in flight holds the others: a second drop or arrow before the
/// first reply is ignored, so a refused first move restores its own rank.
pub fn a_second_move_waits_for_the_first_test() {
  let model =
    workspace()
    |> step(strategy.Live(
      "{\"features\":[{\"id\":\"f1\",\"title\":\"Search\",\"rank\":1,\"state\":\"proposed\"},{\"id\":\"f2\",\"title\":\"Export\",\"rank\":2,\"state\":\"proposed\"}]}",
    ))
    |> step(strategy.Select(strategy.Features))
  let ranks = fn(m: strategy.Model) {
    list.map(m.features, fn(f) { #(f.id, f.rank) })
    |> list.sort(fn(a, b) { string.compare(a.0, b.0) })
  }
  let dropped = step(model, strategy.DropFeature("f1", 1))
  let again =
    dropped
    |> step(strategy.DropFeature("f2", 0))
    |> step(strategy.MoveFeature("f2", 0.5))
  assert again == dropped
  let refused =
    step(
      again,
      strategy.FeatureMoved(
        "{\"error\":{\"code\":\"E7003\",\"message\":\"A run is open.\"}}",
      ),
    )
  assert ranks(refused) == [#("f1", 1.0), #("f2", 2.0)]
  assert !refused.feature_saving
  // The keyboard's move holds the others too.
  let moved = step(model, strategy.MoveFeature("f1", 3.0))
  assert moved.feature_saving
}

/// A fresh list chooses the first model that can start; a refresh keeps a pick
/// the list still has, even when it can't start, and narrows its effort. A
/// failed refresh changes nothing.
pub fn the_plan_models_follow_refreshes_test() {
  let shown = fn(model) { element.to_string(strategy.view(model)) }
  let initial =
    step(
      planned(),
      strategy.ModelsLoaded(
        "[{\"id\":\"a\",\"label\":\"A\",\"available\":false,\"reason\":\"not installed\"},{\"id\":\"b\",\"label\":\"B\",\"efforts\":[\"low\",\"high\"]}]",
      ),
    )
  assert initial.plan_model == "b"
  let chosen =
    initial
    |> step(strategy.PlanModel("b"))
    |> step(strategy.PlanEffort("high"))
  let failed = step(chosen, strategy.ModelsRefreshed("oops"))
  assert failed.models == chosen.models
  assert failed.plan_model == "b"
  assert failed.plan_effort == "high"
  let held =
    step(
      chosen,
      strategy.ModelsRefreshed(
        "[{\"id\":\"a\",\"label\":\"A\"},{\"id\":\"b\",\"label\":\"B\",\"agent\":\"codex\",\"available\":false,\"reason\":\"Codex usage limit reached\",\"retry\":true,\"efforts\":[\"low\",\"high\"]}]",
      ),
    )
  assert held.plan_model == "b"
  assert held.plan_effort == "high"
  assert string.contains(shown(held), "B: Codex usage limit reached")
  assert string.contains(shown(held), "aria-label=\"Try again\"")
  let narrowed =
    step(
      held,
      strategy.ModelsRefreshed(
        "[{\"id\":\"a\",\"label\":\"A\"},{\"id\":\"b\",\"label\":\"B\",\"efforts\":[\"low\"]}]",
      ),
    )
  assert narrowed.plan_model == "b"
  assert narrowed.plan_effort == ""
  let dropped =
    step(narrowed, strategy.ModelsRefreshed("[{\"id\":\"a\",\"label\":\"A\"}]"))
  assert dropped.plan_model == "a"
  assert dropped.plan_effort == ""
}

/// The working state and Earlier runs name the same one model label: the model
/// a run used when its start event named one, else its requested model.
pub fn the_working_state_and_earlier_runs_show_one_model_label_test() {
  let model =
    planned()
    |> step(
      strategy.Live(
        json.to_string(
          json.object([
            #(
              "strategyRun",
              run("r5", "running", #("page", ""), 40, [
                #("used", json.string("claude-opus-5-5")),
              ]),
            ),
          ]),
        ),
      ),
    )
    |> step(strategy.ModelsLoaded(
      "[{\"id\":\"fake\",\"label\":\"Fake model\"},{\"id\":\"claude-opus\",\"label\":\"Opus (latest)\"},{\"id\":\"claude-opus-5-5\",\"label\":\"Opus 5.5\"}]",
    ))
    |> step(strategy.ShowPlan("r5"))
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "Working · Opus 5.5")
  assert string.contains(page, "Ended: planned · Fake model")
  assert !string.contains(page, "fake")
  assert !string.contains(page, "claude-opus")
  let collapsed = step(model, strategy.PlanCollapse)
  let page = element.to_string(strategy.view(collapsed))
  assert string.contains(page, "Working · Opus 5.5")
  assert !string.contains(page, "claude-opus")
}

/// Live strategy-run and task updates refresh the model list; feature, vision
/// and workflow updates do not.
pub fn live_run_and_task_updates_refresh_the_models_test() {
  assert strategy.models_event("{\"strategyRun\":{\"id\":\"r\"}}")
  assert strategy.models_event("{\"strategyRuns\":[]}")
  assert strategy.models_event("{\"task\":{\"id\":\"t\"}}")
  assert strategy.models_event("{\"tasks\":[]}")
  assert strategy.models_event("{\"removed\":\"t\"}")
  assert !strategy.models_event("{\"feature\":{\"id\":\"f\"}}")
  assert !strategy.models_event("{\"vision\":{\"text\":\"\"}}")
  assert !strategy.models_event("{\"workflow\":{}}")
}

pub fn a_milestone_feature_query_and_target_replies_test() {
  let model =
    workspace()
    |> step(strategy.Live(
      "{\"milestones\":[{\"id\":\"m1\",\"title\":\"Launch\"}]}",
    ))
    |> step(strategy.Live(
      "{\"features\":[{\"id\":\"f1\",\"title\":\"Search\",\"milestone\":\"m1\"}]}",
    ))
    |> step(strategy.EditMilestone("m1"))
    |> step(strategy.MilestoneFeatureQuery("sea"))
  assert model.milestone_feature_query == "sea"
  assert model.milestone_draft != None
  let added =
    step(
      model,
      strategy.FeatureTargeted(
        "{\"id\":\"f1\",\"title\":\"Search\",\"milestone\":\"m1\"}",
      ),
    )
  assert added.milestone_error == ""
  assert added.milestone_draft != None
  assert added.features
    == [features.Feature("f1", "Search", "", Some("m1"), 0.0, "proposed", None)]
  let removed =
    step(
      added,
      strategy.FeatureTargeted(
        "{\"id\":\"f1\",\"title\":\"Search\",\"milestone\":null}",
      ),
    )
  assert removed.features
    == [features.Feature("f1", "Search", "", None, 0.0, "proposed", None)]
  let refused =
    step(
      removed,
      strategy.FeatureTargeted(
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown milestone m9\"}",
      ),
    )
  assert refused.milestone_error == "E7023 unknown milestone m9"
  assert step(model, strategy.CancelMilestone).milestone_feature_query == ""
}

pub fn a_feature_link_or_edit_opens_its_milestone_and_a_heading_toggles_it_test() {
  let model =
    workspace()
    |> step(strategy.Live(
      "{\"features\":[{\"id\":\"f1\",\"title\":\"Search\",\"milestone\":\"m1\"}]}",
    ))
    |> step(strategy.Live(
      "{\"milestones\":[{\"id\":\"m1\",\"title\":\"Launch\"},{\"id\":\"m2\",\"title\":\"Later\"}]}",
    ))
    |> step(
      strategy.Live(
        message(".", [
          #("tasks", json.array([task_in("t2", "f1", "m2")], fn(t) { t })),
        ]),
      ),
    )
  assert model.milestone_open == []
  let #(linked, _) = strategy.update(model, strategy.Focus("feature", "", "f1"))
  assert linked.tab == strategy.Features
  assert linked.milestone_open == ["m1"]
  let edited = step(model, strategy.EditMilestone("m1"))
  assert edited.milestone_open == ["m1"]
  let closed = step(edited, strategy.ToggleMilestone("m1"))
  assert closed.milestone_open == []
  let opened = step(closed, strategy.ToggleMilestone("m1"))
  assert opened.milestone_open == ["m1"]
  let removed =
    step(
      opened,
      strategy.MilestoneDeleted("{\"id\":\"m1\",\"title\":\"Launch\"}"),
    )
  assert removed.milestone_open == []
  assert strategy.new(strategy.Roadmap).milestone_open == []
}

fn task_in(id: String, feature: String, milestone: String) -> json.Json {
  json.object([
    #("id", json.string(id)),
    #("title", json.string(id)),
    #("body", json.string("")),
    #("column", json.string("backlog")),
    #("feature", json.string(feature)),
    #("milestone", json.string(milestone)),
    #("dependsOn", json.array([], json.string)),
    #("createdTime", json.int(60)),
    #("closedTime", json.null()),
  ])
}

pub fn a_second_target_waits_for_the_first_test() {
  let model =
    workspace()
    |> step(strategy.Live(
      "{\"milestones\":[{\"id\":\"m1\",\"title\":\"Launch\"},{\"id\":\"m2\",\"title\":\"Next\"}]}",
    ))
    |> step(strategy.Live(
      "{\"features\":[{\"id\":\"f1\",\"title\":\"One\",\"milestone\":\"m1\"},{\"id\":\"f2\",\"title\":\"Two\",\"milestone\":\"m1\"}]}",
    ))
    |> step(strategy.EditMilestone("m1"))
  let first = step(model, strategy.TargetFeature("f1", "m2"))
  assert first.milestone_targeting
  let second = step(first, strategy.TargetFeature("f2", "m2"))
  assert second == first
  let refused =
    step(
      second,
      strategy.FeatureTargeted(
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown milestone m9\"}",
      ),
    )
  assert !refused.milestone_targeting
  assert refused.milestone_error == "E7023 unknown milestone m9"
  let replied =
    step(
      first,
      strategy.FeatureTargeted(
        "{\"id\":\"f1\",\"title\":\"One\",\"milestone\":\"m2\"}",
      ),
    )
  assert !replied.milestone_targeting
  assert replied.milestone_error == ""
}

/// A focused feature's tasks show the run transcript's marks: a green check
/// for a complete task, a red cross for a canceled one, and neither for an
/// open task.
pub fn a_focused_features_tasks_show_complete_and_canceled_marks_test() {
  let model =
    workspace()
    |> step(
      strategy.Live(
        message(".", [
          #(
            "tasks",
            json.preprocessed_array([
              task("t1", "Done", "", "complete", "f1", 10),
              task("t2", "Stopped", "", "cancelled", "f1", 20),
              task("t3", "Open", "", "backlog", "f1", 0),
            ]),
          ),
        ]),
      ),
    )
    |> step(strategy.Live(
      "{\"features\":[{\"id\":\"f1\",\"title\":\"Search\",\"body\":\"Find things fast\",\"milestone\":\"m1\"}]}",
    ))
    |> step(strategy.Live(
      "{\"milestones\":[{\"id\":\"m1\",\"title\":\"Launch\"}]}",
    ))
    |> step(strategy.Select(strategy.Features))
    |> step(strategy.Focus("feature", "", "f1"))
  let html = element.to_string(strategy.view(model))
  let row = fn(title) {
    let assert Ok(found) =
      string.split(html, "<li")
      |> list.find(fn(row) { string.contains(row, ">" <> title <> "</button>") })
    found
  }
  let complete = row("Done")
  assert string.contains(complete, "tool ok")
  assert string.contains(complete, "aria-label=\"Complete\"")
  assert string.contains(complete, "✓")
  assert !string.contains(complete, "✗")
  let canceled = row("Stopped")
  assert string.contains(canceled, "tool failed")
  assert string.contains(canceled, "aria-label=\"Canceled\"")
  assert string.contains(canceled, "✗")
  assert !string.contains(canceled, "✓")
  let open = row("Open")
  assert !string.contains(open, "class=\"tool")
  assert !string.contains(open, "✓")
  assert !string.contains(open, "✗")
  assert string.contains(html, ".tool.ok .icon{color:var(--ok-text)}")
  assert string.contains(html, ".tool.failed .icon{color:var(--revision)}")
}

fn task_list(n: Int) -> List(json.Json) {
  numbers(1, n)
  |> list.map(fn(i) {
    task("t" <> int.to_string(i), title_of(i), "", "backlog", "", 0)
  })
}

pub fn a_live_update_that_shrinks_the_list_shows_a_page_with_rows_test() {
  let model = many(26) |> step(strategy.Page(3))
  let model =
    step(
      model,
      strategy.Live(
        message(".", [#("tasks", json.array(task_list(11), fn(t) { t }))]),
      ),
    )
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "task-11")
  assert !string.contains(page, "task-10")
  assert string.contains(page, "page 2 of 2, 11 tasks")
}

pub fn the_filter_comboboxes_match_each_field_and_order_by_title_test() {
  let model =
    sortable()
    |> step(strategy.ProjectsLoaded(
      "{\"workspace\":true,\"projects\":[{\"dir\":\".\",\"label\":\"Root\",\"name\":\"r\",\"base\":\"\"},{\"dir\":\"web\",\"label\":\"Site\",\"name\":\"web\",\"base\":\"/p/web\"}]}",
    ))
  let found = fn(field, text) {
    list.map(strategy.filter_suggestions(model, field, text), fn(item) {
      item.id
    })
  }
  // A project matches its folder or its label; features and milestones match
  // their titles or ids.
  assert found(strategy.ProjectField, "web") == ["web"]
  assert found(strategy.ProjectField, "SIT") == ["web"]
  assert found(strategy.FeatureField, "f2") == ["f2"]
  assert found(strategy.FeatureField, "alp") == ["f2"]
  assert found(strategy.MilestoneField, "m2") == ["m2"]
  assert found(strategy.MilestoneField, "soon") == ["m2"]
  // The empty query lists each filter's choices by title, with the no-link
  // and no-project choices kept.
  assert found(strategy.ProjectField, "") == ["no-project", ".", "web"]
  assert found(strategy.FeatureField, "") == ["-", "f2", "f1"]
  assert found(strategy.MilestoneField, "") == ["-", "m1", "m2"]
}

pub fn a_filter_combobox_chooses_clears_and_shows_no_matches_test() {
  let model = sortable()
  let typed = step(model, strategy.FilterText(strategy.FeatureField, "f1"))
  assert typed.filters.feature == ""
  let chosen =
    typed
    |> step(strategy.FilterKey(strategy.FeatureField, "ArrowDown"))
    |> step(strategy.FilterKey(strategy.FeatureField, "Enter"))
  assert chosen.filters.feature == "f1"
  assert ids(chosen) == ["c", "a"]
  assert string.contains(
    element.to_string(strategy.view(chosen)),
    "value=\"Beta\"",
  )
  let missing =
    step(chosen, strategy.FilterText(strategy.MilestoneField, "zzz"))
  assert missing.filters.milestone == ""
  assert string.contains(
    element.to_string(strategy.view(missing)),
    "No matches.",
  )
  let escaped =
    step(missing, strategy.FilterKey(strategy.MilestoneField, "Escape"))
  assert escaped.filters.milestone == ""
  assert !string.contains(
    element.to_string(strategy.view(escaped)),
    "No matches.",
  )
  let blurred =
    step(
      step(chosen, strategy.FilterText(strategy.FeatureField, "alp")),
      strategy.FilterBlur(strategy.FeatureField),
    )
  assert blurred.filters.feature == "f1"
  let page = element.to_string(strategy.view(blurred))
  assert string.contains(page, "value=\"Beta\"")
  assert string.contains(page, "aria-expanded=\"false\"")
  assert !string.contains(page, "No matches.")
  let clicked =
    step(model, strategy.FilterChoose(strategy.MilestoneField, "m2"))
  assert clicked.filters.milestone == "m2"
  assert ids(clicked) == ["c"]
  let cleared = step(chosen, strategy.FilterText(strategy.FeatureField, ""))
  assert cleared.filters.feature == ""
  assert list.length(ids(cleared)) == 5
}

pub fn the_filter_combobox_navigates_and_marks_the_active_option_test() {
  let model =
    sortable() |> step(strategy.FilterText(strategy.FeatureField, "a"))
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "role=\"combobox\"")
  assert string.contains(page, "role=\"listbox\"")
  assert string.contains(page, "role=\"option\"")
  assert string.contains(page, "class=\"options\"")
  assert string.contains(page, "for=\"filter-feature-input\"")
  assert string.contains(page, ">Feature</label>")
  assert string.contains(page, "class=\"choice\"")
  assert string.contains(page, "aria-expanded=\"true\"")
  let model =
    step(model, strategy.FilterKey(strategy.FeatureField, "ArrowDown"))
  let page = element.to_string(strategy.view(model))
  assert string.contains(
    page,
    "aria-activedescendant=\"filter-feature-option-0\"",
  )
  let model =
    step(model, strategy.FilterKey(strategy.FeatureField, "ArrowDown"))
  let page = element.to_string(strategy.view(model))
  assert string.contains(
    page,
    "aria-activedescendant=\"filter-feature-option-1\"",
  )
  let model = step(model, strategy.FilterKey(strategy.FeatureField, "ArrowUp"))
  let page = element.to_string(strategy.view(model))
  assert string.contains(
    page,
    "aria-activedescendant=\"filter-feature-option-0\"",
  )
  let chosen = step(model, strategy.FilterKey(strategy.FeatureField, "Enter"))
  assert chosen.filters.feature == "f2"
  assert ids(chosen) == ["b"]
  let opened = step(chosen, strategy.FilterOpen(strategy.FeatureField))
  let page = element.to_string(strategy.view(opened))
  assert string.contains(
    page,
    "aria-activedescendant=\"filter-feature-option-1\"",
  )
  assert string.contains(page, "aria-selected=\"true\"")
  let closed = step(chosen, strategy.FilterBlur(strategy.FeatureField))
  let page = element.to_string(strategy.view(closed))
  assert string.contains(page, "aria-controls=\"filter-feature-list\"")
  assert string.contains(page, "id=\"filter-feature-list\"")
  assert string.contains(page, "aria-expanded=\"false\"")
}

pub fn duplicate_filter_titles_stay_distinct_by_id_test() {
  let model =
    sortable()
    |> step(strategy.Live(
      "{\"features\":[{\"id\":\"f1\",\"title\":\"Same\"},{\"id\":\"f2\",\"title\":\"Same\"}]}",
    ))
    |> step(strategy.Live(
      "{\"milestones\":[{\"id\":\"m1\",\"title\":\"Same\"},{\"id\":\"m2\",\"title\":\"Same\"}]}",
    ))
  let ids_for = fn(field) {
    list.map(strategy.filter_suggestions(model, field, "same"), fn(item) {
      item.id
    })
  }
  assert ids_for(strategy.FeatureField) == ["f1", "f2"]
  assert ids_for(strategy.MilestoneField) == ["m1", "m2"]
  let first = step(model, strategy.FilterChoose(strategy.FeatureField, "f1"))
  let second = step(model, strategy.FilterChoose(strategy.FeatureField, "f2"))
  assert ids(first) == ["c", "a"]
  assert ids(second) == ["b"]
  let page =
    model
    |> step(strategy.FilterOpen(strategy.FeatureField))
    |> strategy.view
    |> element.to_string
  assert string.contains(page, "class=\"hint\">f1")
  assert string.contains(page, "class=\"hint\">f2")
}

pub fn the_query_combines_with_all_three_filters_test() {
  let model =
    sortable()
    |> step(strategy.FilterChoose(strategy.ProjectField, "web"))
    |> step(strategy.FilterChoose(strategy.FeatureField, "f1"))
    |> step(strategy.FilterChoose(strategy.MilestoneField, "m2"))
  let matching =
    step(
      model,
      strategy.Filter(strategy.Filters(..model.filters, query: "fix")),
    )
  assert ids(matching) == ["c"]
  let none =
    step(
      model,
      strategy.Filter(strategy.Filters(..model.filters, query: "add")),
    )
  assert ids(none) == []
}

/// The selected task's band shows its title and toggle, hides the details
/// when collapsed, and expands them again.
pub fn the_task_band_collapses_and_expands_test() {
  let model = many(3) |> step(strategy.Open(".", "t2"))
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "class=\"detail-band\"")
  assert string.contains(page, "aria-expanded=\"true\"")
  assert string.contains(page, "aria-controls=\"task-details\"")
  assert string.contains(
    page,
    "aria-label=\"Collapse the task details: task-02\"",
  )
  assert string.contains(page, "id=\"task-details\"")
  assert string.contains(page, "<solace-task bar")
  let assert Ok(#(_, bar)) =
    string.split_once(page, "<header class=\"detail-bar\"")
  let assert Ok(#(bar, _)) = string.split_once(bar, "</header>")
  assert string.contains(bar, "task-02")
  assert string.contains(bar, "class=\"meta\"")
  assert string.contains(bar, "backlog")
  let model = step(model, strategy.ToggleTask)
  assert model.task_collapsed
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "class=\"detail-band collapsed\"")
  assert string.contains(page, "aria-expanded=\"false\"")
  assert string.contains(
    page,
    "aria-label=\"Expand the task details: task-02\"",
  )
  assert string.contains(page, "<solace-task bar")
  assert string.contains(
    page,
    ".detail-band.collapsed>solace-task{height:0 !important;overflow:hidden;visibility:hidden}",
  )
  let model = step(model, strategy.ToggleTask)
  assert !model.task_collapsed
  let page = element.to_string(strategy.view(model))
  assert !string.contains(page, "class=\"detail-band collapsed\"")
  assert string.contains(page, "aria-expanded=\"true\"")
}

/// With no selected task the Tasks tab has no band.
pub fn the_tasks_tab_has_no_band_without_a_selection_test() {
  let page = many(3) |> strategy.view |> element.to_string
  assert !string.contains(page, "class=\"detail-band")
  assert !string.contains(page, "<solace-task")
}

/// Selecting another task, or a task link, while the band is collapsed
/// updates the band and expands it.
pub fn selecting_a_task_expands_its_band_test() {
  let model =
    many(3)
    |> step(strategy.Open(".", "t1"))
    |> step(strategy.ToggleTask)
  assert model.task_collapsed
  let model = step(model, strategy.Open(".", "t2"))
  assert model.selected == Some(#(".", "t2"))
  assert !model.task_collapsed
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "aria-expanded=\"true\"")
  assert string.contains(page, "task=\"t2\"")
  let model = step(model, strategy.ToggleTask)
  let model = step(model, strategy.Focus("task", ".", "t3"))
  assert model.selected == Some(#(".", "t3"))
  assert !model.task_collapsed
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "aria-expanded=\"true\"")
  assert string.contains(page, "task=\"t3\"")
}

/// Paging changes the rows under the band; the selected task and whether its
/// band is collapsed stay as they were.
pub fn the_selected_task_stays_in_the_band_across_pages_test() {
  let model =
    many(30)
    |> step(strategy.Open(".", "t3"))
    |> step(strategy.ToggleTask)
    |> step(strategy.Page(2))
  assert model.selected == Some(#(".", "t3"))
  assert model.task_collapsed
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "page 2 of 3, 30 tasks")
  assert string.contains(page, "class=\"detail-band collapsed\"")
  assert string.contains(page, "task=\"t3\"")
  let assert Ok(#(_, bar)) =
    string.split_once(page, "<header class=\"detail-bar\"")
  let assert Ok(#(bar, _)) = string.split_once(bar, "</header>")
  assert string.contains(bar, "task-03")
}

/// The collapsed bar names the row as it is now, and falls back to the id
/// when its row is not loaded.
pub fn the_task_bar_falls_back_to_the_id_and_follows_a_live_rename_test() {
  let model =
    many(3)
    |> step(strategy.Open(".", "t2"))
    |> step(strategy.ToggleTask)
  let renamed =
    strategy.Live(
      message(".", [
        #(
          "task",
          json.object([
            #("id", json.string("t2")),
            #("title", json.string("Renamed")),
            #("column", json.string("done")),
            #("finished", json.object([#("outcome", json.string("finished"))])),
          ]),
        ),
      ]),
    )
  let model = step(model, renamed)
  let page = element.to_string(strategy.view(model))
  let assert Ok(#(_, bar)) =
    string.split_once(page, "<header class=\"detail-bar\"")
  let assert Ok(#(bar, _)) = string.split_once(bar, "</header>")
  assert string.contains(bar, "Renamed")
  assert string.contains(bar, "Agent finished")
  let model = strategy.Model(..model, rows: [])
  let page = element.to_string(strategy.view(model))
  let assert Ok(#(_, bar)) =
    string.split_once(page, "<header class=\"detail-bar\"")
  let assert Ok(#(bar, _)) = string.split_once(bar, "</header>")
  assert string.contains(bar, "t2")
}

/// The Tasks panel's display rules apply only while it is shown, and its
/// narrow layout keeps the table and band in the same height contract.
pub fn the_tasks_panel_is_hidden_on_another_tab_and_keeps_its_height_test() {
  let model =
    many(3)
    |> step(strategy.Open(".", "t1"))
    |> step(strategy.Select(strategy.Vision))
  let page = element.to_string(strategy.view(model))
  let assert Ok(#(before, _)) = string.split_once(page, "id=\"panel-tasks\"")
  let tag = string.split(before, "<section") |> list.last |> result.unwrap("")
  assert string.contains(tag, "hidden")
  assert string.contains(
    page,
    ".strategy>[role=tabpanel][hidden]{display:none}",
  )
  assert string.contains(
    page,
    ".strategy>#panel-tasks:not([hidden]){display:flex",
  )
  assert !string.contains(page, ".strategy>#panel-tasks{display:flex")
  assert string.contains(
    page,
    ".strategy>#panel-tasks:not([hidden]){display:flex;height:calc(var(--view-height) - var(--tabs-height))",
  )
  assert string.contains(
    page,
    ".tabs{display:flex;background:var(--sheet);border-bottom:2px solid var(--edge);min-height:var(--tabs-height);box-sizing:border-box}",
  )
  assert string.contains(
    page,
    "--view-height:calc(100dvh - var(--shell-height))",
  )
  assert !string.contains(
    page,
    ".strategy>#panel-tasks>.tasks-scroll{overflow:visible}",
  )
}

/// The Features tab after the stream sends these records.
fn features_page(milestones: String, features: String) -> String {
  features_model(milestones, features) |> features_panel
}

pub fn closed_features_leave_their_milestone_and_show_once_test() {
  let page =
    features_page(
      "{\"milestones\":[{\"id\":\"m1\",\"title\":\"Launch\",\"rank\":1}]}",
      "{\"features\":[{\"id\":\"open\",\"title\":\"Open\",\"milestone\":\"m1\",\"rank\":1,\"state\":\"active\"},{\"id\":\"done\",\"title\":\"Done\",\"milestone\":\"m1\",\"rank\":3,\"state\":\"complete\"},{\"id\":\"dropped\",\"title\":\"Dropped\",\"rank\":2,\"state\":\"canceled\"}]}",
    )
  let count = fn(page, needle) { list.length(string.split(page, needle)) - 1 }
  assert in_order(page, ["Launch", "Open", "Closed", "Dropped", "Done"])
  assert count(page, "data-feature-id=\"done\"") == 1
  assert count(page, "data-feature-id=\"dropped\"") == 1
  assert string.contains(page, "class=\"state state-complete\">complete<")
  assert string.contains(page, "class=\"state state-canceled\">canceled<")
}

pub fn a_page_without_closed_features_has_no_closed_section_test() {
  let page =
    features_page(
      "{\"milestones\":[{\"id\":\"m1\",\"title\":\"Launch\",\"rank\":1}]}",
      "{\"features\":[{\"id\":\"open\",\"title\":\"Open\",\"milestone\":\"m1\",\"state\":\"active\"}]}",
    )
  assert string.contains(page, ">Open <span")
  assert !string.contains(page, ">Closed<")
}

pub fn a_page_with_every_feature_closed_stays_usable_test() {
  let model =
    features_model(
      "{\"milestones\":[{\"id\":\"m1\",\"title\":\"Launch\",\"rank\":1},{\"id\":\"m2\",\"title\":\"Later\",\"rank\":2}]}",
      "{\"features\":[{\"id\":\"done\",\"title\":\"Done\",\"milestone\":\"m1\",\"rank\":2,\"state\":\"complete\"},{\"id\":\"dropped\",\"title\":\"Dropped\",\"rank\":1,\"state\":\"canceled\"}]}",
    )
  let page = features_panel(model)
  assert in_order(page, [
    "Launch",
    "No features.",
    "Later",
    "No features.",
    "Closed",
    "Dropped",
    "Done",
  ])
  assert string.contains(page, "aria-label=\"Delete Dropped\"")
  assert string.contains(page, "class=\"edit\"")
  assert !string.contains(page, ">No milestone<")
  let editing = step(model, strategy.EditFeature("dropped"))
  let form = features_panel(editing)
  let count = fn(page, needle) { list.length(string.split(page, needle)) - 1 }
  assert string.contains(form, "class=\"feature-form\"")
  assert count(form, "data-feature-id=\"dropped\"") == 1
  assert string.contains(form, "id=\"feature-form\"")
  assert string.contains(form, "value=\"Dropped\"")
  assert string.contains(form, ">▾ Edit Dropped</button>")
}

fn features_model(milestones: String, features: String) -> strategy.Model {
  workspace()
  |> step(strategy.Live(milestones))
  |> step(strategy.Live(features))
  |> step(strategy.Select(strategy.Features))
}

fn features_panel(model: strategy.Model) -> String {
  let page = model |> strategy.view |> element.to_string
  let assert Ok(#(_, after)) = string.split_once(page, "id=\"panel-features\"")
  let assert Ok(#(features, _)) =
    string.split_once(after, "<section aria-labelledby=\"tab-roadmap\"")
  features
}

/// New feature and Edit open the bottom band expanded, and a collapsed band
/// keeps the draft it shows when reopened.
pub fn the_feature_band_opens_expanded_and_keeps_its_draft_test() {
  let model =
    features_model(
      "{\"milestones\":[{\"id\":\"m1\",\"title\":\"Launch\",\"rank\":1}]}",
      "{\"features\":[{\"id\":\"f1\",\"title\":\"Search\",\"milestone\":\"m1\"}]}",
    )
  let page = fn(model) { features_panel(model) }
  let new = step(model, strategy.NewFeature)
  assert !new.feature_collapsed
  let opened = page(new)
  assert string.contains(opened, "class=\"detail-band\"")
  assert string.contains(opened, "aria-expanded=\"true\"")
  assert string.contains(opened, "aria-controls=\"feature-form\"")
  assert string.contains(
    opened,
    "aria-label=\"Collapse the feature details: New feature\"",
  )
  assert string.contains(opened, "id=\"feature-form\"")
  let assert Ok(#(stack, band)) =
    string.split_once(opened, "<div class=\"detail-band\"")
  assert string.contains(stack, "class=\"features-scroll\"")
  assert !string.contains(stack, "id=\"feature-text\"")
  assert string.contains(band, "id=\"feature-text\"")
  let typed = step(new, strategy.FeatureTitle("Export"))
  let collapsed = step(typed, strategy.ToggleFeature)
  assert collapsed.feature_collapsed
  assert collapsed.feature_draft
    == Some(features.Draft(None, "Export", "", "proposed", None))
  let folded = page(collapsed)
  assert string.contains(folded, "class=\"detail-band collapsed\"")
  assert string.contains(folded, "aria-expanded=\"false\"")
  assert string.contains(
    folded,
    "aria-label=\"Expand the feature details: New feature\"",
  )
  assert string.contains(folded, "value=\"Export\"")
  assert string.contains(
    element.to_string(strategy.view(collapsed)),
    ".detail-band.collapsed>.feature-form{display:none}",
  )
  let reopened = step(collapsed, strategy.ToggleFeature)
  assert !reopened.feature_collapsed
  assert reopened.feature_draft == collapsed.feature_draft
  assert string.contains(page(reopened), "value=\"Export\"")
  let editing =
    reopened
    |> step(strategy.CancelFeature)
    |> step(strategy.EditFeature("f1"))
  assert !editing.feature_collapsed
  let editing_page = page(editing)
  assert string.contains(
    editing_page,
    "aria-label=\"Collapse the feature details: Edit Search\"",
  )
  let assert Ok(#(stack, band)) =
    string.split_once(editing_page, "<div class=\"detail-band\"")
  assert string.contains(stack, "data-feature-id=\"f1\"")
  assert !string.contains(stack, "id=\"feature-text\"")
  assert string.contains(band, "value=\"Search\"")
  let typing = step(editing, strategy.FeatureTitle("Export"))
  assert string.contains(
    page(typing),
    "aria-label=\"Collapse the feature details: Edit Search\"",
  )
  assert !string.contains(page(typing), "Edit Export")
}

/// A refusal stays in the band, and Cancel closes it and clears the draft.
pub fn cancel_closes_the_feature_band_and_a_refusal_stays_in_it_test() {
  let model =
    features_model("", "{\"features\":[{\"id\":\"f1\",\"title\":\"Search\"}]}")
    |> step(strategy.EditFeature("f1"))
    |> step(strategy.FeatureMilestone("m9"))
  let refused =
    step(
      model,
      strategy.FeatureSaved(
        "{\"code\":\"E7023\",\"message\":\"E7023 unknown milestone m9\"}",
      ),
    )
  assert refused.feature_draft != None
  let band = fn(page) {
    let assert Ok(#(_, band)) =
      string.split_once(page, "<div class=\"detail-band\"")
    band
  }
  let shown = band(features_panel(refused))
  assert string.contains(shown, "role=\"alert\"")
  assert string.contains(shown, "E7023 unknown milestone m9")
  let cancelled = step(refused, strategy.CancelFeature)
  assert cancelled.feature_draft == None
  let page = features_panel(cancelled)
  assert !string.contains(page, "class=\"detail-band")
  assert !string.contains(page, "id=\"feature-text\"")
}

/// The feature stack scrolls in the space the band leaves, and the narrow
/// layout has no fixed page height.
pub fn the_features_stack_scrolls_under_the_band_test() {
  let model =
    features_model("", "{\"features\":[{\"id\":\"f1\",\"title\":\"Search\"}]}")
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "class=\"features-scroll\"")
  assert string.contains(
    page,
    ".strategy>#panel-features:not([hidden]){display:flex;flex-direction:column;overflow:hidden;padding-bottom:0}",
  )
  assert string.contains(
    page,
    ".strategy>#panel-features:not([hidden])>.features-scroll{flex:1;min-height:0;overflow:auto;overscroll-behavior:contain;scrollbar-gutter:stable}",
  )
  assert string.contains(
    page,
    ".detail-band>.feature-form{flex:none;max-height:min(45dvh,calc(100dvh - 190px));overflow-y:auto;margin:0}",
  )
  assert string.contains(
    page,
    ".strategy>#panel-features:not([hidden]){display:flex;flex-direction:column;overflow:visible;padding:var(--space-6);padding-bottom:0}.strategy>#panel-features:not([hidden])>.features-scroll{flex:none;overflow:visible}",
  )
  assert !string.contains(
    page,
    ".strategy>#panel-features:not([hidden]){display:flex;height:calc",
  )
}

/// D-189: a tab change is shown, and the shell's tab change restores it without writing another one.
pub fn a_tab_change_and_history_restore_the_tab_test() {
  let initial = strategy.new(strategy.Vision)
  let model = strategy.Model(..initial, page: 3)
  let #(model, _) = strategy.update(model, strategy.Select(strategy.Features))
  assert model.tab == strategy.Features
  assert model.page == 3
  let #(model, _) = strategy.update(model, strategy.TabShown(strategy.Roadmap))
  assert model.tab == strategy.Roadmap
  assert model.page == 3
  let #(same, _) = strategy.update(model, strategy.Select(strategy.Roadmap))
  assert same.tab == strategy.Roadmap
  assert same.page == 3
}

/// Outside a browser registering reports the error instead of crashing.
pub fn register_outside_browser_test() {
  assert result.is_error(strategy.register())
}

/// A Created or Closed cell holds its date above its time, so the column can
/// narrow; the raw seconds still sort the row.
pub fn the_date_columns_stack_their_date_and_time_test() {
  let page = element.to_string(strategy.view(sortable()))
  assert string.contains(page, "<td class=\"created\"><span class=\"date\">")
  assert string.contains(page, "<td class=\"closed\"><span class=\"date\">")
  assert string.contains(page, "</span><span class=\"time\">")
}

/// A task's milestone cell links its record: the loaded title, the id until
/// it loads, and a dash when there is none. The Feature selector stays.
pub fn a_milestone_cell_links_its_record_test() {
  let task = fn(id, title, milestone) {
    json.object([
      #("id", json.string(id)),
      #("title", json.string(title)),
      #("body", json.string("")),
      #("column", json.string("backlog")),
      #("feature", json.null()),
      #("milestone", milestone),
      #("dependsOn", json.array([], json.string)),
      #("createdTime", json.int(60)),
      #("closedTime", json.null()),
    ])
  }
  let model =
    workspace()
    |> step(strategy.Live(
      "{\"milestones\":[{\"id\":\"m1\",\"title\":\"Launch\"}]}",
    ))
    |> step(
      strategy.Live(
        message(".", [
          #(
            "tasks",
            json.array(
              [
                task("t1", "Loaded", json.string("m1")),
                task("t2", "Unloaded", json.string("m2")),
                task("t3", "None", json.null()),
                task("t4", "Blank", json.string("")),
              ],
              fn(t) { t },
            ),
          ),
        ]),
      ),
    )
  let page = element.to_string(strategy.view(model))
  let row = fn(title) {
    let assert Ok(found) =
      string.split(page, "<tr")
      |> list.find(fn(r) { string.contains(r, ">" <> title <> "</button>") })
    found
  }
  let milestone = fn(row) {
    let assert Ok(found) =
      string.split(row, "<td")
      |> list.find(fn(cell) {
        string.contains(cell, "/milestone/") || string.contains(cell, ">—</td>")
      })
    found
  }
  let loaded = row("Loaded")
  assert string.contains(loaded, "aria-label=\"Feature of Loaded\"")
  assert string.contains(
    milestone(loaded),
    "<a aria-label=\"Launch, milestone of Loaded\" href=\"/milestone/m1\">Launch</a>",
  )
  assert !string.contains(milestone(loaded), "<select")
  let unloaded = row("Unloaded")
  assert string.contains(
    milestone(unloaded),
    "<a aria-label=\"m2, milestone of Unloaded\" href=\"/milestone/m2\">m2</a>",
  )
  assert !string.contains(milestone(unloaded), "<select")
  let without = row("None")
  assert string.contains(milestone(without), ">—</td>")
  assert !string.contains(milestone(without), "<select")
  let blank = row("Blank")
  assert string.contains(milestone(blank), ">—</td>")
  assert !string.contains(milestone(blank), "<select")
}

/// Features and milestones share one class per state, with neutral fallback.
pub fn record_state_labels_share_one_color_class_test() {
  let states = ["proposed", "upcoming", "active", "complete", "canceled"]
  let colors = [
    #("proposed", "#6a5ba6"),
    #("upcoming", "#2f6db5"),
    #("active", "#1f7d35"),
    #("complete", "#000"),
    #("canceled", "#46575a"),
  ]
  let milestones =
    "{\"milestones\":[{\"id\":\"m-proposed\",\"title\":\"Proposed milestone\",\"rank\":1,\"state\":\"proposed\"},{\"id\":\"m-upcoming\",\"title\":\"Upcoming milestone\",\"rank\":2,\"state\":\"upcoming\"},{\"id\":\"m-active\",\"title\":\"Active milestone\",\"rank\":3,\"state\":\"active\"},{\"id\":\"m-complete\",\"title\":\"Complete milestone\",\"rank\":4,\"state\":\"complete\"},{\"id\":\"m-canceled\",\"title\":\"Canceled milestone\",\"rank\":5,\"state\":\"canceled\"},{\"id\":\"m-mystery\",\"title\":\"Mystery milestone\",\"rank\":6,\"state\":\"mystery\"}]}"
  let features =
    "{\"features\":[{\"id\":\"f-proposed\",\"title\":\"Proposed feature\",\"rank\":1,\"milestone\":\"m-proposed\",\"state\":\"proposed\"},{\"id\":\"f-upcoming\",\"title\":\"Upcoming feature\",\"rank\":2,\"milestone\":\"m-proposed\",\"state\":\"upcoming\"},{\"id\":\"f-active\",\"title\":\"Active feature\",\"rank\":3,\"milestone\":\"m-proposed\",\"state\":\"active\"},{\"id\":\"f-complete\",\"title\":\"Complete feature\",\"rank\":4,\"milestone\":\"m-proposed\",\"state\":\"complete\"},{\"id\":\"f-canceled\",\"title\":\"Canceled feature\",\"rank\":5,\"milestone\":\"m-proposed\",\"state\":\"canceled\"},{\"id\":\"f-mystery\",\"title\":\"Mystery feature\",\"rank\":6,\"milestone\":\"m-proposed\",\"state\":\"mystery\"}]}"
  let html =
    workspace()
    |> step(strategy.Live(milestones))
    |> step(strategy.Live(features))
    |> step(strategy.ToggleMilestone("m-proposed"))
    |> strategy.view
    |> element.to_string
  let count = fn(html, text) { list.length(string.split(html, text)) - 1 }
  list.each(states, fn(state) {
    let label =
      "<span class=\"state state-" <> state <> "\">" <> state <> "</span>"
    // A feature card, a milestone heading and a roadmap feature row.
    assert count(html, label) == 3
    assert string.contains(
      html,
      ".state.state-" <> state <> "{background:var(--state-" <> state <> ")",
    )
  })
  list.each(colors, fn(color) {
    assert string.contains(html, "--state-" <> color.0 <> ":" <> color.1 <> ";")
  })
  assert string.contains(html, "--state-ink:#fff")
  // The exact markup shows a state label carries no inline color.
  assert count(html, "<span class=\"state\">mystery</span>") == 3
  assert !string.contains(html, "state-mystery")
}

/// The panel draws the shared effort pick: the model's efforts, its default
/// shown until another is chosen, and the effort it sends.
pub fn the_plan_draws_the_shared_effort_pick_test() {
  let model =
    planned()
    |> step(strategy.ModelsLoaded(
      "[{\"id\":\"m\",\"label\":\"M\",\"effort\":\"medium\",\"efforts\":[\"low\",\"medium\",\"high\",\"xhigh\",\"max\"]},{\"id\":\"n\",\"label\":\"N\",\"effort\":\"high\",\"efforts\":[\"low\",\"high\"]}]",
    ))
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "class=\"effort-pick\"")
  assert string.contains(page, "class=\"filters agent-choice\"")
  assert string.contains(page, "role=\"group\"")
  assert string.contains(page, "aria-label=\"Effort\"")
  assert string.contains(
    page,
    ".effort-pick label{position:relative;display:block;flex:none;width:2.375rem;",
  )
  assert string.contains(page, ".effort-pick>.effort-name{")
  assert string.contains(page, ">Effort</span>")
  assert string.contains(page, ">Low</span>")
  assert string.contains(page, ">Med</span>")
  assert string.contains(page, ">High</span>")
  assert string.contains(page, ">X-Hi</span>")
  assert string.contains(page, ">Max</span>")
  assert !string.contains(page, ">Default</span>")
  assert string.contains(page, "title=\"xhigh\"")
  assert string.contains(
    page,
    ".effort-pick label:hover span{background:var(--hover)}",
  )
  assert string.contains(
    page,
    ".effort-pick input:checked+span{background:var(--ink);color:var(--paper);font-weight:600}",
  )
  assert string.contains(
    page,
    ".effort-pick label:hover input:checked+span{background:var(--hover);color:var(--ink)}",
  )
  assert string.contains(
    page,
    ".plan .agent-choice>select{flex:1 1 5rem;min-width:0;width:auto}",
  )
  assert string.contains(
    page,
    ".plan .agent-choice>.effort-pick{flex:none;margin:0}",
  )
  assert string.contains(
    page,
    ".plan .agent-choice>.unavailable{flex-basis:100%}",
  )
  // Nothing chosen: the model's default (medium) is chosen and sends "".
  assert string.contains(
    page,
    "checked name=\"plan-effort\" type=\"radio\" value>",
  )
  assert !string.contains(
    page,
    "checked name=\"plan-effort\" type=\"radio\" value=\"medium\"",
  )
  let chosen = step(model, strategy.PlanEffort("high"))
  assert chosen.plan_effort == "high"
  let page = element.to_string(strategy.view(chosen))
  assert string.contains(
    page,
    "checked name=\"plan-effort\" type=\"radio\" value=\"high\"",
  )
  let default = step(chosen, strategy.PlanEffort(""))
  assert default.plan_effort == ""
  let page = element.to_string(strategy.view(default))
  assert string.contains(
    page,
    "checked name=\"plan-effort\" type=\"radio\" value>",
  )
  let changed = step(chosen, strategy.PlanModel("n"))
  assert changed.plan_model == "n"
  assert changed.plan_effort == ""
  let page = element.to_string(strategy.view(changed))
  assert string.contains(page, ">High</span>")
  assert string.contains(
    page,
    "checked name=\"plan-effort\" type=\"radio\" value>",
  )
}

/// An answered question without a proposal shows its answer as Markdown, so a
/// `task:<id>` in it is a link, as a proposal's answer is.
pub fn a_plain_question_answer_links_its_task_reference_test() {
  let reply =
    json.to_string(
      run("r6", "waiting", #("page", ""), 70, [
        #(
          "questions",
          json.preprocessed_array([
            json.object([
              #("id", json.string("q1")),
              #("text", json.string("Which task?")),
              #("options", json.array(["Yes", "No"], json.string)),
              #("answer", json.string("task:0123456789abcdef")),
            ]),
          ]),
        ),
      ]),
    )
  let model =
    planned()
    |> step(strategy.PlanText("Plan search"))
    |> step(strategy.StartPlan)
    |> step(strategy.PlanReplied(True, reply))
  let page = element.to_string(strategy.view(model))
  assert string.contains(page, "href=\"/task/0123456789abcdef\"")
  assert string.contains(page, "data-task=\"0123456789abcdef\"")
}
