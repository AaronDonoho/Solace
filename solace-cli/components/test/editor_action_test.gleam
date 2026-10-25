import editor_action_support_test as editor_action_test_support
import gleam/string
import gleeunit/should
import lustre/dev/query
import lustre/dev/simulate
import solace_task as task

fn action_clicked(action: task.EditorAction(Int)) {
  simulate.simple(
    init: fn(_) { 0 },
    update: fn(_, message) { message },
    view: fn(_) {
      task.editor(
        task.EditorConfig(
          id: "body",
          task: "t1",
          base: "/p/a",
          label: "Description",
          text: "",
          on_input: fn(_) { 0 },
          on_format: fn(_, _) { 0 },
          on_block: fn(_) { 0 },
          actions: [action],
        ),
      )
    },
  )
  |> simulate.start(Nil)
  |> simulate.click(on: query.element(matching: query.class("composer-action")))
}

/// A click on the editor's action sends the message it was given.
pub fn an_editor_action_click_sends_its_message_test() {
  action_clicked(task.EditorAction("", "Send", 7, False))
  |> simulate.model
  |> should.equal(7)
}

/// A disabled editor action has no click handler, so a click sends nothing.
pub fn a_disabled_editor_action_click_sends_nothing_test() {
  action_clicked(task.EditorAction("", "Send", 7, True))
  |> simulate.model
  |> should.equal(0)
}

/// With no content, an editor's action is the send icon, in `currentColor`,
/// and its accessible name is the one given.
pub fn editor_action_defaults_to_a_labeled_send_icon_test() {
  let rendered =
    editor_action_test_support.rendered([
      task.EditorAction("", "Send", Nil, False),
    ])
  let button = editor_action_test_support.action_button(rendered)
  button |> string.contains("aria-label=\"Send\"") |> should.be_true
  button |> string.contains("<svg") |> should.be_true
  button |> string.contains("fill=\"currentColor\"") |> should.be_true
  button |> string.contains("aria-hidden=\"true\"") |> should.be_true
  rendered |> string.contains(">Send</button>") |> should.be_false
}

/// Words replace the send icon as the action's content, and stay its
/// accessible name when they are also passed as the name.
pub fn editor_action_words_replace_the_send_icon_test() {
  let rendered =
    editor_action_test_support.rendered([
      task.EditorAction("Save", "Save", Nil, True),
    ])
  let button = editor_action_test_support.action_button(rendered)
  button |> string.contains("aria-label=\"Save\"") |> should.be_true
  button |> string.contains("class=\"composer-action-text\"") |> should.be_true
  button |> string.contains(">Save</span>") |> should.be_true
  rendered |> string.contains("<svg") |> should.be_false
}

/// A disabled action is a disabled button, so a press sends nothing; an
/// enabled one is not disabled.
pub fn editor_action_disabled_sends_nothing_test() {
  let render = fn(disabled) {
    editor_action_test_support.rendered([
      task.EditorAction("", "Send", Nil, disabled),
    ])
    |> editor_action_test_support.action_button
  }
  render(True) |> string.contains(" disabled") |> should.be_true
  render(False) |> string.contains(" disabled") |> should.be_false
}

/// The action is the composer's last child, inside the editor's box at its
/// bottom right; the send icon keeps a right inset on every line, and worded
/// actions reserve their measured room on the last line only, without an
/// empty band below the text. The actions stay on one line and truncate when
/// the composer is too narrow.
pub fn editor_action_sits_inside_the_composer_test() {
  let page =
    editor_action_test_support.rendered([
      task.EditorAction("Save", "Save", Nil, False),
    ])
  page
  |> string.contains("class=\"composer has-actions has-action-words\"")
  |> should.be_true
  page |> string.ends_with("</button></div></div>") |> should.be_true
  task.editor_styles
  |> string.contains(
    ".composer.has-actions{--md-line-pad:calc(var(--control-h-small) + var(--space-3))}",
  )
  |> should.be_true
  task.editor_styles
  |> string.contains(
    ".composer.has-action-words{--md-action-gap:var(--space-2);--md-line-pad:0px}",
  )
  |> should.be_true
  task.editor_styles
  |> string.contains("11rem")
  |> should.be_false
  task.editor_styles
  |> string.contains("max-width:calc(50% - var(--space-2))")
  |> should.be_true
  task.editor_styles
  |> string.contains("flex-wrap:nowrap;justify-content:flex-end")
  |> should.be_true
  task.editor_styles
  |> string.contains(
    ".composer-action{display:inline-flex;align-items:center;justify-content:center;flex:0 1 auto;min-width:var(--control-h-small);min-height:var(--control-h-small);margin:0;padding:0 var(--space-2);overflow:hidden}",
  )
  |> should.be_true
  task.editor_styles
  |> string.contains(
    ".composer-action-text{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;min-width:0}",
  )
  |> should.be_true
  task.editor_styles
  |> string.contains(".composer.has-actions solace-md-editor{padding-bottom")
  |> should.be_false
  task.editor_styles |> string.contains(":has(") |> should.be_false
  // The send icon alone takes the smaller inset.
  let icon =
    editor_action_test_support.rendered([
      task.EditorAction("", "Send", Nil, False),
    ])
  icon |> string.contains("class=\"composer has-actions\"") |> should.be_true
  icon |> string.contains("has-action-words") |> should.be_false
  // With no actions, no room is reserved.
  let plain = editor_action_test_support.rendered([])
  plain |> string.contains("class=\"composer\"") |> should.be_true
  plain |> string.contains("has-actions") |> should.be_false
}
