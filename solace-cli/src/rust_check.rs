use std::collections::HashMap;
use crate::{fail, Failure};
use crate::diagnostics::{Diag, group_errors};
pub(crate) fn post_move_check(s: &crate::Store, before: &HashMap<String, String>) -> Result<Vec<Diag>, Failure> {
    let ix = s.index()?;
    let diags = s.check_all(&ix)?;
    let errors: Vec<&Diag> = diags
        .iter()
        .filter(|d| d.error)
        .filter(|d| {
            let key = d.symbol.clone().or_else(|| d.module.clone().map(|m| format!("{m} imports")));
            key.as_ref().is_none_or(|k| !before.contains_key(k))
        })
        .collect();
    if !errors.is_empty() {
        return fail(format!("Not moved: the result does not type-check, so nothing changed.\n\n{}", group_errors(&errors)));
    }
    Ok(diags.into_iter().filter(|d| !d.error).collect())
}
