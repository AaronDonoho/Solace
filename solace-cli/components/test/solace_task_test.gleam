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
        "{\"columns\":[{\"id\":\"triage\",\"label\":\"Triage\",\"actions\":[\"assign\"]},{\"id\":\"done\",\"final\":true}]}",
      ),
    )
  let text =
    "{\"id\":\"1\",\"title\":\"Fix checkout\",\"body\":\"Details\",\"column\":\"triage\",\"rank\":1,\"paused_from\":null,\"stamps\":{},\"notes\":[],\"questions\":[],\"runs\":[]}"
  let assert Ok(t) = json.parse(text, task.task_decoder())
  task.actions(model.workflow, t)
  |> should.equal(["assign", "pause", "discuss"])
  task.actions(model.workflow, task.Task(..t, column: "done"))
  |> should.equal([])
  let #(model, _) = task.update(model, task.TaskLoaded(model.generation, text))
  model.task |> should.equal(Some(t))
  html(model) |> string.contains("Fix checkout") |> should.be_true
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
  model.workflow |> should.equal(task.Workflow([]))
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
    task.Workflow([task.Column("triage", "Triage", ["assign"], False)])
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
      workflow: task.Workflow([
        task.Column("triage", "Triage", ["assign"], False),
      ]),
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
  let #(model, _) =
    task.update(model, task.ChangesLoaded(model.generation, bare))
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
