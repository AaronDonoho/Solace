import features
import gleam/json
import gleam/list
import gleam/option.{None, Some}
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
    == strategy.Row("t1", "A", "b", "done", Some("f1"), None, Some(70), ["t0"])
  // A task saved before D-87 has none of the fields.
  let assert Ok(old) =
    json.parse(
      "{\"id\":\"t2\",\"title\":\"Old\",\"body\":\"\",\"column\":\"backlog\"}",
      strategy.row_decoder(),
    )
  assert #(old.feature, old.milestone, old.closed, old.depends)
    == #(None, None, None, [])
}

pub fn closed_tasks_come_newest_first_then_open_ones_test() {
  let row = fn(id, title, closed) {
    #(".", strategy.Row(id, title, "", "done", None, None, closed, []))
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
      Some(5),
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
  // A task with an open run offers it instead of a new start.
  let shown = element.to_string(strategy.view(model))
  assert string.contains(shown, "A planning run on this is open.")
  assert !string.contains(shown, ">Start</button>")
}

/// A task row, a feature and a milestone with an open run show a marker; it
/// goes when the run ends.
pub fn open_runs_mark_their_records_until_they_end_test() {
  let model = planned()
  let page = fn(model) { element.to_string(strategy.view(model)) }
  let markers = fn(model) {
    list.length(string.split(page(model), ">Planning</span>")) - 1
  }
  assert plan.open_ids(model.runs, "task") == ["t1"]
  assert markers(model) == 3
  let marker =
    "<span class=\"planning\" title=\"A planning run is open on this\">"
  assert string.contains(page(model), "Index</button>" <> marker)
  assert string.contains(
    page(model),
    "<h4>Search <span class=\"state\">proposed</span>" <> marker,
  )
  assert string.contains(
    page(model),
    "<h3>Launch <span class=\"state\">proposed</span>" <> marker,
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
    "<h3>Launch <span class=\"state\">proposed</span>" <> marker,
  )
}

/// The shown run stays in the panel across tab switches; the panel lists
/// the runs on its context newest first, and Close returns to Start.
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
  assert string.contains(page, ">Start</button>")
  assert list.map(plan.past(model.runs, plan.page), fn(r) { r.id })
    == ["r4", "r0"]
  let assert [_, after] = string.split(page, "Runs on this")
  assert string.contains(after, "Ended: stopped")
  assert string.contains(after, "Ended: planned")
  // An ended run opens read-only.
  let ended = shown(step(model, strategy.ShowPlan("r4")))
  assert !string.contains(ended, "id=\"plan-text\"")
  // Collapsed, the panel keeps one line.
  let collapsed = shown(step(model, strategy.PlanCollapse))
  assert string.contains(collapsed, "No run open")
  assert !string.contains(collapsed, ">Start</button>")
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
                      #("state", json.string("planned")),
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
    "Approving also sets the feature <em>Search</em> back to planned.",
  )
  assert string.contains(page, ">Approve</button>")
  // D-115: an edit shows only the fields that change, a short one as old →
  // new; a field that still matches the record's value isn't shown.
  assert string.contains(page, "<dt>state</dt><dd>planned → active</dd>")
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
  assert string.contains(shown(model), "Planning Context: Whole Project")
  let model = step(model, strategy.Open(".", "t1"))
  assert string.contains(shown(model), "Planning Context: task <em>Index</em>")
  assert string.contains(
    shown(step(model, strategy.ShowPlan("r2"))),
    "Planning Context: feature <em>Search</em>",
  )
  assert string.contains(
    shown(step(model, strategy.ShowPlan("r3"))),
    "Planning Context: milestone <em>Launch</em>",
  )
  assert !string.contains(shown(model), "On the whole page")
}

/// D-98: a task link opens the task on the Tasks tab, from any tab, and asks
/// `<solace-task>` to show it; asking again for the same task is news too.
pub fn a_task_link_opens_its_task_on_the_tasks_tab_test() {
  let model = strategy.new(strategy.Roadmap)
  let #(model, _) = strategy.update(model, strategy.Focus("a", "abc"))
  assert model.tab == strategy.Tasks
  assert model.selected == Some(#("a", "abc"))
  assert model.reveal == #("abc", 1)
  let #(model, _) = strategy.update(model, strategy.Focus("a", "abc"))
  assert model.reveal == #("abc", 2)
  let #(same, _) = strategy.update(model, strategy.Focus("a", ""))
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
    )
  }
  let ask = fn(id, answer) {
    plan.Question(id, "Add a task?", ["Yes", "No"], answer, None, 0)
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
