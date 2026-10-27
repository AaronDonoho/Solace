//! The cargo check after a move: only a diagnostic the move introduced stops it.
use std::collections::HashMap;
pub(crate) fn post_move_check(s: &crate::Store, before: &HashMap<String, String>) -> Result<(), crate::Failure> {
    let ix = s.index()?;
    let diags = s.check_all(&ix)?;
    let errors: Vec<&crate::Diag> = diags
        .iter()
        .filter(|d| d.error)
        .filter(|d| {
            let key = d.symbol.clone().or_else(|| d.module.clone().map(|m| format!("{m} imports")));
            key.as_ref().is_none_or(|k| !before.contains_key(k))
        })
        .collect();
    if !errors.is_empty() {
        return crate::fail(format!("Not moved: the result does not type-check, so nothing changed.\n\n{}", crate::group_errors(&errors)));
    }
    Ok(())
}
