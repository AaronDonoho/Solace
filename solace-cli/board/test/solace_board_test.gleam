import gleam/dict
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{None, Some}
import gleam/result
import gleam/string
import gleeunit
import gleeunit/should
import lustre/element
import solace_board as board
import solace_task

pub fn main() {
  gleeunit.main()
}

fn tasks(model: board.Model) -> List(solace_task.Task) {
  let assert [lane, ..] = model.lanes
  lane.tasks
}

pub fn workflow_and_live_task_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.WorkflowLoaded(
        "",
        "{\"columns\":[{\"id\":\"triage\",\"label\":\"Triage\",\"actions\":[\"assign\"]},{\"id\":\"done\",\"final\":true}]}",
      ),
    )
  let text =
    "{\"id\":\"1\",\"title\":\"Fix checkout\",\"body\":\"Details\",\"column\":\"triage\",\"rank\":1,\"paused_from\":null,\"stamps\":{},\"notes\":[],\"questions\":[],\"runs\":[]}"
  let #(model, _) = board.update(model, board.CaughtUp)
  let #(model, _) = board.update(model, board.Live("{\"task\":" <> text <> "}"))
  list.length(tasks(model)) |> should.equal(1)
  let rendered = board.view(model) |> element.to_string
  rendered |> string.contains("Fix checkout") |> should.be_true
  rendered |> string.contains("<h2>Triage</h2>") |> should.be_true
}

pub fn notices_and_stamp_test() {
  let assert Ok(old) =
    json.parse(
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1,\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"running\",\"model\":\"m\",\"log\":\"/l\"}]}",
      solace_task.task_decoder(),
    )
  let assert Ok(new) =
    json.parse(
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"needs_review\",\"rank\":1,\"stamps\":{\"approved\":{}},\"questions\":[{\"id\":\"q\",\"text\":\"Why?\",\"options\":[]}],\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"ended\",\"model\":\"m\",\"log\":\"/l\",\"outcome\":\"finished\"}]}",
      solace_task.task_decoder(),
    )
  solace_task.notices(old, new)
  |> should.equal([
    solace_task.Notice(
      "Finished T",
      "The agent finished its work.",
      "",
      "1",
      "runs",
    ),
    solace_task.Notice("Question on T", "Why?", "", "1", "questions"),
  ])
  let #(model, _) = board.init(Nil)
  let rendered =
    board.view(
      board.Model(..model, lanes: [
        board.Lane(
          "",
          "",
          "",
          "",
          "",
          solace_task.Workflow(
            [solace_task.Column("needs_review", "Needs review", [], False)],
            dict.new(),
          ),
          [new],
        ),
      ]),
    )
    |> element.to_string
  rendered |> string.contains("class=\"stamp\"") |> should.be_true
}

pub fn replay_shows_end_result_without_notices_test() {
  let event = fn(state, outcome) {
    "{\"task\":{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1,\"runs\":[{\"id\":\"r1\",\"kind\":\"review\",\"state\":\""
    <> state
    <> "\",\"model\":\"m\",\"log\":\"/l\",\"outcome\":\""
    <> outcome
    <> "\"}]}}"
  }
  let #(model, _) = board.init(Nil)
  let #(model, _) = board.update(model, board.Live(event("running", "")))
  let #(model, _) = board.update(model, board.Live(event("ended", "reviewed")))
  // Nothing shows until the replay is over, and then only its end result.
  tasks(model) |> should.equal([])
  let #(model, _) = board.update(model, board.CaughtUp)
  let assert [task] = tasks(model)
  let assert [run] = task.runs
  run.state |> should.equal("ended")
  model.notices |> should.equal([])
  // After it, a change is news.
  let #(model, _) = board.update(model, board.Live(event("running", "")))
  let #(model, _) = board.update(model, board.Live(event("ended", "reviewed")))
  let assert [notice] = model.notices
  notice.heading |> should.equal("Reviewed T")
  notice.id |> should.equal("1")
  notice.tab |> should.equal("runs")
}

pub fn workspace_lanes_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\"bench/pilot\",\"name\":\"pilot\",\"base\":\"/p/bench%2Fpilot\"},{\"dir\":\"web\",\"name\":\"web\",\"base\":\"/p/web\"}],\"found\":[{\"dir\":\"tools\",\"name\":\"tools\",\"imported\":false,\"member\":false}]}",
      ),
    )
  let columns = "{\"columns\":[{\"id\":\"planning\",\"label\":\"Planning\"}]}"
  let #(model, _) =
    board.update(model, board.WorkflowLoaded("bench/pilot", columns))
  let #(model, _) = board.update(model, board.WorkflowLoaded("web", columns))
  board.shared_columns(model.lanes) |> should.be_true
  let task = fn(title) {
    "{\"id\":\"1\",\"title\":\""
    <> title
    <> "\",\"body\":\"\",\"column\":\"planning\",\"rank\":1,\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"running\",\"model\":\"m\",\"log\":\"/l\"}]}"
  }
  let #(model, _) = board.update(model, board.CaughtUp)
  let #(model, _) =
    board.update(
      model,
      board.Live("{\"project\":\"web\",\"task\":" <> task("In web") <> "}"),
    )
  let assert [pilot, web] = model.lanes
  pilot.tasks |> should.equal([])
  list.length(web.tasks) |> should.equal(1)
  let rendered =
    board.view(model)
    |> element.to_string
    |> string.split("</style>")
    |> list.last
    |> result.unwrap("")
  // One header row for both lanes; each lane names its folder and links its view.
  rendered |> string.contains("lane-head") |> should.be_true
  // The header row is a bar that sticks to the top of the lanes area, the
  // board page's scroll area: the lanes container is not a scroll container
  // itself, and the bar is opaque so cards can't show through.
  let css =
    board.view(model)
    |> element.to_string
    |> string.split("</style>")
    |> list.first
    |> result.unwrap("")
  css
  |> string.contains(
    ".lane-head{position:sticky;top:0;z-index:5;background:var(--paper);border-bottom:2px solid var(--edge)}",
  )
  |> should.be_true
  css
  |> string.contains(".lanes{display:block;overflow:visible}")
  |> should.be_true
  css
  |> string.contains(
    ".lanes-area{flex:1;min-height:0;overflow-y:auto;overscroll-behavior:contain}",
  )
  |> should.be_true
  rendered |> string.contains("<h2>Planning</h2>") |> should.be_true
  rendered
  |> string.contains("href=\"/p/bench%2Fpilot/contracts\"")
  |> should.be_true
  rendered |> string.contains(">Projects</button>") |> should.be_true
  // A run ending in a workspace names its project.
  let ended =
    string.replace(
      task("In web"),
      "\"running\"",
      "\"ended\",\"outcome\":\"finished\"",
    )
  let #(model, _) =
    board.update(
      model,
      board.Live("{\"project\":\"web\",\"task\":" <> ended <> "}"),
    )
  model.notices
  |> should.equal([
    solace_task.Notice(
      "web: Finished In web",
      "The agent finished its work.",
      "web",
      "1",
      "runs",
    ),
  ])
  // Different workflows: each lane names its own columns.
  let #(model, _) =
    board.update(
      model,
      board.WorkflowLoaded("web", "{\"columns\":[{\"id\":\"todo\"}]}"),
    )
  board.shared_columns(model.lanes) |> should.be_false
  let rendered =
    board.view(model)
    |> element.to_string
    |> string.split("</style>")
    |> list.last
    |> result.unwrap("")
  // Lanes with different workflows each get a bar of their own, which sticks
  // inside that lane.
  rendered |> string.split("lane-head") |> list.length |> should.equal(3)
  rendered |> string.contains("<h2>todo</h2>") |> should.be_true
  // A collapsed lane hides its columns.
  let #(model, _) = board.update(model, board.Collapse("web"))
  board.view(model)
  |> element.to_string
  |> string.contains("<h2>todo</h2>")
  |> should.be_false
  // Collapsing a lane takes its bar away with its columns.
  board.view(model)
  |> element.to_string
  |> string.split("</style>")
  |> list.last
  |> result.unwrap("")
  |> string.split("lane-head")
  |> list.length
  |> should.equal(2)
  // Its tasks go to the project's own API.
  let #(model, _) = board.update(model, board.Select("bench/pilot", "1"))
  board.view(model)
  |> element.to_string
  |> string.contains("<solace-task base=\"/p/bench%2Fpilot\"")
  |> should.be_true
}

pub fn root_project_lane_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\".\",\"label\":\"visualizer\",\"name\":\"bench_visualizer\",\"base\":\"\"},{\"dir\":\"frontend\",\"label\":\"frontend\",\"name\":\"bench_visualizer_frontend\",\"base\":\"/p/frontend\"}],\"found\":[]}",
      ),
    )
  let #(model, _) = board.update(model, board.CaughtUp)
  // The root's project answers at `/`; an event without `project` is its.
  let #(model, _) =
    board.update(
      model,
      board.Live(
        "{\"task\":{\"id\":\"1\",\"title\":\"Root task\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1}}",
      ),
    )
  let assert [root, frontend] = model.lanes
  list.length(root.tasks) |> should.equal(1)
  frontend.tasks |> should.equal([])
  let #(model, _) = board.update(model, board.ToggleProjects)
  let rendered = board.view(model) |> element.to_string
  rendered |> string.contains("<strong>visualizer</strong>") |> should.be_true
  rendered |> string.contains("(this folder)") |> should.be_true
  rendered |> string.contains("href=\"/contracts\"") |> should.be_true
  rendered
  |> string.contains("href=\"/p/frontend/contracts\"")
  |> should.be_true
}

pub fn lane_target_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\".\",\"label\":\"app\",\"name\":\"app\",\"base\":\"\",\"target\":\"javascript\"},{\"dir\":\"lib\",\"label\":\"lib\",\"name\":\"lib\",\"base\":\"/p/lib\"}],\"found\":[]}",
      ),
    )
  let assert [app, lib] = model.lanes
  app.target |> should.equal("javascript")
  lib.target |> should.equal("")
  let rendered = board.view(model) |> element.to_string
  rendered
  |> string.contains(
    "<strong>app</strong><small class=\"lane-target muted\"> javascript</small>",
  )
  |> should.be_true
  rendered |> string.contains("lane-target muted\"> \n") |> should.be_false
  string.split(rendered, "lane-target") |> list.length |> should.equal(2)
}

pub fn single_project_header_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":false,\"projects\":[{\"dir\":\".\",\"label\":\"shop\",\"name\":\"app\",\"base\":\"\",\"target\":\"erlang\"}],\"found\":[]}",
      ),
    )
  let rendered =
    board.view(model)
    |> element.to_string
    |> string.split("</style>")
    |> list.last
    |> result.unwrap("")
  rendered
  |> string.contains(
    "<header class=\"lane-title\"><span><strong>shop</strong><small> app</small><small class=\"lane-target muted\"> erlang</small></span><small>0 tasks</small><a href=\"/contracts\">Contract view</a></header>",
  )
  |> should.be_true
  rendered |> string.contains("lane-toggle") |> should.be_false
}

/// D-61: a task covering a and b shows in both lanes, names its projects and
/// each change's project, and leaves a lane it moved out of.
pub fn task_across_projects_test() {
  let #(model, _) = board.init(Nil)
  let lane = fn(d) {
    "{\"dir\":\""
    <> d
    <> "\",\"label\":\""
    <> d
    <> "\",\"name\":\""
    <> d
    <> "\",\"base\":\"/p/"
    <> d
    <> "\"}"
  }
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":["
        <> string.join(list.map(["a", "b", "c"], lane), ",")
        <> "],\"found\":[]}",
      ),
    )
  let columns = "{\"columns\":[{\"id\":\"planning\",\"label\":\"Planning\"}]}"
  let model =
    list.fold(["a", "b", "c"], model, fn(m, d) {
      board.update(m, board.WorkflowLoaded(d, columns)).0
    })
  let #(model, _) = board.update(model, board.CaughtUp)
  let task =
    "{\"id\":\"t1\",\"title\":\"Across\",\"body\":\"\",\"column\":\"planning\",\"rank\":1,\"scope\":[\"a\",\"b\"],\"changes\":[{\"entity\":\"symbol:x\",\"name\":\"app.one\",\"project\":\"b\"}]}"
  let #(model, _) =
    board.update(
      model,
      board.Live("{\"project\":\"a\",\"task\":" <> task <> "}"),
    )
  let rendered = board.view(model) |> element.to_string
  string.split(rendered, "Across<small class=\"muted\">a · b</small>")
  |> list.length
  |> should.equal(3)
  string.split(rendered, "<small>1 task</small>")
  |> list.length
  |> should.equal(3)
  rendered |> string.contains("<small>0 tasks</small>") |> should.be_true
  let assert Ok(decoded) = json.parse(task, solace_task.task_decoder())
  decoded.changes |> should.equal([solace_task.Change("b", "app", "one")])
  // The task moved out of a: a's lane drops it.
  let #(model, _) =
    board.update(model, board.Live("{\"project\":\"a\",\"removed\":\"t1\"}"))
  board.view(model)
  |> element.to_string
  |> string.contains("Across")
  |> should.be_false
}

/// Outside a browser registering reports the error instead of crashing.
pub fn register_outside_browser_test() {
  board.register() |> should.be_error
}

pub fn selected_card_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\"a\",\"label\":\"a\",\"name\":\"a\",\"base\":\"/p/a\"},{\"dir\":\"b\",\"label\":\"b\",\"name\":\"b\",\"base\":\"/p/b\"}],\"found\":[]}",
      ),
    )
  let columns = "{\"columns\":[{\"id\":\"planning\",\"label\":\"Planning\"}]}"
  let #(model, _) = board.update(model, board.WorkflowLoaded("a", columns))
  let #(model, _) = board.update(model, board.WorkflowLoaded("b", columns))
  let shared =
    "{\"id\":\"1\",\"title\":\"Across\",\"body\":\"\",\"column\":\"planning\",\"rank\":1,\"scope\":[\"a\",\"b\"]}"
  let other =
    "{\"id\":\"1\",\"title\":\"Other\",\"body\":\"\",\"column\":\"planning\",\"rank\":1}"
  let #(model, _) = board.update(model, board.CaughtUp)
  let #(model, _) =
    board.update(
      model,
      board.Live("{\"project\":\"a\",\"task\":" <> shared <> "}"),
    )
  let #(model, _) =
    board.update(
      model,
      board.Live("{\"project\":\"b\",\"task\":" <> other <> "}"),
    )
  let selected = fn(m) {
    board.view(m)
    |> element.to_string
    |> string.split("class=\"card selected\"")
    |> list.length
  }
  selected(model) |> should.equal(1)
  // A task across a and b shows selected in both lanes.
  let #(model, _) = board.update(model, board.Select("a", "1"))
  selected(model) |> should.equal(3)
  let rendered = board.view(model) |> element.to_string
  string.split(rendered, "aria-current=\"true\"")
  |> list.length
  |> should.equal(3)
  rendered
  |> string.contains(
    "button.card[aria-current=true],button.card[aria-current=true]:hover",
  )
  |> should.be_true
  // Another task with the same id, stored in b: only its card.
  let #(model, _) = board.update(model, board.Select("b", "1"))
  selected(model) |> should.equal(2)
  let #(model, _) = board.update(model, board.Select("a", "1"))
  // The open task moves from a to b: the highlight and details follow it.
  let moved =
    "{\"id\":\"1\",\"title\":\"Moved\",\"body\":\"\",\"column\":\"planning\",\"rank\":1,\"scope\":[\"b\",\"a\"]}"
  let #(model, _) =
    board.update(model, board.Live("{\"project\":\"b\",\"removed\":\"1\"}"))
  let #(model, _) =
    board.update(
      model,
      board.Live("{\"project\":\"b\",\"task\":" <> moved <> "}"),
    )
  let #(model, _) =
    board.update(model, board.Live("{\"project\":\"a\",\"removed\":\"1\"}"))
  model.selected_card |> should.equal(Some(#("b", "1")))
  selected(model) |> should.equal(3)
  board.view(model)
  |> element.to_string
  |> string.contains("<solace-task base=\"/p/b\" dir=\"b\" hosted task=\"1\"")
  |> should.be_true
}

pub fn removing_a_project_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\"a\",\"label\":\"a\",\"name\":\"a\",\"base\":\"/p/a\"},{\"dir\":\"b\",\"label\":\"b\",\"name\":\"b\",\"base\":\"/p/b\"}],\"found\":[]}",
      ),
    )
  let #(model, _) = board.update(model, board.ToggleProjects)
  let shows = fn(model, text) {
    board.view(model) |> element.to_string |> string.contains(text)
  }
  shows(model, "Remove and delete files") |> should.be_false
  // Step one asks whether to keep the files.
  let #(model, _) = board.update(model, board.RemoveProject("a"))
  model.removing |> should.equal(Some(#("a", False, "")))
  shows(model, "Remove only") |> should.be_true
  shows(model, "Remove and delete files") |> should.be_true
  shows(model, "Delete permanently") |> should.be_false
  // Step two warns, and Delete stays disabled until the name matches.
  let #(model, _) = board.update(model, board.RemoveFiles)
  model.removing |> should.equal(Some(#("a", True, "")))
  shows(model, "It cannot be undone") |> should.be_true
  shows(model, " disabled>Delete permanently") |> should.be_true
  let #(model, _) = board.update(model, board.RemoveTyped("b"))
  shows(model, " disabled>Delete permanently") |> should.be_true
  let #(model, _) = board.update(model, board.RemoveTyped("a"))
  shows(model, "Delete permanently") |> should.be_true
  shows(model, " disabled>Delete permanently") |> should.be_false
  // Cancel closes the dialog and sends nothing.
  let #(model, _) = board.update(model, board.RemoveCancel)
  model.removing |> should.equal(None)
  // A reloaded list closes it too.
  let #(model, _) = board.update(model, board.RemoveProject("b"))
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\"a\",\"label\":\"a\",\"name\":\"a\",\"base\":\"/p/a\"}],\"found\":[]}",
      ),
    )
  model.removing |> should.equal(None)
}

/// D-78: a new stream starts from each project's tasks; a snapshot that comes
/// before the project list names its lane is held until it does.
pub fn snapshot_waits_for_its_lane_and_replaces_tasks_test() {
  let task = fn(id, title) {
    "{\"id\":\""
    <> id
    <> "\",\"title\":\""
    <> title
    <> "\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1,\"runs\":[]}"
  }
  let #(model, _) = board.init(Nil)
  let #(model, _) = board.update(model, board.CaughtUp)
  let #(model, _) =
    board.update(
      model,
      board.Live(
        "{\"project\":\"web\",\"tasks\":[" <> task("1", "In web") <> "]}",
      ),
    )
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\"web\",\"name\":\"web\",\"base\":\"/p/web\"}],\"found\":[]}",
      ),
    )
  let assert [web] = model.lanes
  list.map(web.tasks, fn(t) { t.title }) |> should.equal(["In web"])
  model.notices |> should.equal([])
  // A later snapshot replaces the lane's tasks.
  let #(model, _) =
    board.update(
      model,
      board.Live(
        "{\"project\":\"web\",\"tasks\":[" <> task("2", "Another") <> "]}",
      ),
    )
  let assert [web] = model.lanes
  list.map(web.tasks, fn(t) { t.title }) |> should.equal(["Another"])
}

pub fn typed_fields_and_toggles_update_the_model_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) = board.update(model, board.ProjectName("web"))
  let #(model, _) = board.update(model, board.ProjectDir("apps/web"))
  let #(model, _) = board.update(model, board.ProjectTarget("javascript"))
  #(model.project_name, model.project_dir, model.project_target)
  |> should.equal(#("web", "apps/web", "javascript"))
  let open = model.projects_open
  let #(model, _) = board.update(model, board.ToggleProjects)
  model.projects_open |> should.equal(!open)
}

pub fn removing_needs_the_second_step_and_the_exact_name_test() {
  let #(model, _) = board.init(Nil)
  // Nothing is being removed: every removal message is a no-op.
  let #(same, _) = board.update(model, board.RemoveOnly)
  same |> should.equal(model)
  let #(same, _) = board.update(model, board.RemoveFiles)
  same |> should.equal(model)
  let #(same, _) = board.update(model, board.RemoveTyped("a"))
  same |> should.equal(model)
  let #(same, _) = board.update(model, board.RemoveDelete)
  same |> should.equal(model)
  // Typing before the second step is ignored, and Delete before it sends nothing.
  let #(first, _) = board.update(model, board.RemoveProject("a"))
  let #(same, _) = board.update(first, board.RemoveTyped("a"))
  same |> should.equal(first)
  let #(same, _) = board.update(first, board.RemoveDelete)
  same |> should.equal(first)
  // A wrong name keeps the dialog open.
  let #(second, _) = board.update(first, board.RemoveFiles)
  let #(wrong, _) = board.update(second, board.RemoveTyped("b"))
  let #(same, _) = board.update(wrong, board.RemoveDelete)
  same |> should.equal(wrong)
  // Adding a directory that was not found does nothing.
  let #(same, _) = board.update(model, board.AddProject("zzz"))
  same |> should.equal(model)
}

/// A notice's button opens its task and asks `<solace-task>` to show the
/// notice's tab; asking again for the same is news too.
pub fn notice_opens_task_on_its_tab_test() {
  let question =
    solace_task.Notice("Question on T", "Why?", "", "1", "questions")
  let without =
    solace_task.Notice(
      "Finished T",
      "The agent finished its work.",
      "",
      "1",
      "runs",
    )
  let #(model, _) = board.init(Nil)
  let model = board.Model(..model, notices: [question, without])
  let #(model, _) = board.update(model, board.Open(question))
  model.selected_card |> should.equal(Some(#("", "1")))
  model.reveal |> should.equal(#("questions", "1", 1))
  model.notices |> should.equal([without])
  let #(model, _) = board.update(model, board.Open(without))
  model.reveal |> should.equal(#("runs", "1", 2))
  model.notices |> should.equal([])
}

/// The open task's details are `<solace-task>`: it names the task, its
/// project and its API prefix, and takes its events from the board.
pub fn detail_is_a_solace_task_element_test() {
  let #(model, _) = board.init(Nil)
  board.view(model)
  |> element.to_string
  |> string.contains("Select a task to see its details.")
  |> should.be_true
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\".\",\"label\":\"root\",\"name\":\"root\",\"base\":\"\"},{\"dir\":\"a\",\"label\":\"a\",\"name\":\"a\",\"base\":\"/p/a\"}],\"found\":[]}",
      ),
    )
  let #(model, _) = board.update(model, board.Select("a", "t1"))
  let rendered = board.view(model) |> element.to_string
  rendered
  |> string.contains(
    "<solace-task base=\"/p/a\" dir=\"a\" hosted task=\"t1\" workspace>",
  )
  |> should.be_true
  rendered |> string.contains("class=\"detail\"") |> should.be_false
  // The root's project answers at the board's own prefix.
  let #(model, _) = board.update(model, board.BaseChanged("/x"))
  let #(model, _) = board.update(model, board.Select(".", "t2"))
  board.view(model)
  |> element.to_string
  |> string.contains("<solace-task base=\"/x\" dir=\".\" hosted task=\"t2\"")
  |> should.be_true
}

/// The open task's own events, under any project, and its project's workflow
/// go to its details, numbered and in order; other tasks' and other projects'
/// workflows don't; a change to the list of projects does. Opening another
/// card starts the list again.
pub fn live_events_for_the_open_task_go_to_its_detail_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\"a\",\"label\":\"a\",\"name\":\"a\",\"base\":\"/p/a\"},{\"dir\":\"b\",\"label\":\"b\",\"name\":\"b\",\"base\":\"/p/b\"}],\"found\":[]}",
      ),
    )
  let #(model, _) = board.update(model, board.CaughtUp)
  let #(model, _) = board.update(model, board.Select("a", "1"))
  let task = fn(id) {
    "{\"id\":\""
    <> id
    <> "\",\"title\":\"T\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1}"
  }
  let own = "{\"project\":\"a\",\"task\":" <> task("1") <> "}"
  let #(model, _) = board.update(model, board.Live(own))
  model.live |> should.equal([#(1, own)])
  let #(model, _) =
    board.update(
      model,
      board.Live("{\"project\":\"a\",\"task\":" <> task("2") <> "}"),
    )
  let #(model, _) =
    board.update(
      model,
      board.Live("{\"project\":\"b\",\"task\":" <> task("1") <> "}"),
    )
  let elsewhere = "{\"project\":\"b\",\"task\":" <> task("1") <> "}"
  model.live |> should.equal([#(1, own), #(2, elsewhere)])
  // A workflow change after a task update is kept too, not replaced by it.
  let flow = "{\"project\":\"a\",\"workflow\":{\"columns\":[]}}"
  let #(model, _) =
    board.update(
      model,
      board.Live("{\"project\":\"b\",\"workflow\":{\"columns\":[]}}"),
    )
  let #(model, _) = board.update(model, board.Live(flow))
  model.live |> should.equal([#(1, own), #(2, elsewhere), #(3, flow)])
  // Another card starts the list again; numbers keep counting.
  let #(model, _) = board.update(model, board.Select("a", "2"))
  model.live |> should.equal([])
  let #(model, _) = board.update(model, board.Live(flow))
  model.live |> should.equal([#(4, flow)])
  // The Scope tab lists the projects, so a change to the list goes too.
  let projects = "{\"projects\":[\"a\",\"b\",\"c\"]}"
  let #(model, _) = board.update(model, board.Live(projects))
  model.live |> should.equal([#(4, flow), #(5, projects)])
}

/// `<solace-task>` says the open task moved: the board opens it from there.
pub fn task_moved_follows_the_open_task_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) = board.update(model, board.Select("a", "1"))
  let #(model, _) = board.update(model, board.TaskMoved("b", "2"))
  model.selected_card |> should.equal(Some(#("a", "1")))
  let #(model, _) = board.update(model, board.TaskMoved("b", "1"))
  model.selected_card |> should.equal(Some(#("b", "1")))
}

/// What a new connection replays is applied to the cards when it is caught
/// up, and the open task's details get it too, then `resync` to fetch the
/// task again in case an event was lost.
pub fn a_replay_after_a_reconnect_reaches_the_open_detail_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\"a\",\"label\":\"a\",\"name\":\"a\",\"base\":\"/p/a\"}],\"found\":[]}",
      ),
    )
  let #(model, _) = board.update(model, board.Select("a", "1"))
  let event = fn(title) {
    "{\"project\":\"a\",\"task\":{\"id\":\"1\",\"title\":\""
    <> title
    <> "\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1}}"
  }
  // Held while the stream replays; nothing is handed down yet.
  let #(model, _) = board.update(model, board.Live(event("Old")))
  let #(model, _) = board.update(model, board.Live(event("New")))
  model.live |> should.equal([])
  let #(model, _) = board.update(model, board.CaughtUp)
  model.live
  |> should.equal([
    #(1, event("Old")),
    #(2, event("New")),
    #(3, solace_task.resync),
  ])
}

/// A toast has its heading and buttons, never the text of the question or
/// note that raised it.
pub fn notice_shows_only_its_heading_test() {
  let question =
    solace_task.Notice("Question on T", "Why this?", "", "1", "questions")
  let reply =
    solace_task.Notice("Reply on T", "Here is my answer", "", "1", "notes")
  let stopped =
    solace_task.Notice(
      "Stopped T: interrupted",
      "The work run ended: interrupted.",
      "",
      "1",
      "runs",
    )
  let #(model, _) = board.init(Nil)
  let rendered =
    board.view(board.Model(..model, notices: [question, reply, stopped]))
    |> element.to_string
  rendered |> string.contains("Question on T") |> should.be_true
  rendered |> string.contains("Reply on T") |> should.be_true
  rendered |> string.contains("Stopped T: interrupted") |> should.be_true
  rendered |> string.contains("Show task") |> should.be_true
  rendered |> string.contains("Why this?") |> should.be_false
  rendered |> string.contains("Here is my answer") |> should.be_false
  rendered |> string.contains("run ended") |> should.be_false
}

/// Close All shows only when more than 5 toasts are showing, and dismisses
/// every one.
pub fn close_all_shows_above_five_toasts_test() {
  let toast = fn(n) {
    solace_task.Notice(
      "Question on T" <> int.to_string(n),
      "",
      "",
      int.to_string(n),
      "questions",
    )
  }
  let shows = fn(model) {
    board.view(model) |> element.to_string |> string.contains("Close All")
  }
  let #(model, _) = board.init(Nil)
  shows(board.Model(..model, notices: list.map([1, 2, 3, 4, 5], toast)))
  |> should.be_false
  let six = board.Model(..model, notices: list.map([1, 2, 3, 4, 5, 6], toast))
  shows(six) |> should.be_true
  let #(cleared, _) = board.update(six, board.DismissAll)
  cleared.notices |> should.equal([])
  shows(cleared) |> should.be_false
}

/// D-95: a card in a final column has an Archive button; an open one has none,
/// and an archived task leaves the board and its count.
pub fn archive_button_and_archived_cards_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.WorkflowLoaded(
        "",
        "{\"columns\":[{\"id\":\"triage\",\"label\":\"Triage\"},{\"id\":\"complete\",\"label\":\"Complete\",\"final\":true}]}",
      ),
    )
  let task = fn(id, title, column, archived) {
    "{\"task\":{\"id\":\""
    <> id
    <> "\",\"title\":\""
    <> title
    <> "\",\"body\":\"\",\"column\":\""
    <> column
    <> "\",\"rank\":1,\"archivedTime\":"
    <> archived
    <> "}}"
  }
  let #(model, _) = board.update(model, board.CaughtUp)
  let #(model, _) =
    board.update(model, board.Live(task("1", "Open", "triage", "null")))
  let #(model, _) =
    board.update(model, board.Live(task("2", "Done", "complete", "null")))
  let rendered = board.view(model) |> element.to_string
  rendered |> string.contains("aria-label=\"Archive Done\"") |> should.be_true
  rendered |> string.contains("aria-label=\"Archive Open\"") |> should.be_false
  rendered |> string.contains("2 tasks") |> should.be_true
  let #(model, _) = board.update(model, board.Select("", "2"))
  let #(model, _) =
    board.update(model, board.Live(task("2", "Done", "complete", "1700")))
  let rendered = board.view(model) |> element.to_string
  rendered |> string.contains(">Done") |> should.be_false
  rendered |> string.contains("1 task") |> should.be_true
  // The server's reply closes the archived task's details.
  let #(model, _) =
    board.update(
      model,
      board.Archived(
        "{\"id\":\"2\",\"title\":\"Done\",\"body\":\"\",\"column\":\"complete\",\"rank\":1,\"archivedTime\":1700}",
      ),
    )
  model.selected_card |> should.equal(None)
}

/// The case reported: a task stored in `a` and covering the root's project
/// too, opened from either lane, its detail handed a running investigation
/// and then the task completed. Both reach it in order, and its card shows no
/// agent at work.
pub fn a_completed_task_across_two_lanes_reaches_its_detail_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\".\",\"label\":\"root\",\"name\":\"r\",\"base\":\"\"},{\"dir\":\"a\",\"label\":\"a\",\"name\":\"a\",\"base\":\"/p/a\"}],\"found\":[]}",
      ),
    )
  let #(model, _) = board.update(model, board.CaughtUp)
  let flow =
    "{\"project\":\"a\",\"workflow\":{\"columns\":[{\"id\":\"assigned\",\"actions\":[\"complete\"]},{\"id\":\"complete\",\"actions\":[],\"final\":true}]}}"
  let #(model, _) = board.update(model, board.Live(flow))
  let one = fn(column, state) {
    "{\"project\":\"a\",\"task\":{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\""
    <> column
    <> "\",\"rank\":1,\"scope\":[\"a\",\".\"],\"runs\":[{\"id\":\"r\",\"kind\":\"investigate\",\"model\":\"m\",\"state\":\""
    <> state
    <> "\",\"started\":1,\"log\":\"\"}]}}"
  }
  let running = one("assigned", "running")
  let done = one("complete", "ended")
  let #(model, _) = board.update(model, board.Live(running))
  // Either lane's card opens the task from its home, `a`.
  let html = element.to_string(board.view(model))
  string.contains(html, "Agent investigating") |> should.be_true
  let #(model, _) = board.update(model, board.Select("a", "1"))
  let #(model, _) = board.update(model, board.Live(running))
  let #(model, _) = board.update(model, board.Live(done))
  model.live |> should.equal([#(1, running), #(2, done)])
  string.contains(element.to_string(board.view(model)), "Agent investigating")
  |> should.be_false
}

/// D-98: a task link opens the task and asks `<solace-task>` to show it, on
/// its details; asking again for the same task is news too.
pub fn a_task_link_opens_its_task_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) = board.update(model, board.Focus(".", "abc"))
  model.selected_card |> should.equal(Some(#(".", "abc")))
  model.reveal |> should.equal(#("details", "abc", 1))
  let #(model, _) = board.update(model, board.Focus(".", "abc"))
  model.reveal |> should.equal(#("details", "abc", 2))
  let #(same, _) = board.update(model, board.Focus(".", ""))
  same |> should.equal(model)
}

/// A workspace with lanes a and b, whose workflow has two final columns.
fn deps_board() -> board.Model {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\"a\",\"label\":\"a\",\"name\":\"a\",\"base\":\"/p/a\"},{\"dir\":\"b\",\"label\":\"b\",\"name\":\"b\",\"base\":\"/p/b\"}],\"found\":[]}",
      ),
    )
  let columns =
    "{\"columns\":[{\"id\":\"backlog\",\"label\":\"Backlog\"},{\"id\":\"planning\",\"label\":\"Planning\"},{\"id\":\"complete\",\"label\":\"Complete\",\"final\":true},{\"id\":\"cancelled\",\"label\":\"Cancelled\",\"final\":true}]}"
  let #(model, _) = board.update(model, board.WorkflowLoaded("a", columns))
  let #(model, _) = board.update(model, board.WorkflowLoaded("b", columns))
  board.update(model, board.CaughtUp).0
}

/// A live update of a task stored in `dir`; `extra` adds JSON fields.
fn put_task(
  model: board.Model,
  dir: String,
  id: String,
  title: String,
  column: String,
  extra: String,
) -> board.Model {
  board.update(
    model,
    board.Live(
      "{\"project\":\""
      <> dir
      <> "\",\"task\":{\"id\":\""
      <> id
      <> "\",\"title\":\""
      <> title
      <> "\",\"body\":\"\",\"column\":\""
      <> column
      <> "\",\"rank\":1"
      <> extra
      <> "}}",
    ),
  ).0
}

/// The rendered card button whose text has `title`.
fn card_of(model: board.Model, title: String) -> String {
  let assert Ok(card) =
    board.view(model)
    |> element.to_string
    |> string.split("<button")
    |> list.find(fn(s) { string.contains(s, ">" <> title) })
  card
}

/// Whether the card is dimmed and names what it waits on.
fn dimmed(model: board.Model, title: String, waits: String) -> Bool {
  let card = card_of(model, title)
  string.contains(card, "class=\"card waiting\"")
  && string.contains(card, "title=\"Waiting on " <> waits <> "\"")
  && string.contains(card, ">Waiting on " <> waits <> "</small>")
}

/// A task with no `dependsOn` (saved before D-87) is shown as before.
pub fn card_without_dependencies_is_not_dimmed_test() {
  let model = deps_board() |> put_task("a", "1", "Card", "planning", "")
  let card = card_of(model, "Card")
  string.contains(card, "waiting") |> should.be_false
  string.contains(card, "Waiting on") |> should.be_false
}

/// A dependency outside a final column dims the card, which still opens; it
/// brightens live once the dependency completes, and after its own
/// `dependsOn` is cleared.
pub fn unfinished_dependency_dims_the_card_test() {
  let model =
    deps_board()
    |> put_task("a", "d1", "Dep", "planning", "")
    |> put_task("a", "d2", "Other dep", "complete", "")
    |> put_task("a", "1", "Card", "planning", ",\"dependsOn\":[\"d1\",\"d2\"]")
  dimmed(model, "Card", "Dep") |> should.be_true
  // Dimming is visual only: the card opens.
  let #(open, _) = board.update(model, board.Select("a", "1"))
  card_of(open, "Card")
  |> string.contains("aria-current=\"true\" class=\"card selected waiting\"")
  |> should.be_true
  // Its dependency completes: the card brightens.
  let done = put_task(model, "a", "d1", "Dep", "complete", "")
  string.contains(card_of(done, "Card"), "waiting") |> should.be_false
  // Strategy clears its dependencies.
  let cleared =
    put_task(model, "a", "1", "Card", "planning", ",\"dependsOn\":[]")
  string.contains(card_of(cleared, "Card"), "waiting") |> should.be_false
}

/// Complete and cancelled both count as finished.
pub fn finished_or_cancelled_dependency_does_not_dim_test() {
  let model =
    deps_board()
    |> put_task("a", "1", "Card", "planning", ",\"dependsOn\":[\"d1\"]")
  let complete = put_task(model, "a", "d1", "Dep", "complete", "")
  string.contains(card_of(complete, "Card"), "waiting") |> should.be_false
  let cancelled = put_task(model, "a", "d1", "Dep", "cancelled", "")
  string.contains(card_of(cancelled, "Card"), "waiting") |> should.be_false
}

/// A dependency back out of a final column is unfinished again, though its
/// `closedTime` stays set.
pub fn reopened_dependency_dims_again_test() {
  let model =
    deps_board()
    |> put_task("a", "d1", "Dep", "complete", ",\"closedTime\":1700")
    |> put_task("a", "1", "Card", "planning", ",\"dependsOn\":[\"d1\"]")
  string.contains(card_of(model, "Card"), "waiting") |> should.be_false
  let reopened =
    put_task(model, "a", "d1", "Dep", "planning", ",\"closedTime\":1700")
  dimmed(reopened, "Card", "Dep") |> should.be_true
}

/// D-87: a dependency stored in another project is found in its lane.
pub fn dependency_in_another_project_dims_the_card_test() {
  let model =
    deps_board()
    |> put_task("b", "d1", "Dep", "planning", "")
    |> put_task("a", "1", "Card", "planning", ",\"dependsOn\":[\"d1\"]")
  dimmed(model, "Card", "Dep") |> should.be_true
  let done = put_task(model, "b", "d1", "Dep", "complete", "")
  string.contains(card_of(done, "Card"), "waiting") |> should.be_false
}

/// A live update that adds a notice asks for attention; one that adds none,
/// such as a replay, doesn't.
pub fn a_new_notice_raises_attention_test() {
  let n = solace_task.Notice("Reply on Fix", "Done.", ".", "t1", "notes")
  assert board.raised([], [n])
  assert !board.raised([n], [n])
  assert !board.raised([], [])
}

/// The open task's Depends on line updates live: a dependency's update,
/// even in another project, goes to its details; another task's doesn't.
pub fn a_dependency_update_goes_to_the_open_detail_test() {
  let model =
    deps_board()
    |> put_task("b", "d1", "Dep", "backlog", "")
    |> put_task("a", "1", "Card", "backlog", ",\"dependsOn\":[\"d1\"]")
  let #(model, _) = board.update(model, board.Select("a", "1"))
  let dep =
    "{\"project\":\"b\",\"task\":{\"id\":\"d1\",\"title\":\"Dep\",\"body\":\"\",\"column\":\"complete\",\"rank\":1}}"
  let #(model, _) = board.update(model, board.Live(dep))
  let #(model, _) =
    board.update(
      model,
      board.Live(
        "{\"project\":\"b\",\"task\":{\"id\":\"d2\",\"title\":\"Other\",\"body\":\"\",\"column\":\"complete\",\"rank\":1}}",
      ),
    )
  model.live |> should.equal([#(1, dep)])
}

/// D-112: a feature's Tech Debt card carries its mark; another card doesn't.
pub fn tech_debt_card_is_marked_test() {
  let model =
    deps_board()
    |> put_task("a", "1", "Search: Tech Debt", "planning", ",\"techDebt\":true")
    |> put_task("a", "2", "Card", "planning", "")
  string.contains(card_of(model, "Search: Tech Debt"), "class=\"debt\"")
  |> should.be_true
  string.contains(card_of(model, "Card"), "class=\"debt\"") |> should.be_false
}

/// A card's stripe is its feature's color, by the slot the server kept when
/// the feature started (D-114); a task with no feature, or one not started,
/// keeps the gray stripe, and the agent's status no longer colors it. The
/// card names its feature in its tooltip and for screen readers.
pub fn a_card_takes_its_feature_color_test() {
  let model =
    deps_board()
    |> board.update(board.Live(
      "{\"features\":[{\"id\":\"f2\",\"title\":\"Export\",\"colorSlot\":1,\"state\":\"active\"},{\"id\":\"f1\",\"title\":\"Search\",\"colorSlot\":0,\"state\":\"complete\"},{\"id\":\"f3\",\"colorSlot\":null,\"state\":\"active\"}]}",
    ))
    |> fn(r) { r.0 }
    |> put_task("a", "1", "One", "planning", ",\"feature\":\"f1\"")
    |> put_task("a", "2", "Two", "planning", ",\"feature\":\"f2\"")
    |> put_task("a", "3", "Idle", "planning", ",\"feature\":\"f3\"")
    |> put_task("a", "4", "Loose", "planning", "")
  card_of(model, "One")
  |> string.contains("style=\"--tab:" <> solace_task.feature_color(0))
  |> should.be_true
  card_of(model, "Two")
  |> string.contains("style=\"--tab:" <> solace_task.feature_color(1))
  |> should.be_true
  card_of(model, "Two")
  |> string.contains("title=\"Feature: Export\"")
  |> should.be_true
  card_of(model, "Two")
  |> string.contains("<span class=\"sr-only\"> · Feature: Export</span>")
  |> should.be_true
  string.contains(card_of(model, "Idle"), "--tab") |> should.be_false
  string.contains(card_of(model, "Loose"), "--tab") |> should.be_false
  string.contains(card_of(model, "Loose"), "Feature:") |> should.be_false
  // A feature that starts takes the slot the server gives it; a removed one
  // moves no other feature's color.
  let model =
    board.update(
      model,
      board.Live(
        "{\"feature\":{\"id\":\"f3\",\"colorSlot\":2,\"state\":\"active\"}}",
      ),
    ).0
  card_of(model, "Idle")
  |> string.contains("style=\"--tab:" <> solace_task.feature_color(2))
  |> should.be_true
  let model = board.update(model, board.Live("{\"removedFeature\":\"f1\"}")).0
  card_of(model, "Two")
  |> string.contains("style=\"--tab:" <> solace_task.feature_color(1))
  |> should.be_true
  string.contains(board.view(model) |> element.to_string, ".card:has(.working)")
  |> should.be_false
}

/// A card with no feature, or one not started, has a gray stripe that no
/// feature color shares.
pub fn a_card_without_a_feature_is_gray_test() {
  let html = board.view(deps_board()) |> element.to_string
  string.contains(html, "--tab:#999999") |> should.be_true
  string.contains(html, "--tab:var(--file)") |> should.be_false
}

/// The default workflow's column names show their labels in the headings and
/// the column sections, not the ids, and a final column's card has its
/// Archive button. Backlog is not drawn, so Planning comes first (D-133).
pub fn default_workflow_shows_its_column_labels_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.WorkflowLoaded(
        "",
        "{\"columns\":[{\"id\":\"backlog\",\"label\":\"Backlog\",\"actions\":[\"plan\",\"cancel\"]},{\"id\":\"planning\",\"label\":\"Planning\",\"actions\":[\"investigate\",\"assign\",\"complete\",\"to_backlog\",\"cancel\"]},{\"id\":\"assigned\",\"label\":\"Assigned\",\"actions\":[\"reassign\",\"new_agent\",\"review\",\"to_planning\",\"cancel\"]},{\"id\":\"needs_review\",\"label\":\"Needs review\",\"actions\":[\"approve\",\"complete\",\"review\",\"reassign\",\"start_over\",\"to_planning\",\"cancel\"]},{\"id\":\"complete\",\"label\":\"Complete\",\"final\":true},{\"id\":\"cancelled\",\"label\":\"Canceled\",\"final\":true,\"actions\":[\"to_planning\"]}]}",
      ),
    )
  let #(model, _) = board.update(model, board.CaughtUp)
  let card = fn(id, title, column) {
    "{\"task\":{\"id\":\""
    <> id
    <> "\",\"title\":\""
    <> title
    <> "\",\"body\":\"\",\"column\":\""
    <> column
    <> "\",\"rank\":1}}"
  }
  let #(model, _) =
    board.update(model, board.Live(card("1", "Plan me", "planning")))
  let #(model, _) =
    board.update(model, board.Live(card("2", "Done", "complete")))
  let rendered = board.view(model) |> element.to_string
  rendered |> string.contains("<h2>Backlog</h2>") |> should.be_false
  rendered
  |> string.contains(
    "<div class=\"board\"><section aria-label=\"Planning\" class=\"column\"><h2>Planning</h2>",
  )
  |> should.be_true
  rendered |> string.contains("<h2>Assigned</h2>") |> should.be_true
  rendered |> string.contains("<h2>Needs review</h2>") |> should.be_true
  rendered |> string.contains("<h2>Complete</h2>") |> should.be_true
  rendered |> string.contains("<h2>Canceled</h2>") |> should.be_true
  rendered |> string.contains("<h2>needs_review</h2>") |> should.be_false
  rendered |> string.contains("aria-label=\"Needs review\"") |> should.be_true
  rendered |> string.contains(">Archive</button>") |> should.be_true
  rendered |> string.contains("aria-label=\"Archive Done\"") |> should.be_true
}

/// The board's fixed controls say the shared names from `ui_labels`: the
/// header, the lanes, the Projects panel, the removal dialog and the notices.
/// Tasks are created on the Strategy page: the board has no New task button
/// and no editor section.
pub fn shared_labels_on_the_board_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\"a\",\"label\":\"a\",\"name\":\"a\",\"base\":\"/p/a\"},{\"dir\":\"b\",\"label\":\"b\",\"name\":\"b\",\"base\":\"/p/b\"}],\"found\":[{\"dir\":\"tools\",\"name\":\"tools\",\"imported\":true,\"member\":false}]}",
      ),
    )
  let columns = "{\"columns\":[{\"id\":\"backlog\",\"label\":\"Backlog\"}]}"
  let #(model, _) = board.update(model, board.WorkflowLoaded("a", columns))
  let #(model, _) = board.update(model, board.WorkflowLoaded("b", columns))
  let #(model, _) = board.update(model, board.CaughtUp)
  let shows = fn(model, text) {
    board.view(model)
    |> element.to_string
    |> string.split("</style>")
    |> list.last
    |> result.unwrap("")
    |> string.contains(text)
  }
  let button = fn(name) { ">" <> solace_task.ui_label(name) <> "</button>" }
  shows(model, button("new_task")) |> should.be_false
  shows(model, button("create_task")) |> should.be_false
  shows(model, "<section class=\"editor\">") |> should.be_false
  // The guide is the shell's Knowledgebase page (D-140): no Guide link.
  shows(model, "href=\"/guide\"") |> should.be_false
  shows(model, ">Guide</a>") |> should.be_false
  shows(model, button("projects")) |> should.be_true
  shows(model, ">" <> solace_task.ui_label("contract_view") <> "</a>")
  |> should.be_true
  // The Projects panel: no heading repeating its button, then Remove, Add
  // and Create project.
  let #(model, _) = board.update(model, board.ToggleProjects)
  shows(model, "<h2>" <> solace_task.ui_label("projects") <> "</h2>")
  |> should.be_false
  shows(model, button("remove")) |> should.be_true
  shows(model, button("add")) |> should.be_true
  shows(model, button("create_project")) |> should.be_true
  // The removal dialog keeps the shared Remove, Cancel and Delete.
  let #(model, _) = board.update(model, board.RemoveProject("a"))
  shows(model, button("remove_only")) |> should.be_true
  shows(model, button("cancel")) |> should.be_true
  let #(model, _) = board.update(model, board.RemoveFiles)
  let #(model, _) = board.update(model, board.RemoveTyped("a"))
  shows(model, button("delete_permanently"))
  |> should.be_true
  let #(model, _) = board.update(model, board.RemoveCancel)
  let #(model, _) = board.update(model, board.ToggleProjects)
  // A finished run's notice closes with Show task and Dismiss.
  let running =
    "{\"project\":\"a\",\"task\":{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1,\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"running\",\"model\":\"m\",\"log\":\"/l\"}]}}"
  let ended =
    string.replace(running, "\"running\"", "\"ended\",\"outcome\":\"finished\"")
  let #(model, _) = board.update(model, board.Live(running))
  let #(model, _) = board.update(model, board.Live(ended))
  shows(model, button("show_task")) |> should.be_true
  shows(model, "aria-label=\"" <> solace_task.ui_label("dismiss") <> "\"")
  |> should.be_true
}

pub fn dismissed_notices_go_test() {
  let #(model, _) = board.init(Nil)
  let one = solace_task.Notice("Reviewed A", "done", "", "1", "runs")
  let two = solace_task.Notice("Reviewed B", "done", "", "2", "runs")
  let model = board.Model(..model, notices: [one, two])
  let #(model, _) = board.update(model, board.Dismiss(one))
  model.notices |> should.equal([two])
  let #(model, _) = board.update(model, board.Dismiss(one))
  model.notices |> should.equal([two])
}

/// Text areas and inputs fill their container: `width:100%` with
/// `box-sizing:border-box`, no percentage cap; the new-project row's input
/// keeps its flex growth, and the removal dialog's input drops its margins so
/// `width:100%` fits the dialog (whose `overflow:auto` would show a scrollbar
/// for the base rule's 4px side margins).
pub fn textareas_fill_their_container_test() {
  let #(model, _) = board.init(Nil)
  let assert [css, ..] =
    board.view(model) |> element.to_string |> string.split("</style>")
  css
  |> string.contains(
    "textarea{display:block;width:100%;box-sizing:border-box;min-height:70px}",
  )
  |> should.be_true
  css
  |> string.contains("input{width:100%;box-sizing:border-box}")
  |> should.be_true
  css |> string.contains("min(95%") |> should.be_false
  css |> string.contains("min(80%") |> should.be_false
  css
  |> string.contains(".new-project input{flex:1;min-width:220px}")
  |> should.be_true
  css |> string.contains(".removal input{margin:0}") |> should.be_true
}

/// The detail is a band pinned to the bottom of the board page: selecting a
/// card puts `<solace-task>` in the band, whose bar names the task and
/// collapses it to one line; expanding shows the details again.
pub fn detail_band_test() {
  let model = deps_board() |> put_task("a", "1", "Detail task", "backlog", "")
  // Nothing selected: the band shows the select message, no collapse control.
  let html = board.view(model) |> element.to_string
  string.contains(html, "class=\"detail-band\"") |> should.be_true
  string.contains(html, "Select a task to see its details.") |> should.be_true
  let assert Ok(bar) =
    html |> string.split("<header class=\"detail-bar\"") |> list.last
  bar
  |> string.split("</header>")
  |> list.first
  |> result.unwrap("")
  |> string.contains("<button")
  |> should.be_false
  // Selecting a card: the band holds its details and its bar names the task.
  let #(model, _) = board.update(model, board.Select("a", "1"))
  let html = board.view(model) |> element.to_string
  string.contains(html, "class=\"detail-band\"") |> should.be_true
  string.contains(html, "<solace-task base=\"/p/a\"") |> should.be_true
  string.contains(html, "detail-bar\"") |> should.be_true
  string.contains(html, "aria-expanded=\"true\"") |> should.be_true
  // The bar names the task: its title is inside the toggle, not just on a card.
  let assert Ok(bar) =
    html
    |> string.split("<header class=\"detail-bar\"")
    |> list.find(fn(s) { string.contains(s, "Detail task") })
  let assert Ok(bar) = bar |> string.split("</header>") |> list.first
  bar |> string.contains("<solace-task base=\"/p/a\"") |> should.be_false
  // Collapsing: the bar stays, the details hide, the control says collapsed.
  let #(model, _) = board.update(model, board.ToggleDetail)
  model.detail_collapsed |> should.be_true
  let html = board.view(model) |> element.to_string
  string.contains(html, "class=\"detail-band collapsed\"") |> should.be_true
  string.contains(html, "aria-expanded=\"false\"") |> should.be_true
  // The task element stays in the band, hidden by the band's CSS, so its
  // loaded task and live events survive a collapse.
  string.contains(html, "<solace-task base=\"/p/a\"") |> should.be_true
  string.contains(
    html,
    ".detail-band.collapsed > solace-task{height:0 !important;overflow:hidden;visibility:hidden}",
  )
  |> should.be_true
  // The bar lines up with the lanes, and the band's default height leaves
  // the lanes room on short windows.
  string.contains(
    html,
    ".detail-bar{display:flex;gap:8px;align-items:center;padding:10px 0}",
  )
  |> should.be_true
  string.contains(
    html,
    ".detail-band > solace-task{flex:none;height:min(45dvh,calc(100dvh - 190px));min-height:0;overflow-y:auto;visibility:visible;transition:height .18s ease-out,visibility .18s}",
  )
  |> should.be_true
  // Expanding shows the details again.
  let #(model, _) = board.update(model, board.ToggleDetail)
  model.detail_collapsed |> should.be_false
  let html = board.view(model) |> element.to_string
  string.contains(html, "class=\"detail-band collapsed\"") |> should.be_false
  string.contains(html, "<solace-task base=\"/p/a\"") |> should.be_true
}

pub fn showing_a_task_expands_the_band_test() {
  let #(model, _) = board.init(Nil)
  let collapsed = board.Model(..model, detail_collapsed: True)
  let #(m, _) = board.update(collapsed, board.Select("a", "1"))
  m.detail_collapsed |> should.equal(False)
  let #(m, _) = board.update(collapsed, board.Focus("a", "1"))
  m.detail_collapsed |> should.equal(False)
  let notice = solace_task.Notice("Reviewed A", "done", "a", "1", "runs")
  let #(m, _) = board.update(collapsed, board.Open(notice))
  m.detail_collapsed |> should.equal(False)
}

/// The gap above the column names is closed: no alert paragraph when there
/// is no error, and no margins above the lanes inside the scroll area; the
/// scroll area keeps room for its scrollbar so the lanes stay clear of it.
pub fn lanes_gap_test() {
  let model = deps_board()
  board.view(model)
  |> element.to_string
  |> string.contains("role=\"alert\"")
  |> should.be_false
  let #(model, _) = board.update(model, board.ProjectsLoaded("not json"))
  board.view(model)
  |> element.to_string
  |> string.contains("role=\"alert\"")
  |> should.be_true
  let css =
    board.view(model)
    |> element.to_string
    |> string.split("</style>")
    |> list.first
    |> result.unwrap("")
  css
  |> string.contains(".lanes-area > .lanes{margin:0}")
  |> should.be_true
  css |> string.contains(".lanes-area .board{margin:0}") |> should.be_true
  css |> string.contains("scrollbar-gutter:stable") |> should.be_true
}

/// The band's top edge carries a resize handle when a task is open; it is
/// gone when the band is collapsed or no task is open. The handle's own CSS
/// rules sit in the page's style block, so the checks read the page after
/// that block.
pub fn detail_resize_handle_test() {
  let model = deps_board() |> put_task("a", "1", "Detail task", "backlog", "")
  let rendered = fn(model) {
    model
    |> board.view
    |> element.to_string
    |> string.split("</style>")
    |> list.last
    |> result.unwrap("")
  }
  // No task open: no handle.
  rendered(model) |> string.contains("detail-resize") |> should.be_false
  let #(model, _) = board.update(model, board.Select("a", "1"))
  let html = rendered(model)
  html |> string.contains("class=\"detail-resize\"") |> should.be_true
  html |> string.contains("role=\"separator\"") |> should.be_true
  // Collapsed: the handle goes with the height.
  let #(model, _) = board.update(model, board.ToggleDetail)
  rendered(model) |> string.contains("detail-resize") |> should.be_false
}

/// Dragging the handle moves the band's height, clamped so the board keeps
/// its column bar; the height is kept when the drag ends, and rendered on
/// the task element.
pub fn detail_height_drag_test() {
  let model = deps_board() |> put_task("a", "1", "Detail task", "backlog", "")
  let #(model, _) = board.update(model, board.Select("a", "1"))
  let body = fn(m: board.Model) {
    m
    |> board.view
    |> element.to_string
    |> string.split("</style>")
    |> list.last
    |> result.unwrap("")
  }
  // The default: no inline height; the CSS default applies.
  body(model) |> string.contains("height:") |> should.be_false
  let #(model, _) = board.update(model, board.ResizeBegin(500.0, 300.0, 600))
  let #(model, _) = board.update(model, board.ResizeMove(400.0))
  model.detail_height |> should.equal(400)
  body(model) |> string.contains("height:400px") |> should.be_true
  // A drag past the bounds clamps.
  let #(model, _) = board.update(model, board.ResizeMove(0.0))
  model.detail_height |> should.equal(600)
  let #(model, _) = board.update(model, board.ResizeMove(1000.0))
  model.detail_height |> should.equal(120)
  // The drag ends: the height stays and the drag is over.
  let #(model, _) = board.update(model, board.ResizeEnd)
  model.resizing |> should.equal(None)
  model.detail_height |> should.equal(120)
  // A move with no drag in progress changes nothing.
  let #(model, _) = board.update(model, board.ResizeMove(400.0))
  model.detail_height |> should.equal(120)
}

/// The handle's keys: the arrows step the height by 24px, Home returns the
/// default, and other keys change nothing.
pub fn detail_height_keys_test() {
  let model = deps_board() |> put_task("a", "1", "Detail task", "backlog", "")
  let #(model, _) = board.update(model, board.Select("a", "1"))
  let model = board.Model(..model, detail_height: 300)
  let #(model, _) = board.update(model, board.ResizeKey("ArrowUp"))
  model.detail_height |> should.equal(324)
  let #(model, _) = board.update(model, board.ResizeKey("ArrowDown"))
  model.detail_height |> should.equal(300)
  let #(model, _) = board.update(model, board.ResizeKey("Home"))
  model.detail_height |> should.equal(0)
  let #(model, _) = board.update(model, board.ResizeKey("Enter"))
  model.detail_height |> should.equal(0)
  // From the CSS default, a step starts from the height that shows (45% of
  // the 800px fallback window outside a browser: 360), not from 0.
  let #(up, _) = board.update(model, board.ResizeKey("ArrowUp"))
  up.detail_height |> should.equal(384)
  let #(down, _) = board.update(model, board.ResizeKey("ArrowDown"))
  down.detail_height |> should.equal(336)
}

/// D-128: a task shows on the board once its feature is active, and stays
/// when the feature completes; a proposed, upcoming or canceled feature's
/// tasks are off the board. A task with no feature, or a feature not loaded
/// yet, shows.
pub fn only_active_features_show_their_tasks_test() {
  let shown = fn(model, title) {
    string.contains(element.to_string(board.view(model)), title)
  }
  let model =
    deps_board()
    |> board.update(board.Live(
      "{\"features\":[{\"id\":\"f1\",\"state\":\"proposed\"},{\"id\":\"f2\",\"state\":\"upcoming\"},{\"id\":\"f3\",\"state\":\"canceled\"},{\"id\":\"f4\",\"state\":\"complete\"}]}",
    ))
    |> fn(r) { r.0 }
    |> put_task("a", "1", "Idea", "planning", ",\"feature\":\"f1\"")
    |> put_task("a", "2", "Queued", "planning", ",\"feature\":\"f2\"")
    |> put_task("a", "3", "Dropped", "complete", ",\"feature\":\"f3\"")
    |> put_task("a", "4", "Shipped", "planning", ",\"feature\":\"f4\"")
    |> put_task("a", "5", "Loose", "planning", "")
    |> put_task("a", "6", "Unknown", "planning", ",\"feature\":\"f9\"")
  shown(model, "Idea") |> should.be_false
  shown(model, "Queued") |> should.be_false
  shown(model, "Dropped") |> should.be_false
  shown(model, "Shipped") |> should.be_true
  shown(model, "Loose") |> should.be_true
  shown(model, "Unknown") |> should.be_true
  let model =
    board.update(
      model,
      board.Live("{\"feature\":{\"id\":\"f2\",\"state\":\"active\"}}"),
    ).0
  shown(model, "Queued") |> should.be_true
}

/// The board page carries the shared control styles, so its controls and
/// the option lists of selects follow the theme.
pub fn page_uses_the_shared_control_styles_test() {
  let html = board.view(deps_board()) |> element.to_string
  string.contains(html, "option,optgroup{background:var(--paper)")
  |> should.be_true
}

/// D-133: Backlog means not active, so the board shows none of its tasks,
/// with or without a feature, and doesn't count them; the same task in
/// Planning shows.
pub fn backlog_tasks_are_off_the_board_test() {
  let shown = fn(model, title) {
    string.contains(element.to_string(board.view(model)), title)
  }
  let model =
    deps_board()
    |> board.update(board.Live(
      "{\"features\":[{\"id\":\"f1\",\"state\":\"active\"}]}",
    ))
    |> fn(r) { r.0 }
    |> put_task("a", "1", "Loose", "backlog", "")
    |> put_task("a", "2", "Featured", "backlog", ",\"feature\":\"f1\"")
  shown(model, "Loose") |> should.be_false
  shown(model, "Featured") |> should.be_false
  shown(model, "<h2>Backlog</h2>") |> should.be_false
  // Neither lane counts them.
  element.to_string(board.view(model))
  |> string.split("<small>0 tasks</small>")
  |> list.length
  |> should.equal(3)
  let model = put_task(model, "a", "1", "Loose", "planning", "")
  shown(model, "Loose") |> should.be_true
  shown(model, "Featured") |> should.be_false
}

/// D-129: the tasks in no project are the Strategy page's. Their list, sent
/// at the end of every new stream without a `project`, doesn't replace the
/// root lane's tasks, and a single one isn't added to it, replayed or live.
pub fn no_project_messages_leave_the_root_lane_test() {
  let titles = fn(model: board.Model) {
    let assert [root, _] = model.lanes
    list.map(root.tasks, fn(t) { t.title })
  }
  let loose =
    "{\"task\":{\"id\":\"9\",\"title\":\"Loose\",\"body\":\"\",\"column\":\"planning\",\"rank\":1},\"noProject\":true}"
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\".\",\"label\":\"root\",\"name\":\"root\",\"base\":\"\"},{\"dir\":\"web\",\"label\":\"web\",\"name\":\"web\",\"base\":\"/p/web\"}],\"found\":[]}",
      ),
    )
  let #(model, _) = board.update(model, board.CaughtUp)
  let model = put_task(model, ".", "1", "Root", "planning", "")
  // Live.
  let #(model, _) =
    board.update(model, board.Live("{\"tasks\":[],\"noProject\":true}"))
  titles(model) |> should.equal(["Root"])
  let #(model, _) = board.update(model, board.Live(loose))
  titles(model) |> should.equal(["Root"])
  model.replay |> should.equal([])
  // Replayed when a new stream catches up.
  let model = board.Model(..model, caught_up: False)
  let #(model, _) =
    board.update(model, board.Live("{\"tasks\":[],\"noProject\":true}"))
  let #(model, _) = board.update(model, board.Live(loose))
  let #(model, _) = board.update(model, board.CaughtUp)
  titles(model) |> should.equal(["Root"])
  model.replay |> should.equal([])
}
