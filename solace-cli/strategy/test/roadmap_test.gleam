import features
import gleam/json
import gleam/list
import gleam/option.{type Option, None, Some}
import gleam/string
import lustre/element
import lustre/element/html
import roadmap

fn milestone(id: String, rank: Float) -> features.Milestone {
  features.Milestone(id, id, rank, "", None, "proposed")
}

fn feature(id: String, target: String, rank: Float) -> features.Feature {
  feature_in_state(id, target, rank, "upcoming")
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
    features.Milestone("m1", "1.0", 1.0, "b", Some("2027-01-01"), "upcoming")
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

fn dated_milestone(
  id: String,
  rank: Float,
  target_date: String,
) -> features.Milestone {
  features.Milestone(id, id, rank, "", Some(target_date), "proposed")
}

pub fn groups_order_milestones_by_target_date_then_rank_and_id_test() {
  let groups =
    roadmap.groups(
      [
        milestone("undated-b", 2.0),
        dated_milestone("later", 9.0, "2027-06-01"),
        milestone("undated-a", 2.0),
        dated_milestone("earlier", 8.0, "2027-01-01"),
        milestone("undated-first", 1.0),
        dated_milestone("empty", 2.5, ""),
        dated_milestone("same-a", 3.0, "2027-03-01"),
        dated_milestone("same-b", 2.0, "2027-03-01"),
      ],
      [feature("loose", "", 0.0)],
      [],
    )
  assert list.map(groups, fn(g) {
      case g.milestone {
        Some(m) -> m.id
        None -> ""
      }
    })
    == [
      "earlier",
      "same-b",
      "same-a",
      "later",
      "undated-first",
      "undated-a",
      "undated-b",
      "empty",
      "",
    ]
}

pub fn milestone_features_list_targets_and_search_the_others_test() {
  let fs = [
    feature("b", "m1", 1.0),
    feature("a", "m1", 1.0),
    feature("c", "m1", 0.5),
    feature("other", "m2", 1.0),
    feature("loose", "", 2.0),
  ]
  let names = fn(items: List(features.Feature)) {
    list.map(items, fn(f) { f.id })
  }
  assert names(roadmap.targets(fs, "m1")) == ["c", "a", "b"]
  assert roadmap.targets(fs, "m9") == []
  assert names(roadmap.feature_candidates(fs, "m1", "")) == ["loose", "other"]
  assert names(roadmap.feature_candidates(fs, "m1", "OT er")) == ["other"]
  assert roadmap.feature_candidates(fs, "m1", "missing") == []
}

fn editor(
  draft: Option(roadmap.Draft),
  expanded: List(String),
) -> roadmap.Editor(Nil) {
  roadmap.Editor(
    draft: draft,
    saving: False,
    targeting: False,
    error: "",
    expanded: expanded,
    on_toggle: fn(_) { Nil },
    on_new: Nil,
    on_edit: fn(_) { Nil },
    on_title: fn(_) { Nil },
    on_text: fn(_) { Nil },
    on_target_date: fn(_) { Nil },
    on_state: fn(_) { Nil },
    on_format: fn(_, _) { Nil },
    on_block: fn(_) { Nil },
    on_save: Nil,
    on_cancel: Nil,
    deleting: None,
    on_delete: fn(_) { Nil },
    on_confirm_delete: Nil,
    on_cancel_delete: Nil,
    on_target: fn(_, _) { Nil },
    feature_query: "",
    on_feature_query: fn(_) { Nil },
    on_depends: fn(_, _, _) { Nil },
    depends_open: None,
    depends_query: "",
    on_depends_open: fn(_, _) { Nil },
    on_depends_query: fn(_) { Nil },
  )
}

pub fn a_milestone_starts_collapsed_and_its_heading_opens_it_test() {
  let render = fn(expanded) {
    let elements =
      roadmap.view(
        "",
        [
          features.Milestone(
            "m1",
            "Launch",
            1.0,
            "Ship it",
            Some("2027-01-01"),
            "active",
          ),
          features.Milestone("m2", "Later", 2.0, "Second", None, "upcoming"),
        ],
        [feature("f1", "m1", 1.0), feature("f2", "m2", 1.0)],
        [task("t1", "f1", "m1"), task("t2", "f2", "m2")],
        fn(_, _) { Nil },
        editor(None, expanded),
        [],
      )
    element.to_string(html.div([], elements))
  }
  let closed = render([])
  assert string.contains(closed, "Launch")
  assert string.contains(closed, ">active<")
  assert string.contains(closed, "2027-01-01")
  assert string.contains(closed, "0 / 1 feature complete")
  assert !string.contains(closed, "1 / 1 feature complete")
  assert string.contains(closed, "Edit Launch")
  assert string.contains(closed, "Delete Launch")
  assert string.contains(closed, "aria-expanded=\"false\"")
  assert !string.contains(closed, "Ship it")
  assert !string.contains(closed, "<h4>f1")
  assert !string.contains(closed, ">t1</button>")
  let open = render(["m1"])
  assert string.contains(open, "aria-expanded=\"true\"")
  assert string.contains(open, "Ship it")
  assert string.contains(open, "<h4>f1")
  assert string.contains(open, ">t1</button>")
  assert !string.contains(open, "<h4>f2")
  assert !string.contains(open, ">t2</button>")
  assert !string.contains(open, "Second")
  let both = render(["m1", "m2"])
  assert string.contains(both, "<h4>f2")
  assert string.contains(both, ">t2</button>")
}

pub fn a_milestone_holds_the_features_that_target_it_test() {
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
      // b targets m2; t2's own m1 does not put b in m1 (D-188).
      roadmap.Group(Some(milestone("m1", 1.0)), [#(a, [t1, t6])], []),
      roadmap.Group(Some(milestone("m2", 2.0)), [#(b, [])], [t5]),
      // c's task with no milestone, and d, in no milestone at all.
      roadmap.Group(None, [#(c, [t4]), #(d, [])], []),
    ]
  // Nothing loose: no "No milestone" group.
  assert roadmap.groups([milestone("m1", 1.0)], [a], [t1])
    == [roadmap.Group(Some(milestone("m1", 1.0)), [#(a, [t1])], [])]
}

pub fn an_empty_target_date_shows_no_target_test() {
  let page =
    roadmap.view(
      "",
      [features.Milestone("m1", "Milestone", 1.0, "", Some(""), "proposed")],
      [],
      [],
      fn(_, _) { Nil },
      editor(None, []),
      [],
    )
  let html = element.to_string(html.div([], page))
  assert !string.contains(html, "class=\"target\"")
}

pub fn a_milestone_feature_search_names_the_saved_title_test() {
  let draft = roadmap.Draft(Some("m1"), "Edited title", "", "proposed", None)
  let page =
    roadmap.view(
      "",
      [features.Milestone("m1", "Launch", 1.0, "", None, "proposed")],
      [feature("f1", "m2", 1.0)],
      [],
      fn(_, _) { Nil },
      editor(Some(draft), []),
      [],
    )
  let html = element.to_string(html.div([], page))
  assert string.contains(html, "aria-label=\"Find a feature for Launch\"")
  assert !string.contains(
    html,
    "aria-label=\"Find a feature for Edited title\"",
  )
  assert string.contains(html, "class=\"picker\"")
}

fn feature_in_state(
  id: String,
  target: String,
  rank: Float,
  state: String,
) -> features.Feature {
  features.Feature(
    id,
    id,
    "",
    case target {
      "" -> None
      m -> Some(m)
    },
    rank,
    state,
    None,
  )
}

pub fn a_milestone_heading_counts_progress_and_leaves_canceled_features_out_test() {
  let render = fn(expanded) {
    let elements =
      roadmap.view(
        "",
        [milestone("m1", 1.0)],
        [
          feature_in_state("done-a", "m1", 1.0, "complete"),
          feature_in_state("done-b", "m1", 2.0, "complete"),
          feature_in_state("active", "m1", 3.0, "active"),
          feature_in_state("canceled", "m1", 4.0, "canceled"),
        ],
        [],
        fn(_, _) { Nil },
        editor(None, expanded),
        [],
      )
    element.to_string(html.div([], elements))
  }
  let closed = render([])
  assert string.contains(closed, "2 / 3 features complete")
  assert !string.contains(closed, "2 / 4 features complete")
  let open = render(["m1"])
  assert string.contains(open, "2 / 3 features complete")
  assert string.contains(open, "<h4>done-a")
  assert string.contains(open, "<h4>canceled")
}

pub fn milestone_heading_progress_handles_all_none_one_and_zero_features_test() {
  let elements =
    roadmap.view(
      "",
      [
        milestone("all", 1.0),
        milestone("none", 2.0),
        milestone("one", 3.0),
        milestone("zero", 4.0),
      ],
      [
        feature_in_state("all-a", "all", 1.0, "complete"),
        feature_in_state("all-b", "all", 2.0, "complete"),
        feature_in_state("none-a", "none", 1.0, "proposed"),
        feature_in_state("none-b", "none", 2.0, "active"),
        feature_in_state("one-a", "one", 1.0, "complete"),
      ],
      [],
      fn(_, _) { Nil },
      editor(None, []),
      [],
    )
  let page = element.to_string(html.div([], elements))
  let all = milestone_section(page, "all")
  assert string.contains(all, "2 / 2 features complete")
  let none = milestone_section(page, "none")
  assert string.contains(none, "0 / 2 features complete")
  let one = milestone_section(page, "one")
  assert string.contains(one, "1 / 1 feature complete")
  let zero = milestone_section(page, "zero")
  assert string.contains(zero, "0 / 0 features complete")
}

fn milestone_section(page: String, id: String) -> String {
  let marker = "data-milestone-id=\"" <> id <> "\""
  let assert Ok(#(_, after)) = string.split_once(page, marker)
  let assert Ok(#(section, _)) = string.split_once(after, "</section>")
  section
}

pub fn milestone_heading_groups_its_state_date_and_count_test() {
  let elements =
    roadmap.view(
      "",
      [dated_milestone("m1", 1.0, "2027-01-01")],
      [feature_in_state("done", "m1", 1.0, "complete")],
      [],
      fn(_, _) { Nil },
      editor(None, []),
      [],
    )
  let page = element.to_string(html.div([], elements))
  let section = milestone_section(page, "m1")
  let assert Ok(#(_, after)) =
    string.split_once(section, "<span class=\"facts\">")
  let assert Ok(#(facts, _)) = string.split_once(after, "</span></button>")
  assert string.contains(
    facts,
    "<span class=\"state state-proposed\">proposed</span>",
  )
  assert string.contains(facts, "<span class=\"target\">2027-01-01</span>")
  assert string.contains(
    facts,
    "<span class=\"feature-count\">1 / 1 feature complete</span>",
  )
}
