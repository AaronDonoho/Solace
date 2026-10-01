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

pub fn workflow_and_live_task_test() {
  let #(model, _) = board.init(Nil)
  let #(model, _) =
    board.update(
      model,
      board.WorkflowLoaded(
        "{\"columns\":[{\"id\":\"triage\",\"label\":\"Triage\",\"actions\":[\"assign\"]},{\"id\":\"done\",\"final\":true}]}",
      ),
    )
  let text =
    "{\"id\":\"1\",\"title\":\"Fix checkout\",\"body\":\"Details\",\"column\":\"triage\",\"rank\":1,\"paused_from\":null,\"stamps\":{},\"notes\":[],\"questions\":[],\"runs\":[]}"
  let assert Ok(task) = json.parse(text, board.task_decoder())
  board.actions(model.workflow, task) |> should.equal(["assign", "pause"])
  let #(model, _) = board.update(model, board.TaskLoaded(text))
  model.selected |> should.equal(Some(task))
  let #(model, _) = board.update(model, board.Live("{\"task\":" <> text <> "}"))
  list.length(model.tasks) |> should.equal(1)
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
