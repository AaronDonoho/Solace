use std::collections::HashMap;
use crate::{fail, Failure};
use crate::diagnostics::{Diag, group_errors};
pub(crate) fn post_move_check(s: &crate::Store, before: &HashMap<String, String>) -> Result<Vec<Diag>, Failure> {
    let ix = s.index()?;
    post_move_diags(before, s.check_all(&ix)?)
}

/// The warnings from a check already run for a move, or the failure when it
/// added an error.
pub(crate) fn post_move_diags(before: &HashMap<String, String>, diags: Vec<Diag>) -> Result<Vec<Diag>, Failure> {
    let errors = new_errors(before, &diags);
    if !errors.is_empty() {
        return fail(format!("Not moved: the result does not type-check, so nothing changed.\n\n{}", group_errors(&errors)));
    }
    Ok(diags.into_iter().filter(|d| !d.error).collect())
}

/// The errors a check added, rather than ones already on the project.
pub(crate) fn new_errors<'a>(before: &HashMap<String, String>, diags: &'a [Diag]) -> Vec<&'a Diag> {
    diags
        .iter()
        .filter(|d| d.error)
        .filter(|d| {
            let key = d.symbol.clone().or_else(|| d.module.clone().map(|m| format!("{m} imports")));
            key.as_ref().is_none_or(|k| !before.contains_key(k))
        })
        .collect()
}
