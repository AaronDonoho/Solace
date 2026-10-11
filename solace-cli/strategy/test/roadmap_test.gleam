import features
import gleam/json
import gleam/list
import gleam/option.{None, Some}
import roadmap

fn milestone(id: String, rank: Float) -> features.Milestone {
  features.Milestone(id, id, rank, "", None, "proposed")
}

fn feature(id: String, target: String, rank: Float) -> features.Feature {
  features.Feature(
    id,
    id,
    "",
    case target {
      "" -> None
      m -> Some(m)
    },
    rank,
    "planned",
  )
}

fn task(id: String, feature: String, milestone: String) -> roadmap.Task {
  let link = fn(s) {
    case s {
      "" -> None
      s -> Some(s)
    }
  }
  roadmap.Task(".", id, id, link(feature), link(milestone), [], False)
}

pub fn a_milestone_holds_its_targets_and_the_features_of_its_tasks_test() {
  let a = feature("a", "m1", 1.0)
  let b = feature("b", "m2", 1.0)
  let c = feature("c", "", 0.5)
  let d = feature("d", "", 2.0)
  let t1 = task("t1", "a", "m1")
  let t2 = task("t2", "b", "m1")
  let t3 = task("t3", "c", "m1")
  let t4 = task("t4", "c", "")
  let t5 = task("t5", "", "m2")
  // No milestone of its own: in its feature's target.
  let t6 = task("t6", "a", "")
  // Neither a milestone nor a feature: not on the roadmap.
  let t7 = task("t7", "", "")
  let groups =
    roadmap.groups([milestone("m2", 2.0), milestone("m1", 1.0)], [d, b, a, c], [
      t1,
      t2,
      t3,
      t4,
      t5,
      t6,
      t7,
    ])
  assert groups
    == [
      // b is in m1 through its task (D-87 point 2); a once, though it both
      // targets m1 and has tasks there.
      roadmap.Group(
        Some(milestone("m1", 1.0)),
        [#(c, [t3]), #(a, [t1, t6]), #(b, [t2])],
        [],
      ),
      roadmap.Group(Some(milestone("m2", 2.0)), [#(b, [])], [t5]),
      // c's task with no milestone, and d, in no milestone at all.
      roadmap.Group(None, [#(c, [t4]), #(d, [])], []),
    ]
  // Nothing loose: no "No milestone" group.
  assert roadmap.groups([milestone("m1", 1.0)], [a], [t1])
    == [roadmap.Group(Some(milestone("m1", 1.0)), [#(a, [t1])], [])]
}

pub fn create_body_names_the_request_and_the_target_date_test() {
  let draft =
    roadmap.Draft(
      ..roadmap.new_draft(),
      title: "1.0",
      target_date: Some("2027-01-01"),
    )
  assert json.to_string(roadmap.create_body(draft, "r1"))
    == "{\"title\":\"1.0\",\"body\":\"\",\"state\":\"proposed\",\"targetDate\":\"2027-01-01\",\"request\":\"r1\"}"
  assert json.to_string(roadmap.create_body(roadmap.new_draft(), "r2"))
    == "{\"title\":\"\",\"body\":\"\",\"state\":\"proposed\",\"request\":\"r2\"}"
}

pub fn an_edit_sends_only_the_changed_fields_test() {
  let m =
    features.Milestone("m1", "1.0", 1.0, "b", Some("2027-01-01"), "planned")
  let draft = roadmap.draft_of(m)
  assert roadmap.changes(m, draft) == []
  let changed = roadmap.Draft(..draft, title: "1.1", state: "active")
  assert json.to_string(json.object(roadmap.changes(m, changed)))
    == "{\"title\":\"1.1\",\"state\":\"active\"}"
  // A cleared target date is sent as null.
  let cleared = roadmap.Draft(..draft, target_date: None)
  assert json.to_string(json.object(roadmap.changes(m, cleared)))
    == "{\"targetDate\":null}"
}

pub fn depends_body_and_delete_text_test() {
  assert json.to_string(roadmap.depends_body(["t1", "t2"]))
    == "{\"dependsOn\":[\"t1\",\"t2\"]}"
  assert json.to_string(roadmap.depends_body([])) == "{\"dependsOn\":[]}"
  assert roadmap.delete_text(1, 2)
    == "Its 1 feature and 2 tasks lose their link to it; they stay."
}

pub fn any_task_of_the_root_can_be_a_dependency_but_a_cycle_test() {
  let with = fn(t: roadmap.Task, dir, depends, closed) {
    roadmap.Task(..t, dir: dir, depends: depends, closed: closed)
  }
  // Another project and another milestone are offered.
  let a = task("a", "", "m1")
  let b = with(task("b", "", "m1"), ".", ["a"], False)
  let c = with(task("c", "", "m1"), ".", ["b"], False)
  let far = with(task("far", "", "m9"), "web", [], False)
  let done = with(task("done", "", ""), ".", [], True)
  let mine = with(task("mine", "", "m2"), ".", ["far"], False)
  let all = [c, done, mine, far, b, a]
  let ids = fn(ts: List(roadmap.Task)) { list.map(ts, fn(t) { t.id }) }
  // a: b and c depend on it, through each other; a closed task comes last.
  assert ids(roadmap.candidates(all, a, "")) == ["far", "mine", "done"]
  // mine: not itself nor what it depends on already.
  assert ids(roadmap.candidates(all, mine, "")) == ["a", "b", "c", "done"]
  // Every word, ignoring case.
  assert ids(roadmap.candidates(all, a, "FA")) == ["far"]
  assert ids(roadmap.candidates(all, a, "f m")) == []
}

pub fn dependents_are_found_through_a_long_chain_and_a_cycle_in_the_data_test() {
  let node = fn(id, depends) {
    roadmap.Task(".", id, id, None, None, depends, False)
  }
  // d -> c -> b -> a, and x <-> y already cycle; x, y don't touch a's chain.
  let all = [
    node("a", []),
    node("b", ["a"]),
    node("c", ["b"]),
    node("d", ["c"]),
    node("x", ["y"]),
    node("y", ["x"]),
  ]
  let ids = fn(ts: List(roadmap.Task)) { list.map(ts, fn(t) { t.id }) }
  assert ids(roadmap.candidates(all, node("a", []), "")) == ["x", "y"]
  assert ids(roadmap.candidates(all, node("x", ["y"]), ""))
    == ["a", "b", "c", "d"]
}
