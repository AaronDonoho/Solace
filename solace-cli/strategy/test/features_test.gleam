import features
import gleam/json
import gleam/option.{None, Some}

fn feature(id: String, milestone: String, rank: Float) -> features.Feature {
  feature_in_state(id, milestone, rank, "upcoming")
}

pub fn the_decoder_defaults_everything_but_the_id_test() {
  let assert Ok(f) =
    json.parse(
      "{\"id\":\"f1\",\"title\":\"Search\",\"body\":\"b\",\"milestone\":\"m1\",\"rank\":2,\"state\":\"active\",\"startTime\":40,\"colorSlot\":5}",
      features.decoder(),
    )
  assert f
    == features.Feature("f1", "Search", "b", Some("m1"), 2.0, "active", Some(5))
  let assert Ok(bare) = json.parse("{\"id\":\"f2\"}", features.decoder())
  assert bare == features.Feature("f2", "", "", None, 0.0, "proposed", None)
  let assert Ok(cleared) =
    json.parse("{\"id\":\"f3\",\"milestone\":null}", features.decoder())
  assert cleared.milestone == None
  assert json.parse("{\"title\":\"x\"}", features.decoder()) != Ok(bare)
  let assert Ok(m) =
    json.parse("{\"id\":\"m1\",\"rank\":1.5}", features.milestone_decoder())
  assert m == features.Milestone("m1", "", 1.5, "", None, "proposed")
  let assert Ok(full) =
    json.parse(
      "{\"id\":\"m2\",\"title\":\"1.0\",\"body\":\"b\",\"rank\":2,\"targetDate\":\"2027-01-01\",\"state\":\"upcoming\"}",
      features.milestone_decoder(),
    )
  assert full
    == features.Milestone("m2", "1.0", 2.0, "b", Some("2027-01-01"), "upcoming")
}

pub fn groups_follow_milestone_rank_then_feature_rank_then_id_test() {
  let milestones = [
    features.Milestone("m2", "Later", 2.0, "", None, "proposed"),
    features.Milestone("m1", "Launch", 1.0, "", None, "proposed"),
    features.Milestone("m3", "Empty", 3.0, "", None, "proposed"),
  ]
  let groups =
    features.groups(milestones, [
      feature("b", "m1", 1.0),
      feature("a", "m1", 1.0),
      feature("c", "m1", 0.5),
      feature("d", "m2", 1.0),
      feature("loose", "", 0.0),
      feature("gone", "m9", 0.0),
    ])
  assert groups
    == [
      features.Open(
        Some(features.Milestone("m1", "Launch", 1.0, "", None, "proposed")),
        [
          feature("c", "m1", 0.5),
          feature("a", "m1", 1.0),
          feature("b", "m1", 1.0),
        ],
      ),
      features.Open(
        Some(features.Milestone("m2", "Later", 2.0, "", None, "proposed")),
        [
          feature("d", "m2", 1.0),
        ],
      ),
      features.Open(
        Some(features.Milestone("m3", "Empty", 3.0, "", None, "proposed")),
        [],
      ),
      features.Open(None, [
        feature("gone", "m9", 0.0),
        feature("loose", "", 0.0),
      ]),
    ]
}

pub fn no_milestone_group_is_left_out_when_empty_test() {
  assert features.groups(
      [features.Milestone("m1", "L", 1.0, "", None, "proposed")],
      [
        feature("a", "m1", 0.0),
      ],
    )
    |> list_len
    == 1
}

fn list_len(groups: List(features.Group)) -> Int {
  case groups {
    [] -> 0
    [_, ..rest] -> 1 + list_len(rest)
  }
}

pub fn create_body_names_the_request_and_the_milestone_test() {
  let draft =
    features.Draft(None, "Search", "Find **fast**", "upcoming", Some("m1"))
  assert json.to_string(features.create_body(draft, "r1"))
    == "{\"title\":\"Search\",\"body\":\"Find **fast**\",\"state\":\"upcoming\",\"milestone\":\"m1\",\"request\":\"r1\"}"
  // No milestone: the field is left out.
  assert json.to_string(features.create_body(features.new_draft(), "r2"))
    == "{\"title\":\"\",\"body\":\"\",\"state\":\"proposed\",\"request\":\"r2\"}"
}

pub fn an_edit_sends_only_the_changed_fields_test() {
  let f =
    features.Feature("f1", "Search", "Body", Some("m1"), 1.0, "proposed", None)
  let draft = features.draft_of(f)
  assert features.changes(f, draft) == []
  let changed = features.Draft(..draft, title: "Find", state: "active")
  assert json.to_string(json.object(features.changes(f, changed)))
    == "{\"title\":\"Find\",\"state\":\"active\"}"
  // A cleared milestone is sent as null.
  let cleared = features.Draft(..draft, milestone: None)
  assert json.to_string(json.object(features.changes(f, cleared)))
    == "{\"milestone\":null}"
}

pub fn a_move_takes_the_midpoint_or_one_past_the_end_test() {
  let group = [
    feature("a", "", 1.0),
    feature("b", "", 2.0),
    feature("c", "", 4.0),
  ]
  // To the first position: one below the first.
  assert features.move_rank(group, "b", True) == Some(0.0)
  // To a middle position: the midpoint of the new neighbors.
  assert features.move_rank(group, "c", True) == Some(1.5)
  assert features.move_rank(group, "a", False) == Some(3.0)
  // To the last position: one above the last.
  assert features.move_rank(group, "b", False) == Some(5.0)
  assert features.move_rank(group, "a", True) == None
  assert features.move_rank(group, "c", False) == None
}

pub fn the_delete_text_names_the_task_count_test() {
  assert features.delete_text(2)
    == "Its 2 tasks lose their link to it; the tasks stay."
  assert features.delete_text(1)
    == "Its 1 task loses its link to it; the tasks stay."
}

pub fn a_drop_takes_the_midpoint_of_its_new_neighbors_test() {
  let group = [
    feature("a", "", 1.0),
    feature("b", "", 2.0),
    feature("c", "", 4.0),
  ]
  // To the first position: one below the first.
  assert features.drop_rank(group, "c", 0) == Some(0.0)
  // To a middle position, among the others: between a and c.
  assert features.drop_rank(group, "a", 1) == Some(3.0)
  // To the last position: one above the last.
  assert features.drop_rank(group, "a", 2) == Some(5.0)
  // Where it is, cancelled, or past the end: no move.
  assert features.drop_rank(group, "b", 1) == None
  assert features.drop_rank(group, "b", -1) == None
  assert features.drop_rank(group, "b", 3) == None
}

pub fn the_grip_reorders_with_the_arrow_keys_only_test() {
  assert features.grip_key("ArrowUp") == Some(True)
  assert features.grip_key("ArrowDown") == Some(False)
  assert features.grip_key("Tab") == None
}

fn feature_in_state(
  id: String,
  milestone: String,
  rank: Float,
  state: String,
) -> features.Feature {
  features.Feature(
    id,
    id,
    "",
    case milestone {
      "" -> None
      m -> Some(m)
    },
    rank,
    state,
    None,
  )
}

pub fn closed_features_form_one_group_after_milestones_test() {
  let m1 = features.Milestone("m1", "Launch", 1.0, "", None, "proposed")
  let m2 = features.Milestone("m2", "Later", 2.0, "", None, "proposed")
  let groups =
    features.groups([m2, m1], [
      feature("open", "m1", 0.5),
      feature_in_state("done", "m1", 1.0, "complete"),
      feature("loose", "", 3.0),
      feature_in_state("dropped", "", 2.0, "canceled"),
      feature_in_state("gone", "m9", 2.0, "complete"),
    ])
  assert groups
    == [
      features.Open(Some(m1), [feature("open", "m1", 0.5)]),
      features.Open(Some(m2), []),
      features.Open(None, [feature("loose", "", 3.0)]),
      features.Closed([
        feature_in_state("done", "m1", 1.0, "complete"),
        feature_in_state("dropped", "", 2.0, "canceled"),
        feature_in_state("gone", "m9", 2.0, "complete"),
      ]),
    ]
}

pub fn no_closed_group_when_no_feature_is_closed_test() {
  let m1 = features.Milestone("m1", "Launch", 1.0, "", None, "proposed")
  assert features.groups([m1], [
      feature("in", "m1", 1.0),
      feature("loose", "", 0.0),
    ])
    == [
      features.Open(Some(m1), [feature("in", "m1", 1.0)]),
      features.Open(None, [feature("loose", "", 0.0)]),
    ]
}
