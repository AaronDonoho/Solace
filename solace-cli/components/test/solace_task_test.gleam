import gleam/dict
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
  task.actions(model.workflow, t)
  |> should.equal(["assign", "pause", "discuss"])
  task.actions(model.workflow, task.Task(..t, column: "done"))
  |> should.equal([])
  task.actions(model.workflow, task.Task(..t, column: "gone"))
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
  rendered
  |> string.contains("unavailable: codex is not installed")
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
  html(model) |> string.contains("Follow live") |> should.be_true
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
  task.actions(workflow, one("work", "running"))
  |> should.equal(["assign", "pause", "stop"])
  task.actions(workflow, one("discuss", "waiting"))
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
  let #(model, _) = task.update(model, task.Preview("edit-body", True))
  let #(model, _) = task.update(model, task.Preview("note-text", True))
  model.previews |> should.equal(["note-text", "edit-body"])
  let #(model, _) = task.update(model, task.Preview("edit-body", False))
  model.previews |> should.equal(["note-text"])
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
  task.actions(model.workflow, t) |> should.equal(["plan", "pause", "discuss"])
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
  scope |> string.contains("You can change the projects") |> should.be_false
  let running =
    "{\"id\":\"t1\",\"title\":\"Across\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1,\"scope\":[\"a\",\"b\"],\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"running\",\"model\":\"m\",\"log\":\"/l\"}]}"
  let assert Ok(r) = json.parse(running, task.task_decoder())
  body(task.Model(..model, task: Some(r)))
  |> string.contains("You can change the projects")
  |> should.be_true
  // Switches stack vertically.
  let assert [css, ..] = string.split(html(model), "</style>")
  css
  |> string.contains(".projects-pick label{display:flex;")
  |> should.be_true
  css
  |> string.contains(".projects-pick input:checked::before{transform:")
  |> should.be_true
}

/// The detail's styles set no colours of their own: a nested shadow root
/// inherits them from its host, so the board's dark theme reaches it.
pub fn styles_inherit_the_hosts_colours_test() {
  let assert [css, ..] = string.split(html(fresh("1")), "</style>")
  css |> string.contains("--paper:") |> should.be_false
  css |> string.contains("--ink:") |> should.be_false
  css |> string.contains(":host{display:block}") |> should.be_true
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
      False,
      fn(_) { Nil },
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
  task.actions(model.workflow, t) |> should.equal(["plan", "pause", "discuss"])
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
    "Approving also sets the feature <em>Search</em> back to planned.",
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
  task.actions(workflow, one("complete", "")) |> should.equal([])
  task.actions(workflow, one("cancelled", "")) |> should.equal(["to_backlog"])
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
