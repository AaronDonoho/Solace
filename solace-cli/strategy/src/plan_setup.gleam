//// The Strategy page's planning panel settings: its words, its draft and
//// where its runs are asked for (D-93 point 7).

import plan_panel
import solace_task

pub fn settings() -> plan_panel.Settings {
  plan_panel.Settings(
    id: "plan",
    address: "/strategy/runs",
    text_id: "plan-text",
    draft_owner: "page:",
    labels: labels(),
  )
}

fn labels() -> plan_panel.Labels {
  plan_panel.Labels(
    title: solace_task.ui_label("plan"),
    context: solace_task.ui_label("plan_context"),
    whole: "Whole Project",
    open_here: solace_task.ui_label("plan_open_here"),
    marker: solace_task.ui_label("plan"),
    marker_title: solace_task.ui_label("plan_marker"),
    run_open: "A run on this is open",
    resize: "Resize the planning panel",
    earlier: "Earlier runs",
    new_message: "New message ↓",
  )
}
