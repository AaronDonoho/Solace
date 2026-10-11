import solace_task

/// Registers the shared elements on their own: `<solace-task>`.
pub fn main() -> Nil {
  let assert Ok(_) = solace_task.register()
  Nil
}
