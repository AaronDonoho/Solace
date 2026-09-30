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
        "{\"columns\":[{\"id\":\"triage\",\"label\":\"Triage\",\"actions\":[\"assign\"]},{\"id\":\"done\",\"final\":true}],\"types\":{\"implementation\":{\"writes\":true}}}",
      ),
    )
  let text =
    "{\"id\":\"1\",\"title\":\"Fix checkout\",\"body\":\"Details\",\"column\":\"triage\",\"rank\":1,\"type\":\"implementation\",\"paused_from\":null,\"stamps\":{},\"notes\":[],\"questions\":[],\"runs\":[]}"
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
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"blocked\",\"rank\":1,\"type\":\"implementation\",\"attention\":{\"reason\":\"task timeout\",\"time\":1}}",
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
        "{\"id\":\"1\",\"title\":\"T\",\"body\":\"\",\"column\":\"assigned\",\"rank\":1,\"type\":\"implementation\",\"notes\":[{\"author\":\"agent\",\"text\":\"Done: **five** tests <b>pass</b>\",\"time\":1}],\"runs\":[{\"id\":\"r1\",\"kind\":\"work\",\"state\":\"running\",\"model\":\"m\",\"log\":\"/l\",\"started\":1}]}",
      ),
    )
  let assert Some(task) = model.selected
  board.status(task) |> should.equal("Agent working")
  let rendered = board.view(model) |> element.to_string
  rendered |> string.contains("status working") |> should.be_true
  // Markdown is formatted, and agent HTML is escaped.
  rendered |> string.contains("<strong>five</strong>") |> should.be_true
  rendered |> string.contains("&lt;b&gt;pass") |> should.be_true
  rendered |> string.contains("Follow live") |> should.be_true
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
