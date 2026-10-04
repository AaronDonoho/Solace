import gleam/json
import gleam/list
import gleam/option.{Some}
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
  |> should.equal(["assign", "pause"])
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
  let rendered = board.view(model) |> element.to_string
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
  let rendered = board.view(model) |> element.to_string
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
