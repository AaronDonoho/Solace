import gleam/json
import gleam/list
import gleam/option.{None, Some}
import gleam/result
import gleam/string
import gleeunit
import gleeunit/should
import lustre/element
import solace_board as board

pub fn main() {
  gleeunit.main()
}

fn tasks(model: board.Model) -> List(board.Task) {
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
  let assert Ok(task) = json.parse(text, board.task_decoder())
  board.actions(board.workflow(model), task)
  |> should.equal(["assign", "pause", "discuss"])
  let #(model, _) = board.update(model, board.TaskLoaded(text))
  model.selected |> should.equal(Some(task))
  let #(model, _) = board.update(model, board.CaughtUp)
  let #(model, _) = board.update(model, board.Live("{\"task\":" <> text <> "}"))
  list.length(tasks(model)) |> should.equal(1)
  let rendered = board.view(model) |> element.to_string
  rendered |> should.not_equal("")
  let #(model, _) =
    board.update(
      model,
      board.Changed("{\"code\":\"E7006\",\"message\":\"Approval required\"}"),
    )
  model.error |> should.equal("Approval required")
}

pub fn attention_and_unavailable_models_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.ModelsLoaded(
        "[{\"id\":\"gone\",\"label\":\"Gone\",\"available\":false,\"reason\":\"codex is not installed\"},{\"id\":\"ok\",\"label\":\"Ok\",\"available\":true,\"reason\":null}]",
      ),
    )
  // The first available model is chosen by default.
  model.model |> should.equal("ok")
  let #(model, _) =
    board.update(
      model,
      board.TaskLoaded(
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"blocked\",\"rank\":1,\"attention\":{\"reason\":\"task timeout\",\"time\":1}}",
      ),
    )
  let rendered = board.view(model) |> element.to_string
  rendered |> string.contains("Needs attention</strong>") |> should.be_true
  rendered |> string.contains("<p>task timeout</p>") |> should.be_true
  rendered
  |> string.contains("unavailable: codex is not installed")
  |> should.be_true
}

pub fn working_status_notes_and_transcript_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.TaskLoaded(
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1,\"notes\":[{\"author\":\"agent\",\"text\":\"Done: **five** tests <b>pass</b>\",\"time\":1}],\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"running\",\"model\":\"m\",\"log\":\"/l\",\"started\":1}]}",
      ),
    )
  let assert Some(task) = model.selected
  board.status(task) |> should.equal("Agent working")
  let #(model, _) = board.update(model, board.Tab("notes"))
  let rendered = board.view(model) |> element.to_string
  rendered |> string.contains("status working") |> should.be_true
  // Markdown is formatted, and agent HTML is escaped.
  rendered |> string.contains("<strong>five</strong>") |> should.be_true
  rendered |> string.contains("&lt;b&gt;pass") |> should.be_true
  let #(model, _) = board.update(model, board.Tab("runs"))
  board.view(model)
  |> element.to_string
  |> string.contains("Follow live")
  |> should.be_true
  let #(model, _) = board.update(model, board.Watch("r1"))
  let #(model, _) =
    board.update(
      model,
      board.TranscriptLoaded(
        "r1",
        "{\"state\":\"running\",\"transcript\":[{\"kind\":\"message\",\"text\":\"Looking\"},{\"kind\":\"tool\",\"title\":\"solace view a\",\"input\":\"\",\"output\":\"pub fn a()\",\"status\":\"ok\"}]}",
      ),
    )
  let rendered = board.view(model) |> element.to_string
  rendered |> string.contains("solace view a") |> should.be_true
  rendered |> string.contains("Hide activity") |> should.be_true
  // A transcript for a run no longer watched is dropped.
  let #(model, _) = board.update(model, board.Unwatch)
  let #(model, _) =
    board.update(
      model,
      board.TranscriptLoaded("r1", "{\"state\":\"running\",\"transcript\":[]}"),
    )
  model.entries |> should.equal([])
}

pub fn notes_newest_first_with_agent_and_finished_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.TaskLoaded(
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"needs_review\",\"rank\":1,\"finished\":{\"kind\":\"work\",\"outcome\":\"finished\",\"time\":3},\"notes\":[{\"author\":\"human\",\"run\":null,\"text\":\"older\",\"time\":1},{\"author\":\"agent\",\"run\":\"r1\",\"text\":\"newer\",\"time\":2}],\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"ended\",\"model\":\"claude-opus\",\"effort\":\"high\",\"log\":\"/l\",\"outcome\":\"finished\"}]}",
      ),
    )
  let assert Some(task) = model.selected
  board.status(task) |> should.equal("Agent finished")
  let #(model, _) = board.update(model, board.Tab("notes"))
  let rendered = board.view(model) |> element.to_string
  rendered |> string.contains("status finished") |> should.be_true
  rendered
  |> string.contains("agent · work · claude-opus · high")
  |> should.be_true
  let assert [_, after] = string.split(rendered, "newer")
  after |> string.contains("older") |> should.be_true
}

pub fn effort_and_create_draft_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.ModelsLoaded(
        "[{\"id\":\"c\",\"label\":\"Claude\",\"effort\":\"medium\",\"efforts\":[\"low\",\"max\"]}]",
      ),
    )
  let #(model, _) =
    board.update(
      model,
      board.TaskLoaded(
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"planning\",\"rank\":1}",
      ),
    )
  let rendered = board.view(model) |> element.to_string
  rendered |> string.contains("Default (medium)") |> should.be_true
  rendered |> string.contains(">max</option>") |> should.be_true
  let #(model, _) = board.update(model, board.EffortSelected("max"))
  model.effort |> should.equal("max")
  let #(model, _) = board.update(model, board.AgentSelected("c"))
  model.effort |> should.equal("")
  // A failed create keeps the draft; a created task clears it.
  let #(model, _) = board.update(model, board.NewTask)
  let #(model, _) = board.update(model, board.DraftTitle("Draft"))
  let #(model, _) =
    board.update(
      model,
      board.Created("{\"code\":\"E7001\",\"message\":\"title required\"}"),
    )
  #(model.draft_title, model.creating) |> should.equal(#("Draft", True))
  let #(model, _) =
    board.update(
      model,
      board.Created(
        "{\"id\":\"2\",\"title\":\"Draft\",\"body\":\"\",\"column\":\"backlog\",\"rank\":2}",
      ),
    )
  #(model.draft_title, model.creating) |> should.equal(#("", False))
}

pub fn settings_question_shows_diff_and_approve_test() {
  board.settings_diff("a=1\nb=2", "a=1\nb=3")
  |> should.equal("- b=2\n+ b=3")
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.TaskLoaded(
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"blocked\",\"rank\":1,\"questions\":[{\"id\":\"q\",\"text\":\"Change solace.toml\",\"options\":[\"Approve\",\"Reject\"],\"settings\":{\"base\":\"x=1\",\"proposed\":\"x=1\\n[commands]\"}}]}",
      ),
    )
  let #(model, _) = board.update(model, board.Tab("questions"))
  let rendered = board.view(model) |> element.to_string
  rendered |> string.contains("+ [commands]") |> should.be_true
  rendered |> string.contains(">Approve</button>") |> should.be_true
  rendered |> string.contains("Reject with the editor") |> should.be_true
}

pub fn notices_and_stamp_test() {
  let assert Ok(old) =
    json.parse(
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1,\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"running\",\"model\":\"m\",\"log\":\"/l\"}]}",
      board.task_decoder(),
    )
  let assert Ok(new) =
    json.parse(
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"needs_review\",\"rank\":1,\"stamps\":{\"approved\":{}},\"questions\":[{\"id\":\"q\",\"text\":\"Why?\",\"options\":[]}],\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"ended\",\"model\":\"m\",\"log\":\"/l\",\"outcome\":\"finished\"}]}",
      board.task_decoder(),
    )
  board.notices(old, new)
  |> should.equal([
    board.Notice("Finished T", "The agent finished its work."),
    board.Notice("Question on T", "Why?"),
  ])
  board.notices(new, new) |> should.equal([])
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
          board.Workflow([
            board.Column("needs_review", "Needs review", [], False),
          ]),
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
  model.notices
  |> should.equal([board.Notice("Reviewed T", "The review is ready.")])
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
  let columns = "{\"columns\":[{\"id\":\"backlog\",\"label\":\"Backlog\"}]}"
  let #(model, _) =
    board.update(model, board.WorkflowLoaded("bench/pilot", columns))
  let #(model, _) = board.update(model, board.WorkflowLoaded("web", columns))
  board.shared_columns(model.lanes) |> should.be_true
  let task = fn(title) {
    "{\"id\":\"1\",\"title\":\""
    <> title
    <> "\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1,\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"running\",\"model\":\"m\",\"log\":\"/l\"}]}"
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
  rendered |> string.contains("<h2>Backlog</h2>") |> should.be_true
  rendered
  |> string.contains("href=\"/p/bench%2Fpilot/view\"")
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
    board.Notice("web: Finished In web", "The agent finished its work."),
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
  rendered |> string.contains("lane-head") |> should.be_false
  rendered |> string.contains("<h2>todo</h2>") |> should.be_true
  // A collapsed lane hides its columns.
  let #(model, _) = board.update(model, board.Collapse("web"))
  board.view(model)
  |> element.to_string
  |> string.contains("<h2>todo</h2>")
  |> should.be_false
  // Its tasks go to the project's own API.
  let #(model, _) = board.update(model, board.Select("bench/pilot", "1"))
  model.lane |> should.equal("bench/pilot")
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
  rendered |> string.contains("href=\"/view\"") |> should.be_true
  rendered |> string.contains("href=\"/p/frontend/view\"") |> should.be_true
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
    "<header class=\"lane-title\"><span><strong>shop</strong><small> app</small><small class=\"lane-target muted\"> erlang</small></span><small>0 tasks</small><a href=\"/view\">Contract view</a></header>",
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
  let columns = "{\"columns\":[{\"id\":\"backlog\",\"label\":\"Backlog\"}]}"
  let model =
    list.fold(["a", "b", "c"], model, fn(m, d) {
      board.update(m, board.WorkflowLoaded(d, columns)).0
    })
  let #(model, _) = board.update(model, board.CaughtUp)
  let task =
    "{\"id\":\"t1\",\"title\":\"Across\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1,\"scope\":[\"a\",\"b\"],\"changes\":[{\"entity\":\"symbol:x\",\"name\":\"app.one\",\"project\":\"b\"}]}"
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
  let assert Ok(decoded) = json.parse(task, board.task_decoder())
  decoded.changes |> should.equal([board.Change("b", "app", "one")])
  // The task moved out of a: a's lane drops it.
  let #(model, _) =
    board.update(model, board.Live("{\"project\":\"a\",\"removed\":\"t1\"}"))
  board.view(model)
  |> element.to_string
  |> string.contains("Across")
  |> should.be_false
  // A new task's projects: the lane it starts from, others added; the
  // starting lane can be unchecked even when it is the only one.
  let #(model, _) = board.update(model, board.NewTaskIn("b"))
  let #(model, _) = board.update(model, board.DraftScope("b", False))
  model.draft_scope |> should.equal([])
  let #(model, _) = board.update(model, board.DraftScope("b", True))
  let #(model, _) = board.update(model, board.DraftScope("a", True))
  model.draft_scope |> should.equal(["a", "b"])
  let #(model, _) = board.update(model, board.DraftScope("a", False))
  let #(model, _) = board.update(model, board.DraftScope("b", False))
  model.draft_scope |> should.equal([])
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
  let columns = "{\"columns\":[{\"id\":\"backlog\",\"label\":\"Backlog\"}]}"
  let #(model, _) = board.update(model, board.WorkflowLoaded("a", columns))
  let #(model, _) = board.update(model, board.WorkflowLoaded("b", columns))
  let shared =
    "{\"id\":\"1\",\"title\":\"Across\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1,\"scope\":[\"a\",\"b\"]}"
  let other =
    "{\"id\":\"1\",\"title\":\"Other\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1}"
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
  let rendered = board.view(model) |> element.to_string
  string.split(rendered, "class=\"card selected\"")
  |> list.length
  |> should.equal(1)
  let #(model, _) = board.update(model, board.Select("a", "1"))
  let rendered = board.view(model) |> element.to_string
  string.split(rendered, "class=\"card selected\"")
  |> list.length
  |> should.equal(3)
  string.split(rendered, "aria-current=\"true\"")
  |> list.length
  |> should.equal(3)
  let #(model, _) =
    board.update(model, board.SelectedTaskLoaded(1, "a", "1", shared))
  let rendered = board.view(model) |> element.to_string
  string.split(rendered, "class=\"card selected\"")
  |> list.length
  |> should.equal(3)
  rendered
  |> string.contains(
    "button.card[aria-current=true],button.card[aria-current=true]:hover",
  )
  |> should.be_true
  let #(model, _) = board.update(model, board.Select("b", "1"))
  let #(model, _) =
    board.update(model, board.SelectedTaskLoaded(1, "a", "1", shared))
  model.selected |> should.equal(None)
  let rendered = board.view(model) |> element.to_string
  string.split(rendered, "class=\"card selected\"")
  |> list.length
  |> should.equal(2)
  let #(model, _) =
    board.update(model, board.SelectedTaskLoaded(2, "b", "1", other))
  let assert Some(task) = model.selected
  task.title |> should.equal("Other")
  let rendered = board.view(model) |> element.to_string
  string.split(rendered, "class=\"card selected\"")
  |> list.length
  |> should.equal(2)
  let #(model, _) = board.update(model, board.Select("a", "1"))
  let #(model, _) = board.update(model, board.Select("b", "1"))
  let #(model, _) = board.update(model, board.Select("a", "1"))
  let #(model, _) =
    board.update(model, board.SelectedTaskLoaded(3, "a", "1", shared))
  model.selected |> should.equal(None)
  let #(model, _) =
    board.update(model, board.SelectedTaskLoaded(5, "a", "1", shared))
  let assert Some(task) = model.selected
  task.title |> should.equal("Across")
  // The open task moves from a to b: details and highlight follow it.
  let moved =
    "{\"id\":\"1\",\"title\":\"Moved\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1,\"scope\":[\"b\",\"a\"]}"
  let #(model, _) =
    board.update(model, board.Live("{\"project\":\"b\",\"removed\":\"1\"}"))
  let #(model, _) =
    board.update(
      model,
      board.Live("{\"project\":\"b\",\"task\":" <> moved <> "}"),
    )
  let #(model, _) =
    board.update(model, board.Live("{\"project\":\"a\",\"removed\":\"1\"}"))
  let assert Some(task) = model.selected
  task.title |> should.equal("Moved")
  model.selected_card |> should.equal(Some(#("b", "1")))
  model.lane |> should.equal("b")
  let rendered = board.view(model) |> element.to_string
  string.split(rendered, "class=\"card selected\"")
  |> list.length
  |> should.equal(3)
}

pub fn models_loaded_keeps_choice_test() {
  let list_json =
    "[{\"id\":\"a\",\"label\":\"A\"},{\"id\":\"b\",\"label\":\"B\",\"efforts\":[\"low\",\"high\"]},{\"id\":\"c\",\"label\":\"C\",\"available\":false}]"
  let #(base, _) = board.init(Nil)
  let loaded = fn(model, text) {
    let #(m, _) = board.update(model, board.ModelsLoaded(text))
    m
  }
  // No choice yet: the first available agent.
  let first = loaded(board.Model(..base, model: "", effort: ""), list_json)
  assert first.model == "a"
  // A remembered available agent and its effort are kept.
  let kept = loaded(board.Model(..base, model: "b", effort: "high"), list_json)
  assert kept.model == "b"
  assert kept.effort == "high"
  // An effort the agent doesn't offer is dropped.
  let dropped =
    loaded(board.Model(..base, model: "b", effort: "max"), list_json)
  assert dropped.effort == ""
  // An unavailable or unknown agent falls back to the first available one.
  let unavailable =
    loaded(board.Model(..base, model: "c", effort: "high"), list_json)
  assert unavailable.model == "a"
  assert unavailable.effort == ""
  let unknown = loaded(board.Model(..base, model: "zzz", effort: ""), list_json)
  assert unknown.model == "a"
}

/// Source modules come before test modules, symbols are alphabetical, and a
/// task across projects names the project in each heading.
pub fn change_groups_test() {
  let changes = [
    board.Change("a", "app_test", "t_test"),
    board.Change("a", "util", "zed"),
    board.Change("a", "app", "beta"),
    board.Change("a", "app", "alpha"),
  ]
  board.change_groups(changes, False)
  |> should.equal([
    #("app", ["alpha", "beta"]),
    #("util", ["zed"]),
    #("app_test", ["t_test"]),
  ])
  board.change_groups([board.Change("b", "app", "one")], True)
  |> should.equal([#("b · app", ["one"])])
}

pub fn stop_and_discussion_actions_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.WorkflowLoaded(
        "",
        "{\"columns\":[{\"id\":\"triage\",\"label\":\"Triage\",\"actions\":[\"assign\"]}]}",
      ),
    )
  let task = fn(kind, state) {
    let text =
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"triage\",\"rank\":1,\"paused_from\":null,\"stamps\":{},\"notes\":[],\"questions\":[],\"runs\":[{\"id\":\"r\",\"kind\":\""
      <> kind
      <> "\",\"model\":\"m\",\"state\":\""
      <> state
      <> "\",\"started\":1,\"log\":\"\"}]}"
    let assert Ok(t) = json.parse(text, board.task_decoder())
    t
  }
  board.actions(board.workflow(model), task("work", "running"))
  |> should.equal(["assign", "pause", "stop"])
  board.actions(board.workflow(model), task("discuss", "waiting"))
  |> should.equal(["assign", "pause", "end_discussion", "stop"])
  board.status(task("discuss", "waiting"))
  |> should.equal("Waiting for your reply")
  board.status(task("discuss", "running")) |> should.equal("Agent discussing")
}
