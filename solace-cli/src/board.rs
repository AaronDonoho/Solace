use super::*;
#[cfg(test)] use std::os::unix::process::CommandExt;
use std::time::{Duration, Instant};

mod prerequisites;
pub(super) mod runs;
pub(super) mod server;
pub(super) mod task_model;
pub(super) mod reviews;
pub(super) mod briefs;
pub(super) mod settings;
pub(super) mod roles;
pub(crate) mod unfinished;
pub(crate) mod delivery;

use serde_json::Value;
use std::env;
pub(super) use crate::board::runs::after;
#[cfg(test)] pub(crate) use crate::board::task_model::valid_id;
pub(super) use crate::board::task_model::task;
pub(super) use crate::board::task_model::all_tasks;
#[cfg(test)] pub(super) use crate::board::task_model::adopt_tasks;
pub(super) use crate::board::task_model::flow;
pub(super) use crate::board::task_model::column;
pub(super) use crate::board::task_model::active;
#[cfg(test)] pub(super) use crate::board::task_model::move_to;
pub(super) use crate::board::task_model::new_task;
pub(super) use crate::board::runs::record_reads;
pub(super) use crate::board::runs::run_reads;
#[cfg(test)] pub(super) use crate::board::runs::stream_task_event;
pub(super) use crate::board::reviews::snapshot;
pub(super) use crate::board::reviews::record;
pub(super) use crate::board::reviews::existing_folders;
#[cfg(test)] pub(super) use crate::board::reviews::kept_on_disk;
pub(super) use crate::board::reviews::changes_of;
pub(super) use crate::board::reviews::reviewable;
pub(super) use crate::board::reviews::changed_lines;
pub(super) use crate::board::reviews::diff_ops;
pub(super) use crate::board::reviews::restore;
#[cfg(test)] pub(super) use crate::board::reviews::settle_final;
pub(super) use crate::board::reviews::before_file;
pub(super) use crate::board::reviews::fingerprint;
#[cfg(test)] pub(super) use crate::board::briefs::remember;
#[cfg(test)] pub(crate) use crate::board::server::api;
#[cfg(test)] pub(super) use crate::board::task_model::read_task_api;
#[cfg(test)] pub(super) use crate::board::task_model::created;
pub(super) use crate::board::task_model::lock;
pub(super) use crate::board::task_model::try_lock;
pub(super) use crate::board::task_model::open;
#[cfg(test)] pub(super) use crate::board::task_model::store_dir;
#[cfg(test)] pub(crate) use crate::board::setup::parse_system;
pub(super) use crate::board::runs::context;
pub(super) use crate::board::runs::strategy_context;
pub(crate) use strategy::{design_command, design_guard};
pub(super) fn err<T>(code: &str, message: impl std::fmt::Display) -> Result<T, Failure> {
    lockdown::error(code, "task", message)
}
/// The tasks whose home is the project `root`, in rank order.
pub(super) fn tasks(root: &Path) -> Result<Vec<Value>, Failure> {
    crate::board::task_model::tasks(root)
}
// Whether a model can start: its agent kind's checks and usage-limit marks.
// An endpoint that is down shows up as a failed run.
mod agents;
pub(super) mod setup;
pub use agents::unavailable;
// Strategy runs (D-93) are in `strategy.rs`, a module of this one, so they
// share its helpers.
#[path = "strategy.rs"]
mod strategy;

#[cfg(test)]
use strategy::{strategy_brief, strategy_list, strategy_note_show, strategy_run_show, strategy_show};
pub use strategy::{strategy_command, strategy_guard};
#[cfg(test)]
// The unit tests are in their own file, so this one stays within watch_limit.
#[path = "board_tests.rs"]
mod tests;
