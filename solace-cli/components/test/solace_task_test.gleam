import gleam/dict
import gleam/int
import gleam/json
import gleam/list
import gleam/option.{None, Some}
import gleam/string
import gleeunit/should
import lustre/element
import solace_task as task

/// The element with `id` as its task, connected nowhere: messages drive it.
fn fresh(id: String) -> task.Model {
  let #(model, _) = task.init(Nil)
  task.Model(..model, id: id)
}

fn loaded(text: String) -> task.Model {
  let model = fresh("1")
  let #(model, _) = task.update(model, task.TaskLoaded(model.generation, text))
  model
}

fn html(model: task.Model) -> String {
  task.view(model) |> element.to_string
}

/// The rendered page without its styles.
fn body(model: task.Model) -> String {
  let assert [_, after] = string.split(html(model), "</style>")
  after
}

pub fn workflow_actions_and_errors_test() {
  let model = fresh("1")
  let #(model, _) =
    task.update(
      model,
      task.WorkflowLoaded(
        model.generation,
        "{\"columns\":[{\"id\":\"triage\",\"label\":\"Triage\",\"actions\":[\"assign\"]},{\"id\":\"done\",\"final\":true},{\"id\":\"gone\",\"final\":true,\"actions\":[\"to_backlog\"]}],\"actions\":{\"assign\":{\"to\":\"triage\"},\"to_backlog\":{\"to\":\"triage\",\"label\":\"Back to backlog\"}}}",
      ),
    )
  let text =
    "{\"id\":\"1\",\"title\":\"Fix checkout\",\"body\":\"Details\",\"column\":\"triage\",\"rank\":1,\"paused_from\":null,\"stamps\":{},\"notes\":[],\"questions\":[],\"runs\":[]}"
  let assert Ok(t) = json.parse(text, task.task_decoder())
  task.actions(model.workflow, t, False)
  |> should.equal(["assign", "pause", "discuss"])
  task.actions(model.workflow, task.Task(..t, column: "done"), False)
  |> should.equal([])
  task.actions(model.workflow, task.Task(..t, column: "gone"), False)
  |> should.equal(["to_backlog"])
  task.action_label(model.workflow, "to_backlog")
  |> should.equal("Back to backlog")
  task.action_label(model.workflow, "assign") |> should.equal("assign")
  let #(model, _) = task.update(model, task.TaskLoaded(model.generation, text))
  model.task |> should.equal(Some(t))
  html(model) |> string.contains("Fix checkout") |> should.be_true
  let #(gone, _) =
    task.update(
      model,
      task.TaskLoaded(
        model.generation,
        string.replace(text, "\"triage\"", "\"gone\""),
      ),
    )
  html(gone) |> string.contains(">Back to backlog</button>") |> should.be_true
  let #(model, _) =
    task.update(
      model,
      task.Changed("{\"code\":\"E7006\",\"message\":\"Approval required\"}"),
    )
  model.error |> should.equal("Approval required")
}

/// A reply for a task opened before, and a workflow from the project shown
/// before, are dropped.
pub fn replies_for_another_opening_are_dropped_test() {
  let model = fresh("1")
  let old = model.generation
  let #(model, _) = task.update(model, task.TaskChanged("2"))
  model.generation |> should.equal(old + 1)
  let #(model, _) =
    task.update(
      model,
      task.TaskLoaded(
        old,
        "{\"id\":\"1\",\"title\":\"Old\",\"body\":\"\",\"column\":\"x\",\"rank\":1}",
      ),
    )
  model.task |> should.equal(None)
  let #(model, _) =
    task.update(
      model,
      task.WorkflowLoaded(old, "{\"columns\":[{\"id\":\"x\"}]}"),
    )
  model.workflow |> should.equal(task.Workflow([], dict.new()))
  // The same value again opens nothing new.
  let #(same, _) = task.update(model, task.TaskChanged("2"))
  same |> should.equal(model)
}

pub fn attention_and_unavailable_models_test() {
  let model = fresh("1")
  let #(model, _) =
    task.update(
      model,
      task.ModelsLoaded(
        "[{\"id\":\"gone\",\"label\":\"Gone\",\"available\":false,\"reason\":\"codex is not installed\"},{\"id\":\"ok\",\"label\":\"Ok\",\"available\":true,\"reason\":null}]",
      ),
    )
  // The first available model is chosen by default.
  model.model |> should.equal("ok")
  let #(model, _) =
    task.update(
      model,
      task.TaskLoaded(
        model.generation,
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1,\"attention\":{\"reason\":\"task timeout\",\"time\":1,\"hold\":true}}",
      ),
    )
  let rendered = html(model)
  rendered |> string.contains("Needs attention</strong>") |> should.be_true
  rendered |> string.contains("<p>task timeout</p>") |> should.be_true
  // Only the model that can start is offered; the other is under the pick, with why.
  rendered |> string.contains("value=\"gone\"") |> should.be_false
  rendered |> string.contains(">Ok</option>") |> should.be_true
  rendered
  |> string.contains("<summary>1 more model unavailable</summary>")
  |> should.be_true
  rendered
  |> string.contains("<li>Gone: codex is not installed</li>")
  |> should.be_true
}

pub fn working_status_notes_and_transcript_test() {
  let model =
    loaded(
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1,\"notes\":[{\"author\":\"agent\",\"text\":\"Done: **five** tests <b>pass</b>\\n## Done\\n- fixed A\\n  - nested\",\"time\":1}],\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"running\",\"model\":\"m\",\"log\":\"/l\",\"started\":1}]}",
    )
  let assert Some(t) = model.task
  task.status(t) |> should.equal("Agent working")
  let #(model, _) = task.update(model, task.Tab("notes"))
  let rendered = html(model)
  rendered |> string.contains("status working") |> should.be_true
  // Markdown is formatted, and agent HTML is escaped.
  rendered |> string.contains("<strong>five</strong>") |> should.be_true
  rendered |> string.contains("&lt;b&gt;pass") |> should.be_true
  rendered |> string.contains("<h4>Done</h4>") |> should.be_true
  rendered |> string.contains("<ul><li>fixed A<ul><li>nested") |> should.be_true
  let #(model, _) = task.update(model, task.Tab("runs"))
  html(model) |> string.contains(">Watch</button>") |> should.be_true
  let #(model, _) = task.update(model, task.Watch("r1"))
  let #(model, _) =
    task.update(
      model,
      task.TranscriptLoaded(
        "r1",
        "{\"state\":\"running\",\"transcript\":[{\"kind\":\"message\",\"text\":\"Looking\"},{\"kind\":\"tool\",\"title\":\"solace view a\",\"input\":\"\",\"output\":\"pub fn a()\",\"status\":\"ok\"}]}",
      ),
    )
  let rendered = html(model)
  rendered |> string.contains("solace view a") |> should.be_true
  rendered |> string.contains("Hide activity") |> should.be_true
  // A transcript for a run no longer watched is dropped.
  let #(model, _) = task.update(model, task.Unwatch)
  let #(model, _) =
    task.update(
      model,
      task.TranscriptLoaded("r1", "{\"state\":\"running\",\"transcript\":[]}"),
    )
  model.entries |> should.equal([])
}

pub fn notes_newest_first_with_agent_and_finished_test() {
  let model =
    loaded(
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"needs_review\",\"rank\":1,\"finished\":{\"kind\":\"work\",\"outcome\":\"finished\",\"time\":3},\"notes\":[{\"author\":\"human\",\"run\":null,\"text\":\"older\",\"time\":1},{\"author\":\"agent\",\"run\":\"r1\",\"text\":\"newer\",\"time\":2}],\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"ended\",\"model\":\"claude-opus\",\"effort\":\"high\",\"log\":\"/l\",\"outcome\":\"finished\"}]}",
    )
  let assert Some(t) = model.task
  task.status(t) |> should.equal("Agent finished")
  let #(model, _) = task.update(model, task.Tab("notes"))
  let rendered = html(model)
  rendered |> string.contains("status finished") |> should.be_true
  rendered
  |> string.contains("agent · work · claude-opus · high")
  |> should.be_true
  let assert [_, after] = string.split(rendered, "newer")
  after |> string.contains("older") |> should.be_true
}

pub fn effort_choice_test() {
  let model = fresh("1")
  let #(model, _) =
    task.update(
      model,
      task.ModelsLoaded(
        "[{\"id\":\"c\",\"label\":\"Claude\",\"effort\":\"medium\",\"efforts\":[\"low\",\"max\"]}]",
      ),
    )
  let #(model, _) =
    task.update(
      model,
      task.TaskLoaded(
        model.generation,
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"planning\",\"rank\":1}",
      ),
    )
  let rendered = html(model)
  rendered |> string.contains("Default (medium)") |> should.be_true
  rendered |> string.contains(">max</option>") |> should.be_true
  let #(model, _) = task.update(model, task.EffortSelected("max"))
  model.effort |> should.equal("max")
  let #(model, _) = task.update(model, task.AgentSelected("c"))
  model.effort |> should.equal("")
}

pub fn models_loaded_keeps_choice_test() {
  let list_json =
    "[{\"id\":\"a\",\"label\":\"A\"},{\"id\":\"b\",\"label\":\"B\",\"efforts\":[\"low\",\"high\"]},{\"id\":\"c\",\"label\":\"C\",\"available\":false}]"
  let base = fresh("1")
  let loaded = fn(model, text) {
    let #(m, _) = task.update(model, task.ModelsLoaded(text))
    m
  }
  // No choice yet: the first available agent.
  let first = loaded(task.Model(..base, model: "", effort: ""), list_json)
  assert first.model == "a"
  // A remembered available agent and its effort are kept.
  let kept = loaded(task.Model(..base, model: "b", effort: "high"), list_json)
  assert kept.model == "b"
  assert kept.effort == "high"
  // An effort the agent doesn't offer is dropped.
  let dropped = loaded(task.Model(..base, model: "b", effort: "max"), list_json)
  assert dropped.effort == ""
  // An unavailable or unknown agent falls back to the first available one.
  let unavailable =
    loaded(task.Model(..base, model: "c", effort: "high"), list_json)
  assert unavailable.model == "a"
  assert unavailable.effort == ""
  let unknown = loaded(task.Model(..base, model: "zzz", effort: ""), list_json)
  assert unknown.model == "a"
}

pub fn settings_question_shows_diff_and_approve_test() {
  task.settings_diff("a=1\nb=2", "a=1\nb=3")
  |> should.equal("- b=2\n+ b=3")
  let model =
    loaded(
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1,\"questions\":[{\"id\":\"q\",\"text\":\"Change solace.toml\",\"options\":[\"Approve\",\"Reject\"],\"settings\":{\"base\":\"x=1\",\"proposed\":\"x=1\\n[commands]\"}}]}",
    )
  let #(model, _) = task.update(model, task.Tab("questions"))
  let rendered = html(model)
  rendered |> string.contains("+ [commands]") |> should.be_true
  rendered |> string.contains(">Approve</button>") |> should.be_true
  rendered |> string.contains("Reject with the editor") |> should.be_true
}

/// Source modules come before test modules, symbols are alphabetical, and a
/// task across projects names the project in each heading.
pub fn change_groups_test() {
  let changes = [
    task.Change("a", "app_test", "t_test"),
    task.Change("a", "util", "zed"),
    task.Change("a", "app", "beta"),
    task.Change("a", "app", "alpha"),
  ]
  task.change_groups(changes, False)
  |> should.equal([
    #("app", ["alpha", "beta"]),
    #("util", ["zed"]),
    #("app_test", ["t_test"]),
  ])
  task.change_groups([task.Change("b", "app", "one")], True)
  |> should.equal([#("b · app", ["one"])])
}

/// A file is listed by its path under Files, after the code; an entry that
/// is neither keeps its name under Other.
pub fn file_changes_list_under_files_test() {
  let text =
    "{\"id\":\"t1\",\"title\":\"T\",\"body\":\"\",\"column\":\"triage\",\"rank\":1,\"changes\":[{\"entity\":\"file:src/api.mjs\",\"name\":\"file:src/api.mjs\"},{\"entity\":\"symbol:y\",\"name\":\"pilot/checkout.pay\"},{\"entity\":\"module:m\",\"name\":\"module:m\"}]}"
  let assert Ok(t) = json.parse(text, task.task_decoder())
  task.change_groups(t.changes, False)
  |> should.equal([
    #("pilot/checkout", ["pay"]),
    #("Files", ["src/api.mjs"]),
    #("Other", ["module:m"]),
  ])
}

pub fn stop_and_discussion_actions_test() {
  let workflow =
    task.Workflow(
      [task.Column("triage", "Triage", ["assign"], False)],
      dict.new(),
    )
  let one = fn(kind, state) {
    let text =
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"triage\",\"rank\":1,\"paused_from\":null,\"stamps\":{},\"notes\":[],\"questions\":[],\"runs\":[{\"id\":\"r\",\"kind\":\""
      <> kind
      <> "\",\"model\":\"m\",\"state\":\""
      <> state
      <> "\",\"started\":1,\"log\":\"\"}]}"
    let assert Ok(t) = json.parse(text, task.task_decoder())
    t
  }
  task.actions(workflow, one("work", "running"), False)
  |> should.equal(["assign", "pause", "stop"])
  task.actions(workflow, one("discuss", "waiting"), False)
  |> should.equal(["assign", "pause", "end_discussion", "stop"])
  task.status(one("discuss", "waiting"))
  |> should.equal("Waiting for your reply")
  task.status(one("discuss", "running")) |> should.equal("Agent discussing")
}

/// An action in flight disables the action buttons and says what it waits for;
/// its reply clears that.
pub fn an_action_waits_for_its_reply_test() {
  let text =
    "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"triage\",\"rank\":1,\"paused_from\":null,\"stamps\":{},\"notes\":[],\"questions\":[],\"runs\":[{\"id\":\"r\",\"kind\":\"work\",\"model\":\"m\",\"state\":\"running\",\"started\":1,\"log\":\"\"}]}"
  let model =
    task.Model(
      ..loaded(text),
      workflow: task.Workflow(
        [task.Column("triage", "Triage", ["assign"], False)],
        dict.new(),
      ),
    )
  let #(model, _) = task.update(model, task.Action("assign"))
  model.acting |> should.equal("assign")
  let rendered = html(model)
  string.contains(rendered, "<button disabled>assign</button>")
  |> should.be_true
  string.contains(rendered, "current command to finish before assign")
  |> should.be_true
  // A second click while waiting sends nothing more.
  let #(again, _) = task.update(model, task.Action("assign"))
  again |> should.equal(model)
  let #(model, _) = task.update(model, task.Acted(text))
  model.acting |> should.equal("")
  let assert Ok(t) = json.parse(text, task.task_decoder())
  task.acting_text(task.Task(..t, runs: []), "assign")
  |> should.equal("Sending assign…")
}

pub fn typed_fields_tabs_and_previews_update_the_model_test() {
  let model = fresh("1")
  let #(model, _) = task.update(model, task.Text("hello"))
  let #(model, _) = task.update(model, task.Title("T2"))
  let #(model, _) = task.update(model, task.Body("B2"))
  let #(model, _) = task.update(model, task.Rank("3.5"))
  #(model.text, model.title, model.body, model.rank)
  |> should.equal(#("hello", "T2", "B2", "3.5"))
  let #(model, _) = task.update(model, task.Tab("notes"))
  model.tab |> should.equal("notes")
  let #(model, _) = task.update(model, task.Failed("boom"))
  model.error |> should.equal("boom")
}

pub fn editing_needs_a_task_and_a_numeric_rank_test() {
  let model = fresh("1")
  // With no task loaded, these change nothing.
  let #(same, _) = task.update(model, task.Edit(True))
  same |> should.equal(model)
  let #(same, _) = task.update(model, task.Save)
  same |> should.equal(model)
  let #(same, _) = task.update(model, task.AddNote)
  same |> should.equal(model)
  let #(same, _) = task.update(model, task.DeleteNote(0))
  same |> should.equal(model)
  let #(same, _) = task.update(model, task.Answer("q", "yes"))
  same |> should.equal(model)
  let #(same, _) = task.update(model, task.Action("assign"))
  same |> should.equal(model)
  let #(same, _) = task.update(model, task.SetScope("a", True))
  same |> should.equal(model)
  let model =
    loaded(
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"B\",\"column\":\"triage\",\"rank\":1,\"paused_from\":null,\"stamps\":{},\"notes\":[],\"questions\":[],\"runs\":[]}",
    )
  // Editing starts from the task's own fields.
  let #(model, _) = task.update(model, task.Title("changed"))
  let #(model, _) = task.update(model, task.Edit(True))
  #(model.editing, model.title, model.body, model.rank)
  |> should.equal(#(True, "T", "B", "1.0"))
  // A rank that is not a number is refused before anything is sent.
  let #(model, _) = task.update(model, task.Rank("high"))
  let #(model, _) = task.update(model, task.Save)
  model.error |> should.equal("Rank must be a number")
  // Adding a note shows the notes tab.
  let #(model, _) = task.update(model, task.AddNote)
  model.tab |> should.equal("notes")
}

/// Events and action replies carry no `changes` (D-77): the task keeps its
/// list, and a fetch of the task replaces it.
pub fn changes_survive_events_and_replies_test() {
  let bare =
    "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"triage\",\"rank\":1}"
  let full =
    "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"triage\",\"rank\":1,\"changes\":[{\"entity\":\"symbol:x\",\"name\":\"app.one\",\"project\":\"\"}]}"
  let model = task.Model(..loaded(full), hosted: True)
  let changes = fn(m: task.Model) {
    let assert Some(t) = m.task
    t.changes
  }
  let one = [task.Change("", "app", "one")]
  changes(model) |> should.equal(one)
  let #(model, _) = task.update(model, task.Live("{\"task\":" <> bare <> "}"))
  changes(model) |> should.equal(one)
  let #(model, _) = task.update(model, task.Changed(bare))
  changes(model) |> should.equal(one)
  let #(model, _) = task.update(model, task.Refetched(model.generation, bare))
  changes(model) |> should.equal([])
}

/// Only this task's events count: another task's, another project's, and a
/// stream of its own replaying what it missed change nothing.
pub fn live_takes_only_this_tasks_events_test() {
  let one = fn(id, title) {
    "{\"id\":\""
    <> id
    <> "\",\"title\":\""
    <> title
    <> "\",\"body\":\"\",\"column\":\"triage\",\"rank\":1}"
  }
  let model = task.Model(..loaded(one("1", "T")), dir: "a")
  let title = fn(m: task.Model) {
    let assert Some(t) = m.task
    t.title
  }
  // Its own stream: nothing applies until the replay is over.
  let #(model, _) =
    task.update(model, task.Live("{\"task\":" <> one("1", "Replayed") <> "}"))
  title(model) |> should.equal("T")
  let #(model, _) = task.update(model, task.CaughtUp)
  let #(model, _) =
    task.update(model, task.Live("{\"task\":" <> one("2", "Other") <> "}"))
  title(model) |> should.equal("T")
  let #(model, _) =
    task.update(
      model,
      task.Live("{\"project\":\"b\",\"task\":" <> one("1", "In b") <> "}"),
    )
  title(model) |> should.equal("T")
  let #(model, _) =
    task.update(
      model,
      task.Live("{\"project\":\"a\",\"task\":" <> one("1", "In a") <> "}"),
    )
  title(model) |> should.equal("In a")
  // Its project's workflow changes the actions offered.
  let #(model, _) =
    task.update(
      model,
      task.Live(
        "{\"project\":\"a\",\"workflow\":{\"columns\":[{\"id\":\"triage\",\"actions\":[\"plan\"]}]}}",
      ),
    )
  let assert Some(t) = model.task
  task.actions(model.workflow, t, False)
  |> should.equal(["plan", "pause", "discuss"])
}

/// A task whose home leaves its projects lives in the first one left
/// (D-61): the element asks for it there from now on.
pub fn a_task_that_leaves_its_home_follows_it_test() {
  let model =
    task.Model(
      ..loaded(
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"triage\",\"rank\":1,\"scope\":[\"a\",\"b\"]}",
      ),
      dir: "a",
      base: "/p/a",
    )
  let #(model, _) =
    task.update(
      model,
      task.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\"a\",\"name\":\"a\",\"base\":\"/p/a\"},{\"dir\":\"b\",\"label\":\"Bee\",\"name\":\"b\",\"base\":\"/p/b\"}]}",
      ),
    )
  model.projects
  |> should.equal([
    task.Project("a", "a", "/p/a"),
    task.Project("b", "Bee", "/p/b"),
  ])
  // Still in a: nothing moves.
  let #(model, _) =
    task.update(
      model,
      task.Changed(
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"triage\",\"rank\":1,\"scope\":[\"a\",\"b\"]}",
      ),
    )
  #(model.dir, model.base) |> should.equal(#("a", "/p/a"))
  let #(model, _) =
    task.update(
      model,
      task.Changed(
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"triage\",\"rank\":1,\"scope\":[\"b\"]}",
      ),
    )
  #(model.dir, model.base) |> should.equal(#("b", "/p/b"))
  // Removed from its project on its own stream: it is opened from the other.
  let model =
    task.Model(
      ..model,
      dir: "a",
      base: "/p/a",
      caught_up: True,
      task: option.map(model.task, fn(t) { task.Task(..t, scope: ["a", "b"]) }),
    )
  let #(model, _) =
    task.update(model, task.Live("{\"project\":\"a\",\"removed\":\"1\"}"))
  #(model.dir, model.base) |> should.equal(#("b", "/p/b"))
}

/// The Scope tab sits right of Changes, with a vertical switch per project;
/// Details doesn't hold the projects, and a running task locks them.
pub fn scope_tab_test() {
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
  let text =
    "{\"id\":\"t1\",\"title\":\"Across\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1,\"scope\":[\"a\",\"b\"]}"
  let model = task.Model(..loaded(text), dir: "a", workspace: True)
  let #(model, _) =
    task.update(
      model,
      task.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":["
        <> string.join(list.map(["a", "b", "c"], lane), ",")
        <> "]}",
      ),
    )
  let details = body(model)
  let assert [_, after_changes] = string.split(details, ">Changes (0)</button>")
  after_changes |> string.contains(">Scope</button>") |> should.be_true
  details
  |> string.contains("Projects this task can change")
  |> should.be_false
  // Outside a workspace there is no Scope tab.
  body(task.Model(..model, workspace: False))
  |> string.contains(">Scope</button>")
  |> should.be_false
  let #(model, _) = task.update(model, task.Tab("scope"))
  let scope = body(model)
  scope |> string.contains("Projects this task can change") |> should.be_true
  scope |> string.split("role=\"switch\"") |> list.length |> should.equal(4)
  // A change to the workspace's list fetches it again; a project added
  // meanwhile gets its switch.
  task.projects_changed("{\"projects\":[\"a\",\"b\",\"c\",\"d\"]}")
  |> should.be_true
  task.projects_changed("{\"project\":\"a\",\"workflow\":{\"columns\":[]}}")
  |> should.be_false
  let #(added, _) =
    task.update(
      model,
      task.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":["
        <> string.join(list.map(["a", "b", "c", "d"], lane), ",")
        <> "]}",
      ),
    )
  body(added)
  |> string.split("role=\"switch\"")
  |> list.length
  |> should.equal(5)
  scope |> string.contains("You can add projects") |> should.be_false
  let running =
    "{\"id\":\"t1\",\"title\":\"Across\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1,\"scope\":[\"a\",\"b\"],\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"running\",\"model\":\"m\",\"log\":\"/l\"}]}"
  let assert Ok(r) = json.parse(running, task.task_decoder())
  let working = body(task.Model(..model, task: Some(r)))
  working |> string.contains("You can add projects now") |> should.be_true
  // While the agent works, the chosen projects (a, b) are locked on; c can be added.
  let assert [_, switches] = string.split(working, "projects-pick")
  let assert [_, a, b, c] = string.split(switches, "<input")
  let locked = fn(rest) {
    let assert [input, ..] = string.split(rest, ">")
    string.contains(input, "disabled")
  }
  #(locked(a), locked(b), locked(c)) |> should.equal(#(True, True, False))
  // Switches stack vertically.
  let assert [css, ..] = string.split(html(model), "</style>")
  css
  |> string.contains(".projects-pick label{display:flex;")
  |> should.be_true
  css
  |> string.contains(".projects-pick input:checked::before{transform:")
  |> should.be_true
}

/// A notice's button names a tab and a task: an open task shows it now,
/// another once it loads.
pub fn reveal_shows_the_tab_when_the_task_is_open_test() {
  let text =
    "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1}"
  let model = loaded(text)
  let #(model, _) = task.update(model, task.Reveal("runs", "1"))
  #(model.tab, model.revealing) |> should.equal(#("runs", ""))
  let #(model, _) = task.update(model, task.Reveal("questions", "2"))
  #(model.tab, model.revealing) |> should.equal(#("questions", "2"))
  let #(model, _) = task.update(model, task.TaskChanged("2"))
  let #(model, _) =
    task.update(
      model,
      task.TaskLoaded(
        model.generation,
        string.replace(text, "\"id\":\"1\"", "\"id\":\"2\""),
      ),
    )
  #(model.tab, model.revealing) |> should.equal(#("questions", ""))
  // No notice yet: nothing changes.
  let #(same, _) = task.update(model, task.Reveal("", ""))
  same |> should.equal(model)
}

pub fn editor_toolbar_has_every_markdown_tool_test() {
  let rendered =
    task.editor(
      "body",
      "t1",
      "/p/a",
      "Description",
      "",
      fn(_) { Nil },
      fn(_, _) { Nil },
      fn(_) { Nil },
    )
    |> element.to_string
  [
    "title=\"Heading\"", "title=\"Bold\"", "title=\"Code\"",
    "title=\"Code block\"", "title=\"List\"", "title=\"Numbered list\"",
    "title=\"Quote\"", "title=\"Link\"", "title=\"Add an image\"",
    "data-base=\"/p/a\"",
  ]
  |> list.each(fn(tool) { rendered |> string.contains(tool) |> should.be_true })
}

pub fn notices_test() {
  let assert Ok(old) =
    json.parse(
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1,\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"running\",\"model\":\"m\",\"log\":\"/l\"}]}",
      task.task_decoder(),
    )
  let assert Ok(new) =
    json.parse(
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"needs_review\",\"rank\":1,\"stamps\":{\"approved\":{}},\"questions\":[{\"id\":\"q\",\"text\":\"Why?\",\"options\":[]}],\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"ended\",\"model\":\"m\",\"log\":\"/l\",\"outcome\":\"finished\"}]}",
      task.task_decoder(),
    )
  task.notices(old, new)
  |> should.equal([
    task.Notice("Finished T", "The agent finished its work.", "", "1", "runs"),
    task.Notice("Question on T", "Why?", "", "1", "questions"),
  ])
  task.notices(new, new) |> should.equal([])
  new.stamps |> should.equal(["approved"])
}

/// A notice's tab is the notes when the run left one, else the runs; a
/// question opens the questions.
pub fn notice_tabs_test() {
  let one = fn(runs, notes, questions) {
    let assert Ok(t) =
      json.parse(
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1,\"notes\":["
          <> notes
          <> "],\"questions\":["
          <> questions
          <> "],\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\""
          <> runs
          <> "\",\"model\":\"m\",\"log\":\"/l\",\"outcome\":\"finished\"}]}",
        task.task_decoder(),
      )
    t
  }
  let note =
    "{\"author\":\"agent\",\"run\":\"r1\",\"text\":\"Done\",\"time\":1}"
  let assert [with_note] =
    task.notices(one("running", "", ""), one("ended", note, ""))
  with_note.tab |> should.equal("notes")
  let assert [without] =
    task.notices(one("running", "", ""), one("ended", "", ""))
  without.tab |> should.equal("runs")
  let asked = "{\"id\":\"q\",\"text\":\"Why?\",\"options\":[]}"
  let assert [question] =
    task.notices(one("running", "", ""), one("running", "", asked))
  question.tab |> should.equal("questions")
}

/// A toast when the agent replies to a discussion; none when the discussion
/// ends or is stopped; a finished work run still toasts.
pub fn discussion_reply_notices_test() {
  let one = fn(kind, state, notes, outcome) {
    let assert Ok(t) =
      json.parse(
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1,\"notes\":["
          <> notes
          <> "],\"runs\":[{\"id\":\"r1\",\"kind\":\""
          <> kind
          <> "\",\"state\":\""
          <> state
          <> "\",\"model\":\"m\",\"log\":\"/l\",\"outcome\":\""
          <> outcome
          <> "\"}]}",
        task.task_decoder(),
      )
    t
  }
  let reply =
    "{\"author\":\"agent\",\"run\":\"r1\",\"text\":\"Here is my take\",\"time\":2}"
  let human =
    "{\"author\":\"human\",\"run\":null,\"text\":\"And also\",\"time\":3}"
  // The agent replies: the toast shows its note on the notes tab.
  let assert [toast] =
    task.notices(
      one("discuss", "running", "", ""),
      one("discuss", "waiting", reply, ""),
    )
  toast
  |> should.equal(task.Notice("Reply on T", "Here is my take", "", "1", "notes"))
  // A human note while the run stays waiting: no toast.
  task.notices(
    one("discuss", "waiting", reply, ""),
    one("discuss", "waiting", reply <> "," <> human, ""),
  )
  |> should.equal([])
  // The agent takes the human's turn: no toast.
  task.notices(
    one("discuss", "waiting", reply, ""),
    one("discuss", "running", reply, ""),
  )
  |> should.equal([])
  // Ending or stopping a discussion: no toast.
  task.notices(
    one("discuss", "waiting", reply, ""),
    one("discuss", "ended", reply, "discussed"),
  )
  |> should.equal([])
  task.notices(
    one("discuss", "waiting", reply, ""),
    one("discuss", "ended", reply, "stopped"),
  )
  |> should.equal([])
  // A work run that finishes still toasts.
  task.notices(
    one("work", "running", "", ""),
    one("work", "ended", "", "finished"),
  )
  |> should.equal([
    task.Notice("Finished T", "The agent finished its work.", "", "1", "runs"),
  ])
}

/// Outside a browser registering reports the error instead of crashing.
pub fn register_outside_browser_test() {
  task.register() |> should.be_error
}

/// Messages handed down by the host are applied in order, each once: a task
/// update and a workflow change that arrive together both count, and a list
/// that still holds applied ones applies only the new.
pub fn handed_messages_apply_in_order_once_test() {
  let one = fn(title) {
    "{\"task\":{\"id\":\"1\",\"title\":\""
    <> title
    <> "\",\"body\":\"\",\"column\":\"triage\",\"rank\":1}}"
  }
  let flow =
    "{\"workflow\":{\"columns\":[{\"id\":\"triage\",\"actions\":[\"plan\"]}]}}"
  let model = task.Model(..loaded(one("T")), hosted: True)
  let title = fn(m: task.Model) {
    let assert Some(t) = m.task
    t.title
  }
  // Out of order in the list; applied oldest first.
  let #(model, _) =
    task.update(
      model,
      task.Handed([#(3, one("Third")), #(1, one("First")), #(2, flow)]),
    )
  title(model) |> should.equal("Third")
  model.seen |> should.equal(3)
  let assert Some(t) = model.task
  task.actions(model.workflow, t, False)
  |> should.equal(["plan", "pause", "discuss"])
  // The same list again changes nothing; a longer one applies only the new.
  let #(same, _) =
    task.update(model, task.Handed([#(2, flow), #(3, one("Third"))]))
  same |> should.equal(model)
  let #(model, _) =
    task.update(model, task.Handed([#(3, one("Third")), #(4, one("Fourth"))]))
  title(model) |> should.equal("Fourth")
  model.seen |> should.equal(4)
}

/// A strategy proposal shows what it creates or changes, its fields with the
/// title first, the record Approve reopens by name, and Approve and Reject;
/// answered, it shows the answer instead.
pub fn proposal_display_test() {
  let assert Ok(p) =
    json.parse(
      "{\"kind\":\"task\",\"fields\":{\"title\":\"Index\",\"feature\":\"f1\",\"dependsOn\":[\"t1\",\"t2\"],\"milestone\":null},\"reopens\":[{\"kind\":\"feature\",\"id\":\"f1\"}]}",
      task.proposal_decoder(),
    )
  p.fields
  |> should.equal([
    #("title", "Index"),
    #("dependsOn", "t1, t2"),
    #("feature", "f1"),
    #("milestone", "(none)"),
  ])
  let name = fn(kind, id) {
    case kind, id {
      "feature", "f1" -> "Search"
      _, _ -> ""
    }
  }
  let open =
    task.proposal_view("", p, ["Approve", "Reject"], "", name, False, fn(o) {
      o
    })
    |> element.to_string
  open |> string.contains("Create task<em> Index</em>") |> should.be_true
  open
  |> string.contains(
    "Approving also sets the feature <em>Search</em> back to upcoming.",
  )
  |> should.be_true
  open |> string.contains(">Approve</button>") |> should.be_true
  open |> string.contains(">Reject</button>") |> should.be_true
  let answered =
    task.proposal_view(
      "",
      p,
      ["Approve", "Reject"],
      "Created task t9",
      name,
      False,
      fn(o) { o },
    )
    |> element.to_string
  answered |> string.contains("Answer: Created task t9") |> should.be_true
  answered |> string.contains(">Approve</button>") |> should.be_false
}

/// An edit names the record it changes, by its loaded title when the fields
/// don't hold one; the vision shows its new text; busy buttons are disabled.
pub fn proposal_edit_and_vision_test() {
  let assert Ok(edit) =
    json.parse(
      "{\"kind\":\"feature\",\"id\":\"f1\",\"fields\":{\"state\":\"active\"}}",
      task.proposal_decoder(),
    )
  let name = fn(_, _) { "Search" }
  let shown =
    task.proposal_view("", edit, ["Approve", "Reject"], "", name, True, fn(o) {
      o
    })
    |> element.to_string
  shown |> string.contains("Change feature<em> Search</em>") |> should.be_true
  shown |> string.contains("<dt>state</dt><dd>active</dd>") |> should.be_true
  shown |> string.contains("disabled") |> should.be_true
  let assert Ok(vision) =
    json.parse(
      "{\"kind\":\"vision\",\"fields\":{\"text\":\"For teams\"},\"base\":\"r1\"}",
      task.proposal_decoder(),
    )
  vision.base |> should.equal("r1")
  task.proposal_view("", vision, ["Approve", "Reject"], "", name, False, fn(o) {
    o
  })
  |> element.to_string
  |> string.contains("Replace the vision")
  |> should.be_true
}

/// A run that ends with an outcome of its own says so in the notice's heading.
pub fn ended_run_outcome_is_in_the_heading_test() {
  let one = fn(state) {
    let assert Ok(t) =
      json.parse(
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1,\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\""
          <> state
          <> "\",\"model\":\"m\",\"log\":\"/l\",\"outcome\":\"interrupted\"}]}",
        task.task_decoder(),
      )
    t
  }
  let assert [notice] = task.notices(one("running"), one("ended"))
  notice.heading |> should.equal("Stopped T: interrupted")
  notice.text |> should.equal("The work run ended: interrupted.")
}

/// D-95: a task with an `archivedTime` is archived; null or missing is not.
pub fn archived_time_decodes_to_archived_test() {
  let base =
    "\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"complete\",\"rank\":1"
  let archived = fn(extra) {
    let assert Ok(t) =
      json.parse("{" <> base <> extra <> "}", task.task_decoder())
    t.archived
  }
  archived(",\"archivedTime\":1700") |> should.be_true
  archived(",\"archivedTime\":null") |> should.be_false
  archived("") |> should.be_false
}

/// A task in a final column has no agent at work: a copy that still shows a
/// run going says what the task finished with, and offers only the column's
/// own actions (D-108): none in complete, the way back in canceled.
pub fn a_final_column_shows_no_live_agent_test() {
  let workflow =
    task.Workflow(
      [
        task.Column("assigned", "Assigned", ["complete"], False),
        task.Column("complete", "Complete", [], True),
        task.Column("cancelled", "canceled", ["to_backlog"], True),
      ],
      dict.new(),
    )
  let one = fn(column, finished) {
    let text =
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\""
      <> column
      <> "\",\"rank\":1,"
      <> finished
      <> "\"runs\":[{\"id\":\"r\",\"kind\":\"investigate\",\"model\":\"m\",\"state\":\"running\",\"started\":1,\"log\":\"\"}]}"
    let assert Ok(t) = json.parse(text, task.task_decoder())
    t
  }
  task.status_in(workflow, one("assigned", ""))
  |> should.equal("Agent investigating")
  task.status_in(workflow, one("complete", "")) |> should.equal("")
  task.status_in(
    workflow,
    one(
      "complete",
      "\"finished\":{\"kind\":\"investigate\",\"outcome\":\"reported\",\"time\":3},",
    ),
  )
  |> should.equal("Report ready")
  task.actions(workflow, one("complete", ""), False) |> should.equal([])
  task.actions(workflow, one("cancelled", ""), False)
  |> should.equal(["to_backlog"])
}

/// The case reported: a task stored in one project and covering another,
/// whose detail a host hands a running investigation and then the task
/// completed. It shows completed, with no agent at work.
pub fn a_hosted_detail_takes_the_completed_task_test() {
  let one = fn(column, state) {
    "{\"project\":\"solace-cli/board\",\"task\":{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\""
    <> column
    <> "\",\"rank\":1,\"scope\":[\"solace-cli/board\",\".\"],\"runs\":[{\"id\":\"r\",\"kind\":\"investigate\",\"model\":\"m\",\"state\":\""
    <> state
    <> "\",\"started\":1,\"log\":\"\"}]}}"
  }
  let workflow =
    task.Workflow(
      [
        task.Column("assigned", "Assigned", ["complete"], False),
        task.Column("complete", "Complete", [], True),
      ],
      dict.new(),
    )
  let model =
    task.Model(
      ..loaded(
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1}",
      ),
      hosted: True,
      dir: "solace-cli/board",
      workflow: workflow,
    )
  let #(model, _) =
    task.update(
      model,
      task.Handed([
        #(1, one("assigned", "running")),
        #(2, one("complete", "ended")),
      ]),
    )
  let assert Some(t) = model.task
  t.column |> should.equal("complete")
  task.status_in(model.workflow, t) |> should.equal("")
  task.status(t) |> should.equal("")
  // A refetch takes the whole task, runs included.
  let #(model, _) =
    task.update(
      task.Model(..model, task: Some(task.Task(..t, column: "assigned"))),
      task.Refetched(
        model.generation,
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"complete\",\"rank\":1,\"runs\":[]}",
      ),
    )
  let assert Some(t) = model.task
  t.column |> should.equal("complete")
  t.runs |> should.equal([])
}

/// The host's `resync`, and this task's update named under another project,
/// fetch the task again instead of being applied or dropped; the copy shown
/// stays until the fetch answers.
pub fn a_doubtful_message_fetches_the_task_again_test() {
  let one = fn(project, id) {
    "{\"project\":\""
    <> project
    <> "\",\"task\":{\"id\":\""
    <> id
    <> "\",\"title\":\"Other\",\"body\":\"\",\"column\":\"triage\",\"rank\":1}}"
  }
  let model =
    task.Model(
      ..loaded(
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"triage\",\"rank\":1}",
      ),
      hosted: True,
      dir: "a",
    )
  task.stale(model, task.resync) |> should.be_true
  task.stale(model, one(".", "1")) |> should.be_true
  task.stale(model, one("a", "1")) |> should.be_false
  task.stale(model, one(".", "2")) |> should.be_false
  task.stale(model, "{\"project\":\".\",\"workflow\":{\"columns\":[]}}")
  |> should.be_false
  let #(after, _) = task.update(model, task.Live(task.resync))
  after |> should.equal(model)
  let #(after, _) = task.update(model, task.Live(one(".", "1")))
  after |> should.equal(model)
}

/// Copy ID beside the title copies the task's reference, `task:<id>`, and
/// says whether it copied until its moment passes or another task opens
/// (D-98). A reply or a timer for an earlier click changes nothing.
pub fn copy_id_beside_the_title_test() {
  let model =
    loaded(
      "{\"id\":\"t1\",\"title\":\"Fix\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1}",
    )
  task.reference("t1") |> should.equal("task:t1")
  let shown = body(model)
  let assert [_, after_title] = string.split(shown, ">Fix</h2>")
  after_title |> string.contains(">Copy ID</button>") |> should.be_true
  shown |> string.contains("Copies task:t1") |> should.be_true
  let #(asked, _) = task.update(model, task.CopyId)
  body(asked) |> string.contains(">Copy ID</button>") |> should.be_true
  let #(copied, _) = task.update(asked, task.CopyDone(1, True))
  body(copied) |> string.contains(">Copied</button>") |> should.be_true
  let #(later, _) = task.update(copied, task.CopyReset(1))
  body(later) |> string.contains(">Copy ID</button>") |> should.be_true
  // A copy that failed says so.
  let #(failed, _) = task.update(asked, task.CopyDone(1, False))
  body(failed) |> string.contains(">Copy failed</button>") |> should.be_true
  // A second click: the first click's timer doesn't clear its label early.
  let #(again, _) = task.update(copied, task.CopyId)
  let #(again, _) = task.update(again, task.CopyDone(2, True))
  let #(again, _) = task.update(again, task.CopyReset(1))
  body(again) |> string.contains(">Copied</button>") |> should.be_true
  // Another task opens: a late reply for this one shows nothing.
  let #(other, _) = task.update(asked, task.TaskChanged("t2"))
  let #(other, _) = task.update(other, task.CopyDone(1, True))
  other.copy.1 |> should.equal("")
}

/// D-87: a task's `dependsOn`, read as `[]` for a task saved before it.
pub fn depends_on_decodes_test() {
  let assert Ok(t) =
    json.parse(
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1,\"dependsOn\":[\"a\",\"b\"]}",
      task.task_decoder(),
    )
  t.depends |> should.equal(["a", "b"])
  let assert Ok(old) =
    json.parse(
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1}",
      task.task_decoder(),
    )
  old.depends |> should.equal([])
}

/// A workflow with an open column and two final ones, as the default's.
const depends_flow =
  "{\"columns\":[{\"id\":\"backlog\",\"label\":\"Backlog\",\"actions\":[\"plan\"]},{\"id\":\"complete\",\"label\":\"Complete\",\"final\":true},{\"id\":\"cancelled\",\"label\":\"Cancelled\",\"final\":true}]}"

/// Task `id` titled `title` in `column`, depending on `depends`.
fn depends_task(
  id: String,
  title: String,
  column: String,
  depends: String,
) -> String {
  "{\"id\":\""
  <> id
  <> "\",\"title\":\""
  <> title
  <> "\",\"body\":\"\",\"column\":\""
  <> column
  <> "\",\"rank\":1,\"dependsOn\":["
  <> depends
  <> "]}"
}

/// The Details tab lists each dependency as a link with its title and
/// column; an unfinished one is marked, a cancelled or complete one isn't,
/// and one that can't be found shows its id. No dependencies, no line.
pub fn depends_on_line_test() {
  let none = loaded(depends_task("1", "T", "backlog", ""))
  assert !string.contains(html(none), "Depends on")
  let model =
    loaded(depends_task("1", "T", "backlog", "\"d1\",\"d2\",\"d3\",\"d4\""))
  let g = model.generation
  let #(model, _) = task.update(model, task.WorkflowLoaded(g, depends_flow))
  // Every dependency was asked for once it loaded.
  assert list.map(model.deps, fn(d) { d.0 }) |> list.sort(string.compare)
    == ["d1", "d2", "d3", "d4"]
  let #(model, _) =
    task.update(
      model,
      task.DepLoaded(g, "d1", depends_task("d1", "First", "backlog", "")),
    )
  let #(model, _) =
    task.update(
      model,
      task.DepLoaded(g, "d2", depends_task("d2", "Second", "cancelled", "")),
    )
  let #(model, _) =
    task.update(
      model,
      task.DepLoaded(g, "d3", depends_task("d3", "Third", "complete", "")),
    )
  let #(model, _) =
    task.update(
      model,
      task.DepFound(
        g,
        "d4",
        "{\"code\":\"E7023\",\"message\":\"no such task\"}",
      ),
    )
  let page = html(model)
  assert string.contains(page, "Depends on")
  assert string.contains(page, "href=\"/task/d1\"")
  assert string.contains(page, "data-task=\"d1\"")
  assert string.contains(page, ">First</a> (Backlog, unfinished)")
  assert string.contains(page, ">Second</a> (Cancelled)")
  assert string.contains(page, ">Third</a> (Complete)")
  assert string.contains(page, ">d4</a>")
  assert list.length(string.split(page, ", unfinished)")) == 2
  assert list.length(string.split(page, "class=\"dep unfinished\"")) == 2
}

/// A dependency's update under another project changes its line; another
/// task's doesn't count. A change to `dependsOn` asks for the new one, and a
/// reply for a task shown before is dropped.
pub fn depends_on_line_updates_live_test() {
  let model =
    task.Model(
      ..loaded(depends_task("1", "T", "backlog", "\"d1\"")),
      dir: "a",
      hosted: True,
    )
  let g = model.generation
  let #(model, _) = task.update(model, task.WorkflowLoaded(g, depends_flow))
  let #(model, _) =
    task.update(
      model,
      task.DepLoaded(g, "d1", depends_task("d1", "First", "backlog", "")),
    )
  assert string.contains(html(model), "(Backlog, unfinished)")
  let #(model, _) =
    task.update(
      model,
      task.Handed([
        #(
          1,
          "{\"project\":\"b\",\"task\":"
            <> depends_task("d1", "Renamed", "complete", "")
            <> "}",
        ),
        #(
          2,
          "{\"project\":\"b\",\"task\":"
            <> depends_task("d9", "Other", "backlog", "")
            <> "}",
        ),
      ]),
    )
  let page = html(model)
  assert string.contains(page, ">Renamed</a> (Complete)")
  assert !string.contains(page, ", unfinished)")
  assert !string.contains(page, "class=\"dep unfinished\"")
  assert !string.contains(page, "Other")
  // Back out of a final column, it is unfinished again.
  let #(model, _) =
    task.update(
      model,
      task.Live(
        "{\"project\":\"b\",\"task\":"
        <> depends_task("d1", "Renamed", "backlog", "")
        <> "}",
      ),
    )
  assert string.contains(html(model), ">Renamed</a> (Backlog, unfinished)")
  // `dependsOn` gains d2: it is asked for, and shows its id meanwhile.
  let #(model, _) =
    task.update(
      model,
      task.Handed([
        #(
          3,
          "{\"project\":\"a\",\"task\":"
            <> depends_task("1", "T", "backlog", "\"d1\",\"d2\"")
            <> "}",
        ),
      ]),
    )
  assert list.key_find(model.deps, "d2") == Ok(None)
  assert string.contains(html(model), ">d2</a>")
  // A reply for the task shown before changes nothing.
  let #(same, _) =
    task.update(
      model,
      task.DepLoaded(g - 1, "d2", depends_task("d2", "Old", "backlog", "")),
    )
  assert same == model
}

/// D-112: a task with `techDebt: true` is a feature's Tech Debt task; a
/// missing field is not.
pub fn tech_debt_decodes_test() {
  let base =
    "\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1"
  let debt = fn(extra) {
    let assert Ok(t) =
      json.parse("{" <> base <> extra <> "}", task.task_decoder())
    t.tech_debt
  }
  debt(",\"techDebt\":true") |> should.be_true
  debt("") |> should.be_false
}

/// Features take colors by their slots, 5 places apart along the 16-color
/// rainbow; all 16 come before one repeats.
pub fn feature_colors_skip_four_in_rainbow_order_test() {
  list.map([0, 1, 2, 3, 4], task.feature_color)
  |> should.equal(["#d7263d", "#5f9a1a", "#2f72d8", "#d93a7f", "#9a9200"])
  int.range(from: 0, to: 16, with: [], run: fn(acc, i) { [i, ..acc] })
  |> list.map(task.feature_color)
  |> list.unique
  |> list.length
  |> should.equal(16)
  task.feature_color(16) |> should.equal(task.feature_color(0))
  // A feature not started has no color; a gap left by a deleted one stays.
  task.feature_colors([
    #("late", Some(17)),
    #("idle", None),
    #("b", Some(2)),
    #("a", Some(0)),
  ])
  |> should.equal([
    #("late", "#5f9a1a"),
    #("b", "#2f72d8"),
    #("a", "#d7263d"),
  ])
}

pub fn a_task_names_its_feature_test() {
  let base =
    "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"triage\",\"rank\":1"
  let assert Ok(t) =
    json.parse(base <> ",\"feature\":\"f1\"}", task.task_decoder())
  t.feature |> should.equal(Some("f1"))
  let assert Ok(t) =
    json.parse(base <> ",\"feature\":null}", task.task_decoder())
  t.feature |> should.equal(None)
  let assert Ok(t) = json.parse(base <> "}", task.task_decoder())
  t.feature |> should.equal(None)
}

/// The detail's styles set no colors of their own: a nested shadow root
/// inherits them from its host, so the board's dark theme reaches it.
pub fn styles_inherit_the_hosts_colors_test() {
  let assert [css, ..] = string.split(html(fresh("1")), "</style>")
  css |> string.contains("--paper:") |> should.be_false
  css |> string.contains("--ink:") |> should.be_false
  css |> string.contains(":host{display:block}") |> should.be_true
}

/// D-115: an edit proposal carries the record's current values as `before`;
/// a field whose value doesn't change isn't shown.
pub fn proposal_unchanged_fields_are_hidden_test() {
  let assert Ok(p) =
    json.parse(
      "{\"kind\":\"feature\",\"id\":\"f1\",\"fields\":{\"title\":\"Search\",\"state\":\"active\"},\"before\":{\"title\":\"Search\",\"state\":\"planned\"}}",
      task.proposal_decoder(),
    )
  let shown =
    task.proposal_view(
      "",
      p,
      ["Approve", "Reject"],
      "",
      fn(_, _) { "Search" },
      False,
      fn(o) { o },
    )
    |> element.to_string
  shown |> string.contains("<dt>title</dt>") |> should.be_false
  shown
  |> string.contains("<dt>state</dt><dd>planned → active</dd>")
  |> should.be_true
}

/// A changed short field shows `old → new`, a record id by its title.
pub fn proposal_short_field_shows_old_to_new_test() {
  let assert Ok(p) =
    json.parse(
      "{\"kind\":\"task\",\"id\":\"t1\",\"fields\":{\"feature\":\"f1\",\"dependsOn\":[\"t2\"]},\"before\":{\"feature\":\"f0\",\"dependsOn\":[]}}",
      task.proposal_decoder(),
    )
  let name = fn(kind, id) {
    case kind, id {
      "feature", "f1" -> "Search"
      "feature", "f0" -> "Old"
      "task", "t2" -> "Index"
      _, _ -> ""
    }
  }
  let shown =
    task.proposal_view("", p, ["Approve", "Reject"], "", name, False, fn(o) {
      o
    })
    |> element.to_string
  shown
  |> string.contains("<dt>feature</dt><dd>Old → Search</dd>")
  |> should.be_true
  shown
  |> string.contains("<dt>dependsOn</dt><dd>(none) → Index</dd>")
  |> should.be_true
}

/// The body shows as a line diff: lines the two bodies share collapse to one
/// muted line, a removed line is struck, an added one marked, in the body's
/// order, and a toggle shows the new body as Markdown; the vision's new text
/// diffs the same way.
pub fn proposal_body_shows_as_a_diff_test() {
  let assert Ok(p) =
    json.parse(
      "{\"kind\":\"task\",\"id\":\"t1\",\"fields\":{\"body\":\"Keep me\\n\\nAdd me\"},\"before\":{\"body\":\"Keep me\\n\\nDrop me\"}}",
      task.proposal_decoder(),
    )
  let shown =
    task.proposal_view(
      "",
      p,
      ["Approve", "Reject"],
      "",
      fn(_, _) { "Index" },
      False,
      fn(o) { o },
    )
    |> element.to_string
  shown |> string.contains("1 line unchanged") |> should.be_true
  shown |> string.contains("<del>- Drop me</del>") |> should.be_true
  shown |> string.contains("+ Add me") |> should.be_true
  shown |> string.contains("<summary>New body</summary>") |> should.be_true
  let assert Ok(vision) =
    json.parse(
      "{\"kind\":\"vision\",\"fields\":{\"text\":\"Keep me\\n\\nAdd me\"},\"before\":{\"text\":\"Keep me\\n\\nDrop me\"}}",
      task.proposal_decoder(),
    )
  task.proposal_view(
    "",
    vision,
    ["Approve", "Reject"],
    "",
    fn(_, _) { "" },
    False,
    fn(o) { o },
  )
  |> element.to_string
  |> string.contains("<del>- Drop me</del>")
  |> should.be_true
}

/// An edit whose fields all match the record's current values says "No
/// change" and shows none of them.
pub fn proposal_that_changes_nothing_says_so_test() {
  let assert Ok(p) =
    json.parse(
      "{\"kind\":\"task\",\"id\":\"t1\",\"fields\":{\"title\":\"Index\"},\"before\":{\"title\":\"Index\"}}",
      task.proposal_decoder(),
    )
  let shown =
    task.proposal_view(
      "",
      p,
      ["Approve", "Reject"],
      "",
      fn(_, _) { "Index" },
      False,
      fn(o) { o },
    )
    |> element.to_string
  shown |> string.contains("No change") |> should.be_true
  shown |> string.contains("<dt>title</dt>") |> should.be_false
}

/// The line diff keeps the body's order: a line edited in the middle shows
/// where it is, between the muted lines before it and after it.
pub fn proposal_body_shows_a_change_in_its_place_test() {
  let assert Ok(p) =
    json.parse(
      "{\"kind\":\"task\",\"id\":\"t1\",\"fields\":{\"body\":\"One\\n\\nTwo edited\\n\\nThree\"},\"before\":{\"body\":\"One\\n\\nTwo\\n\\nThree\"}}",
      task.proposal_decoder(),
    )
  let shown =
    task.proposal_view(
      "",
      p,
      ["Approve", "Reject"],
      "",
      fn(_, _) { "Index" },
      False,
      fn(o) { o },
    )
    |> element.to_string
  // The struck line comes after the muted line for the lines before it, the
  // added line after the struck one, and the muted line for the lines after
  // it after the added one.
  case string.split(shown, "<del>- Two</del>") {
    [before, after] -> {
      before |> string.contains("1 line unchanged") |> should.be_true
      case string.split(after, "+ Two edited") {
        [_, tail] ->
          tail |> string.contains("1 line unchanged") |> should.be_true
        _ -> should.be_true(False)
      }
    }
    _ -> should.be_true(False)
  }
}

/// A vision proposal whose text is the vision's says "No change", as an edit
/// whose fields all match does.
pub fn a_vision_proposal_that_changes_nothing_says_so_test() {
  let assert Ok(v) =
    json.parse(
      "{\"kind\":\"vision\",\"fields\":{\"text\":\"For teams\"},\"before\":{\"text\":\"For teams\"}}",
      task.proposal_decoder(),
    )
  let shown =
    task.proposal_view(
      "",
      v,
      ["Approve", "Reject"],
      "",
      fn(_, _) { "" },
      False,
      fn(o) { o },
    )
    |> element.to_string
  shown |> string.contains("No change") |> should.be_true
  shown |> string.contains("lines unchanged") |> should.be_false
}

/// A body that differs only in the space around its lines, or in blank lines,
/// is not a change; a changed line beside respaced ones shows only that line.
pub fn proposal_body_that_only_changes_spacing_says_no_change_test() {
  let show = fn(json_text) {
    let assert Ok(p) = json.parse(json_text, task.proposal_decoder())
    task.proposal_view(
      "",
      p,
      ["Approve", "Reject"],
      "",
      fn(_, _) { "Index" },
      False,
      fn(o) { o },
    )
    |> element.to_string
  }
  let same =
    show(
      "{\"kind\":\"task\",\"id\":\"t1\",\"fields\":{\"body\":\"One\\n\\nTwo\"},\"before\":{\"body\":\"One \\n  Two\"}}",
    )
  same |> string.contains("No change") |> should.be_true
  same |> string.contains("<dt>body</dt>") |> should.be_false
  let one =
    show(
      "{\"kind\":\"task\",\"id\":\"t1\",\"fields\":{\"body\":\"One\\nTwo\\nThree\"},\"before\":{\"body\":\"One \\n  Two\\nFour\"}}",
    )
  one |> string.contains("2 lines unchanged") |> should.be_true
  one |> string.contains("<del>- Four</del>") |> should.be_true
  one |> string.contains("+ Three") |> should.be_true
}

/// A vision proposal saved without `before` shows its text as Markdown under
/// "Replace the vision", not as a field list.
pub fn a_vision_proposal_saved_before_before_shows_markdown_test() {
  let assert Ok(vision) =
    json.parse(
      "{\"kind\":\"vision\",\"fields\":{\"text\":\"# For teams\"},\"base\":\"r1\"}",
      task.proposal_decoder(),
    )
  let shown =
    task.proposal_view(
      "",
      vision,
      ["Approve", "Reject"],
      "",
      fn(_, _) { "" },
      False,
      fn(o) { o },
    )
    |> element.to_string
  shown |> string.contains("Replace the vision") |> should.be_true
  shown |> string.contains("<div class=\"md\">") |> should.be_true
  shown |> string.contains("<dt>text</dt>") |> should.be_false
}

/// The default workflow's action buttons show their labels, not the action
/// names (D-108), and the fixed buttons read `ui_labels`. Back to backlog
/// shows from Planning only with the `strategy` attribute (D-132). Back to
/// planning shows from Needs review and Canceled, feature or not (D-133).
pub fn default_workflow_buttons_show_their_labels_test() {
  let model = fresh("1")
  let #(model, _) =
    task.update(
      model,
      task.WorkflowLoaded(
        model.generation,
        "{\"columns\":[{\"id\":\"backlog\",\"label\":\"Backlog\",\"actions\":[\"plan\",\"cancel\"]},{\"id\":\"planning\",\"label\":\"Planning\",\"actions\":[\"investigate\",\"assign\",\"complete\",\"to_backlog\",\"cancel\"]},{\"id\":\"assigned\",\"label\":\"Assigned\",\"actions\":[\"reassign\",\"new_agent\",\"review\",\"to_planning\",\"cancel\"]},{\"id\":\"needs_review\",\"label\":\"Needs review\",\"actions\":[\"approve\",\"complete\",\"review\",\"reassign\",\"start_over\",\"to_planning\",\"cancel\"]},{\"id\":\"complete\",\"label\":\"Complete\",\"final\":true},{\"id\":\"cancelled\",\"label\":\"Canceled\",\"final\":true,\"actions\":[\"to_planning\"]}],\"actions\":{\"plan\":{\"to\":\"planning\",\"label\":\"Plan\"},\"to_backlog\":{\"to\":\"backlog\",\"label\":\"Back to backlog\"},\"to_planning\":{\"unstamp\":\"approved\",\"to\":\"planning\",\"label\":\"Back to planning\"},\"investigate\":{\"run\":\"investigate\",\"model\":true,\"label\":\"Investigate\"},\"review\":{\"run\":\"review\",\"label\":\"Review\"},\"assign\":{\"to\":\"assigned\",\"run\":\"work\",\"model\":true,\"label\":\"Assign\"},\"new_agent\":{\"to\":\"assigned\",\"run\":\"new_work\",\"label\":\"New agent\"},\"approve\":{\"stamp\":\"approved\",\"label\":\"Approve\"},\"complete\":{\"to\":\"complete\",\"label\":\"Complete\"},\"reassign\":{\"unstamp\":\"approved\",\"to\":\"assigned\",\"run\":\"work\",\"label\":\"Reassign\"},\"start_over\":{\"revert\":true,\"unstamp\":\"approved\",\"to\":\"assigned\",\"run\":\"new_work\",\"label\":\"Start over\"},\"cancel\":{\"revert\":true,\"to\":\"cancelled\",\"label\":\"Cancel\"}}}",
      ),
    )
  let #(model, _) =
    task.update(
      model,
      task.TaskLoaded(
        model.generation,
        "{\"id\":\"1\",\"title\":\"Fix checkout\",\"body\":\"\",\"column\":\"planning\",\"rank\":1,\"notes\":[],\"questions\":[],\"runs\":[]}",
      ),
    )
  let rendered = html(model)
  rendered |> string.contains(">Investigate</button>") |> should.be_true
  rendered |> string.contains(">Assign</button>") |> should.be_true
  rendered |> string.contains(">Complete</button>") |> should.be_true
  rendered |> string.contains(">Back to backlog</button>") |> should.be_false
  let #(on_strategy, _) = task.update(model, task.StrategyChanged(True))
  html(on_strategy)
  |> string.contains(">Back to backlog</button>")
  |> should.be_true
  rendered |> string.contains(">Cancel</button>") |> should.be_true
  rendered |> string.contains(">assign</button>") |> should.be_false
  rendered |> string.contains(">Add note</button>") |> should.be_true
  rendered |> string.contains(">Copy ID</button>") |> should.be_true
  rendered |> string.contains(">Back to planning</button>") |> should.be_false
  let in_column = fn(column, feature) {
    task.update(
      model,
      task.TaskLoaded(
        model.generation,
        "{\"id\":\"1\",\"title\":\"Fix checkout\",\"body\":\"\",\"column\":\""
          <> column
          <> "\",\"rank\":1,\"feature\":"
          <> feature
          <> ",\"notes\":[],\"questions\":[],\"runs\":[]}",
      ),
    ).0
    |> html
  }
  in_column("needs_review", "null")
  |> string.contains(">Back to planning</button>")
  |> should.be_true
  let cancelled = in_column("cancelled", "\"f1\"")
  cancelled |> string.contains(">Back to planning</button>") |> should.be_true
  cancelled |> string.contains(">Back to backlog</button>") |> should.be_false
  in_column("complete", "null")
  |> string.contains(">Back to planning</button>")
  |> should.be_false
}

/// The task detail's fixed buttons, the model picker and the scope legend
/// say the shared names from `ui_labels`, so one rename in the map moves them.
pub fn shared_labels_on_the_task_test() {
  let text =
    "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"planning\",\"rank\":1,\"notes\":[],\"questions\":[],\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"running\",\"model\":\"m\",\"log\":\"/l\",\"started\":1}],\"changes\":[{\"name\":\"app.alpha\",\"entity\":\"symbol:app.alpha\",\"project\":\"a\"}]}"
  let base =
    task.Model(
      ..loaded(text),
      dir: "a",
      workspace: True,
      workflow: task.Workflow(
        [task.Column("planning", "Needs review", [], False)],
        dict.new(),
      ),
    )
  let #(model, _) =
    task.update(
      base,
      task.ProjectsLoaded(
        "{\"workspace\":true,\"projects\":[{\"dir\":\"a\",\"label\":\"a\",\"name\":\"a\",\"base\":\"/p/a\"}]}",
      ),
    )
  let #(model, _) =
    task.update(
      model,
      task.ModelsLoaded("[{\"id\":\"m\",\"label\":\"M\",\"available\":true}]"),
    )
  // The task's column shows the workflow's label, not its id.
  body(base)
  |> string.contains("Needs review")
  |> should.be_true
  body(base)
  |> string.contains("needs_review")
  |> should.be_false
  // The model picker names itself to readers, and says what to pick.
  body(model)
  |> string.contains("aria-label=\"" <> task.ui_label("model") <> "\"")
  |> should.be_true
  // Editing Details saves with Save and closes with Cancel.
  let #(editing, _) = task.update(model, task.Edit(True))
  let page = body(editing)
  page
  |> string.contains(">" <> task.ui_label("save") <> "</button>")
  |> should.be_true
  page
  |> string.contains(">" <> task.ui_label("cancel") <> "</button>")
  |> should.be_true
  // The runs tab watches and hides with the shared pair.
  let #(runs, _) = task.update(model, task.Tab("runs"))
  body(runs)
  |> string.contains(">" <> task.ui_label("watch") <> "</button>")
  |> should.be_true
  let #(watched, _) = task.update(runs, task.Watch("r1"))
  body(watched)
  |> string.contains(">" <> task.ui_label("hide_activity") <> "</button>")
  |> should.be_true
  // The scope legend names the projects the task can change.
  let #(scope, _) = task.update(model, task.Tab("scope"))
  body(scope)
  |> string.contains(task.ui_label("scope_projects"))
  |> should.be_true
  // The changes tab links to the contract page by its shared name.
  let #(changes, _) = task.update(model, task.Tab("changes"))
  body(changes)
  |> string.contains(">" <> task.ui_label("contract_view") <> "</a>")
  |> should.be_true
}

/// Text areas and inputs fill their container: `width:100%` with
/// `box-sizing:border-box`, no percentage cap; the composer box and the
/// title have no `max-width` of their own.
pub fn textareas_fill_their_container_test() {
  let assert [css, ..] = string.split(html(fresh("1")), "</style>")
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
  |> string.contains(
    ".composer{border:1px solid var(--edge);border-radius:0;margin:8px 4px;background:var(--paper)}",
  )
  |> should.be_true
  css
  |> string.contains(
    ".title{display:block;box-sizing:border-box;width:calc(100% - 8px);margin:8px 4px}",
  )
  |> should.be_true
}

/// The controls' styles name no color of their own, only the theme's
/// tokens, and give the option list of a select the page's colors.
pub fn control_styles_use_only_tokens_test() {
  let styles = task.control_styles
  assert !string.contains(styles, "#")
  assert !string.contains(styles, "rgb")
  assert !string.contains(styles, "hsl")
  assert string.contains(
    styles,
    "option,optgroup{background:var(--paper);color:var(--ink)}",
  )
  assert string.contains(styles, "select:hover")
}

/// An unknown label key never reaches a button: it says nothing.
pub fn unknown_label_says_nothing_test() {
  task.ui_label("no_such_button") |> should.equal("")
  task.ui_label("copy_id") |> should.equal("Copy ID")
}

/// D-129: a task whose home is no project decodes `noProject`, and its
/// detail says "no project"; a task with a project doesn't.
pub fn a_task_with_no_project_shows_a_marker_test() {
  let base =
    "\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"backlog\",\"rank\":1"
  let homeless = fn(extra) {
    let assert Ok(t) =
      json.parse("{" <> base <> extra <> "}", task.task_decoder())
    t.no_project
  }
  homeless(",\"noProject\":true") |> should.be_true
  homeless("") |> should.be_false
  let shown = body(loaded("{" <> base <> ",\"noProject\":true}"))
  shown |> string.contains("no project") |> should.be_true
  let plain = body(loaded("{" <> base <> "}"))
  plain |> string.contains("no project") |> should.be_false
}

/// The editor is one `<solace-md-editor>` that styles the markdown as it is
/// written: no Write or Preview tab. It is named, labelled and placeholdered
/// as the textarea was, and carries the task and base for images.
pub fn editor_styles_markdown_as_it_is_written_test() {
  let rendered =
    task.editor(
      "body",
      "t1",
      "/p/a",
      "Description",
      "**hi**",
      fn(_) { Nil },
      fn(_, _) { Nil },
      fn(_) { Nil },
    )
    |> element.to_string
  [
    "<solace-md-editor", "id=\"body\"", "class=\"md-editor\"",
    "data-task=\"t1\"", "data-base=\"/p/a\"", "label=\"Description\"",
    "placeholder=\"Description (markdown). Paste or drop images here.\"",
  ]
  |> list.each(fn(part) { rendered |> string.contains(part) |> should.be_true })
  ["<textarea", ">Write<", ">Preview<", "class=\"preview\""]
  |> list.each(fn(part) { rendered |> string.contains(part) |> should.be_false })
  // A task not yet created has no image tool, and its placeholder no images.
  let draft =
    task.editor(
      "body",
      "",
      "",
      "Vision",
      "",
      fn(_) { Nil },
      fn(_, _) { Nil },
      fn(_) { Nil },
    )
    |> element.to_string
  draft
  |> string.contains("placeholder=\"Vision (markdown).\"")
  |> should.be_true
  draft |> string.contains("title=\"Add an image\"") |> should.be_false
}

/// D-129: a task with no project is not followed to a project of its scope,
/// and its own updates, named under its home folder, are not in doubt.
pub fn a_task_with_no_project_stays_where_it_is_opened_test() {
  let model =
    task.Model(
      ..loaded(
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"triage\",\"rank\":1,\"scope\":[\"a\"],\"noProject\":true}",
      ),
      dir: ".",
      base: "",
    )
  let #(model, _) =
    task.update(
      model,
      task.Changed(
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"triage\",\"rank\":1,\"scope\":[\"b\"],\"noProject\":true}",
      ),
    )
  #(model.dir, model.base) |> should.equal(#(".", ""))
  assert !task.stale(
    model,
    "{\"project\":\"web\",\"noProject\":true,\"task\":{\"id\":\"1\"}}",
  )
  assert task.stale(model, "{\"project\":\"web\",\"task\":{\"id\":\"1\"}}")
}

/// D-132: Plan shows for a backlog task anywhere. Back to backlog shows from
/// Planning only on the Strategy page, from Canceled on any page, and never
/// for a task with a feature.
pub fn back_to_backlog_needs_the_strategy_page_and_no_feature_test() {
  let workflow =
    task.Workflow(
      [
        task.Column("backlog", "Backlog", ["plan", "cancel"], False),
        task.Column("planning", "Planning", ["assign", "to_backlog"], False),
        task.Column("cancelled", "Canceled", ["to_backlog"], True),
      ],
      dict.new(),
    )
  let one = fn(column, feature) {
    let text =
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\""
      <> column
      <> "\",\"rank\":1,\"feature\":"
      <> feature
      <> ",\"runs\":[]}"
    let assert Ok(t) = json.parse(text, task.task_decoder())
    t
  }
  let offers = fn(t, strategy, name) {
    list.contains(task.actions(workflow, t, strategy), name)
  }
  offers(one("backlog", "null"), False, "plan") |> should.be_true
  offers(one("backlog", "\"f\""), True, "plan") |> should.be_true
  offers(one("planning", "null"), True, "to_backlog") |> should.be_true
  offers(one("planning", "null"), False, "to_backlog") |> should.be_false
  offers(one("planning", "\"f\""), True, "to_backlog") |> should.be_false
  offers(one("cancelled", "null"), False, "to_backlog") |> should.be_true
  offers(one("cancelled", "\"f\""), True, "to_backlog") |> should.be_false
}

/// A dependency is asked for at its project's base; one with no project
/// (D-129) at the root's base as `GET /projects` lists it; one not found,
/// nowhere.
pub fn a_dependency_is_asked_for_where_it_is_stored_test() {
  let model =
    task.Model(..loaded(depends_task("1", "T", "backlog", "")), projects: [
      task.Project(".", "root", "/hosted"),
      task.Project("shop", "shop", "/p/shop"),
    ])
  assert task.dep_base(model, "{\"dir\":\"shop\",\"project\":true}")
    == Ok("/p/shop")
  assert task.dep_base(model, "{\"dir\":\"shop\",\"project\":false}")
    == Ok("/hosted")
  assert task.dep_base(model, "{\"dir\":\"web\"}") == Ok("/p/web")
  // A root that isn't listed is asked at "".
  assert task.dep_base(
      task.Model(..model, projects: []),
      "{\"dir\":\"shop\",\"project\":false}",
    )
    == Ok("")
  assert task.dep_base(
      model,
      "{\"code\":\"E7023\",\"message\":\"no such task\"}",
    )
    == Error(Nil)
}

pub fn usage_limits_can_be_tried_again_test() {
  let model = fresh("1")
  let #(model, _) =
    task.update(
      model,
      task.ModelsLoaded(
        "[{\"id\":\"sol\",\"label\":\"Sol\",\"agent\":\"codex\",\"available\":false,\"reason\":\"Codex usage limit reached (resets 3:45 PM)\",\"limit\":true},{\"id\":\"luna\",\"label\":\"Luna\",\"agent\":\"codex\",\"available\":false,\"reason\":\"codex is not installed\",\"limit\":false}]",
      ),
    )
  let #(model, _) =
    task.update(
      model,
      task.TaskLoaded(
        model.generation,
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"planning\",\"rank\":1}",
      ),
    )
  let rendered = html(model)
  // No model can start: what to set up, instead of an empty pick.
  rendered |> string.contains("No model can start yet") |> should.be_true
  rendered |> string.contains("claude.ai/install.sh") |> should.be_true
  rendered |> string.contains("<select") |> should.be_false
  rendered
  |> string.contains("<summary>2 models unavailable</summary>")
  |> should.be_true
  // Only the usage limit has Try again.
  rendered
  |> string.contains(
    "Sol: Codex usage limit reached (resets 3:45 PM)<button type=\"button\">Try again</button>",
  )
  |> should.be_true
  rendered
  |> string.contains("<li>Luna: codex is not installed</li>")
  |> should.be_true
  // Try again's reply is the model list as it is now: the cleared model is offered.
  let #(model, _) = task.update(model, task.TryAgain("codex"))
  let #(model, _) =
    task.update(
      model,
      task.ModelsLoaded(
        "[{\"id\":\"sol\",\"label\":\"Sol\",\"agent\":\"codex\",\"available\":true,\"limit\":false}]",
      ),
    )
  model.model |> should.equal("sol")
  html(model) |> string.contains("unavailable</summary>") |> should.be_false
}

/// A run shows the model its agent used, when its start event named one;
/// an alias says which model it resolved to.
pub fn a_run_names_the_model_it_used_test() {
  let model =
    loaded(
      "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"needs_review\",\"rank\":1,\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"ended\",\"model\":\"claude-opus\",\"effort\":\"high\",\"log\":\"/l\",\"outcome\":\"finished\",\"used\":\"claude-opus-5-5\"}]}",
    )
  let assert Some(t) = model.task
  let assert [run] = t.runs
  run.used |> should.equal("claude-opus-5-5")
  let #(model, _) = task.update(model, task.Tab("runs"))
  html(model)
  |> string.contains("work · claude-opus · high · claude-opus-5-5")
  |> should.be_true
}

/// Every model held by a usage limit: one line about the limits, not the
/// install steps the agents are already past. A model that isn't installed
/// keeps the install steps.
pub fn all_models_held_by_usage_limits_says_so_test() {
  let loaded = fn(models: String) {
    let model = fresh("1")
    let #(model, _) = task.update(model, task.ModelsLoaded(models))
    let #(model, _) =
      task.update(
        model,
        task.TaskLoaded(
          model.generation,
          "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"planning\",\"rank\":1}",
        ),
      )
    html(model)
  }
  let rendered =
    loaded(
      "[{\"id\":\"sol\",\"label\":\"Sol\",\"agent\":\"codex\",\"available\":false,\"reason\":\"Codex usage limit reached (resets 3:45 PM)\",\"limit\":true},{\"id\":\"luna\",\"label\":\"Luna\",\"agent\":\"claude\",\"available\":false,\"reason\":\"Claude Code usage limit reached (resets 8pm)\",\"limit\":true}]",
    )
  rendered
  |> string.contains("Every model is held by a usage limit")
  |> should.be_true
  rendered |> string.contains("No model can start yet") |> should.be_false
  rendered |> string.contains("claude.ai/install.sh") |> should.be_false
  rendered |> string.contains("<select") |> should.be_false
  // A model that isn't installed: the install steps stand.
  let rendered =
    loaded(
      "[{\"id\":\"sol\",\"label\":\"Sol\",\"agent\":\"codex\",\"available\":false,\"reason\":\"Codex usage limit reached\",\"limit\":true},{\"id\":\"luna\",\"label\":\"Luna\",\"agent\":\"claude\",\"available\":false,\"reason\":\"claude is not installed\",\"limit\":false}]",
    )
  rendered |> string.contains("No model can start yet") |> should.be_true
  rendered |> string.contains("claude.ai/install.sh") |> should.be_true
}
