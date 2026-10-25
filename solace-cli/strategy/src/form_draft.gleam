//// Description drafts the Strategy page's forms keep in browser storage.
//// An editor is a form's textarea; its owner is the record's id, or "new".

import gleam/option.{type Option, None, Some}
import solace_task

/// The owner a record draft belongs to: its id, or "new".
pub fn owner(id: Option(String)) -> String {
  case id {
    None -> "new"
    Some(id) -> id
  }
}

/// The storage key of `editor`'s draft for `owner`.
pub fn key(editor: String, owner: String) -> String {
  solace_task.draft_key(editor, owner)
}

/// The description saved for `owner`, "" when there is none.
pub fn load(editor: String, owner: String) -> String {
  solace_task.draft_load(editor, owner)
}

/// Saves a description, removing it when blank.
pub fn save(editor: String, owner: String, body: String) -> Nil {
  solace_task.draft_save(editor, owner, body)
}

/// Removes a description.
pub fn clear(editor: String, owner: String) -> Nil {
  solace_task.draft_clear(editor, owner)
}

/// `draft` with the stored description when one was typed.
pub fn with_body(
  editor: String,
  owner: String,
  draft: a,
  set_body: fn(a, String) -> a,
) -> a {
  case load(editor, owner) {
    "" -> draft
    body -> set_body(draft, body)
  }
}
