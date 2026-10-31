#[cfg(test)] use crate::extract::rust::rewrite::collapse_single_member_groups;
/// Collapsing a group whose only member renames `self` keeps the alias.
#[test]
fn a_collapsed_self_alias_keeps_its_alias() {
    assert_eq!(collapse_single_member_groups("crate::a::{self as m}"), "crate::a as m");
    assert_eq!(collapse_single_member_groups("crate::a::{self}"), "crate::a");
    assert_eq!(collapse_single_member_groups("crate::a::{self as m,}"), "crate::a as m");
    assert_eq!(collapse_single_member_groups("crate::a::{self,}"), "crate::a");
    assert_eq!(collapse_single_member_groups("crate::a::{self as m, x}"), "crate::a::{self as m, x}");
}

