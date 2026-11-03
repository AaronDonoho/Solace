//! The roles Solace runs agents in: one stable list that Settings, the board
//! and briefs read. A role says what an agent does, never which model it uses.

use serde_json::{Value, json};

/// One agent role: its stable identifier, its name for humans, what it does,
/// and the run kinds it covers. A role with no run kinds starts no run yet.
pub(crate) struct Role {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) purpose: &'static str,
    pub(crate) runs: &'static [&'static str],
}

/// Every role, in the order Settings and the board show it.
pub(crate) const ROLES: [Role; 5] = [
    Role {
        id: "strategist",
        name: "Strategist",
        purpose: "Plans with the human on the Strategy page, turning their direction into tasks, features and milestones.",
        runs: &[STRATEGY_RUN],
    },
    Role {
        id: "investigator",
        name: "Investigator",
        purpose: concat!(
            "Investigates a task and reports what it needs, the constraints and the risks ",
            "before work begins, without changing the project."
        ),
        runs: &["investigate", "discuss"],
    },
    Role { id: "worker", name: "Worker", purpose: "Makes the change a task describes and checks it.", runs: &["work", "new_work"] },
    Role {
        id: "reviewer",
        name: "Reviewer",
        purpose: "Reviews a task's changes and reports what needs work, without changing the project.",
        runs: &["review"],
    },
    Role {
        id: "coordinator",
        name: "Coordinator",
        purpose: "Coordinates several agents and checks their combined changes. It starts no run yet.",
        runs: &[],
    },
];

/// The role with this identifier, or none when it is unknown.
pub(crate) fn from_id(id: &str) -> Option<&'static Role> {
    ROLES.iter().find(|role| role.id == id)
}

/// The one role that covers a run kind, or none when it is unknown.
pub(crate) fn for_run(kind: &str) -> Option<&'static Role> {
    ROLES.iter().find(|role| role.runs.contains(&kind))
}

/// The role a served role object names, or none when its identifier is unknown.
#[cfg(test)]
pub(crate) fn from_json(value: &Value) -> Option<&'static Role> {
    from_id(value["id"].as_str()?)
}

/// One role as the API serves it.
pub(crate) fn to_json(role: &Role) -> Value {
    json!({"id": role.id, "name": role.name, "purpose": role.purpose, "runs": role.runs})
}

/// Every role as the API serves it.
pub(crate) fn api() -> Value {
    Value::Array(ROLES.iter().map(to_json).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_role_set_is_the_five_canonical_roles() {
        let ids: Vec<&str> = ROLES.iter().map(|role| role.id).collect();
        assert_eq!(ids, ["strategist", "investigator", "worker", "reviewer", "coordinator"]);
        for role in &ROLES {
            assert!(!role.name.is_empty(), "{} has no name", role.id);
            assert!(!role.purpose.is_empty(), "{} has no purpose", role.id);
        }
    }

    #[test]
    fn each_run_kind_belongs_to_exactly_one_role() {
        let expected = [
            (STRATEGY_RUN, "strategist"),
            ("investigate", "investigator"),
            ("discuss", "investigator"),
            ("work", "worker"),
            ("new_work", "worker"),
            ("review", "reviewer"),
        ];
        for (kind, id) in expected {
            assert_eq!(for_run(kind).map(|role| role.id), Some(id), "{kind}");
        }
        let covered: Vec<&str> = ROLES.iter().flat_map(|role| role.runs.iter().copied()).collect();
        assert_eq!(covered.len(), expected.len());
        for kind in &covered {
            assert_eq!(covered.iter().filter(|other| *other == kind).count(), 1, "{kind}");
        }
        assert!(for_run("unknown").is_none());
        assert!(from_id("coordinator").is_some_and(|role| role.runs.is_empty()));
    }

    #[test]
    fn roles_round_trip_through_the_served_json() {
        let all = api();
        let listed = all.as_array().unwrap();
        assert_eq!(listed.len(), ROLES.len());
        for (role, served) in ROLES.iter().zip(listed) {
            assert_eq!(served["id"], role.id);
            assert_eq!(served["name"], role.name);
            assert_eq!(served["purpose"], role.purpose);
            assert_eq!(served["runs"], json!(role.runs));
            assert_eq!(from_json(served).map(|found| found.id), Some(role.id));
            assert!(served.get("model").is_none());
        }
        assert!(from_id("unknown").is_none());
        assert!(from_json(&json!({"id": "unknown"})).is_none());
        assert!(from_json(&json!({})).is_none());
    }
}

/// The strategist's run kind: strategy runs record it and map back through `for_run`.
pub(crate) const STRATEGY_RUN: &str = "strategy";
