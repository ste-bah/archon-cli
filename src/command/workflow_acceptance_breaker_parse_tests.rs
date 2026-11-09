use super::tests::fixture;
use super::*;
pub(super) fn proposals() -> serde_json::Value {
    serde_json::json!({"mutants":[
        {"kind":"behaviour","breaks":"keeps the good value","edits":[{"key":"value","find":"good","replace":"bad"}]},
        {"kind":"behaviour","breaks":"keeps the good value","edits":[{"key":"value","find":"good","replace":"wrong"}]},
        {"kind":"body_gut","breaks":"","edits":[{"key":"value","find":"printf good;","replace":":"}]}
    ]})
}
#[test]
fn counts_unknown_kinds_and_normalised_quotes() {
    let f = fixture("unused");
    let view = materialise_view(
        f.parent.path(),
        f.source.path(),
        f.snapshot(),
        &[(
            "value".into(),
            archon_workflow::base_snapshot::SnapshotKind::Blob,
        )],
        &[],
    )
    .unwrap();
    let raw = proposals();
    for behaviours in [0, 1, 6] {
        let mut v = raw.clone();
        let b = v["mutants"][0].clone();
        let gut = v["mutants"][2].clone();
        v["mutants"] =
            serde_json::Value::Array(std::iter::repeat_n(b, behaviours).chain([gut]).collect());
        let parsed = prompt::parse(
            &v.to_string(),
            "task",
            "item",
            "keeps the good value",
            &view,
        )
        .unwrap();
        let expected = if v["mutants"].as_array().unwrap().len() > 5 {
            6
        } else {
            v["mutants"].as_array().unwrap().len()
        };
        assert_eq!(parsed.mutants.len(), expected);
        assert_eq!(
            parsed.refusals.len(),
            v["mutants"].as_array().unwrap().len() - expected
        );
    }
    for guts in [0, 2] {
        let mut v = raw.clone();
        let gut = v["mutants"][2].clone();
        v["mutants"] = serde_json::Value::Array(
            vec![v["mutants"][0].clone(), v["mutants"][1].clone()]
                .into_iter()
                .chain(std::iter::repeat_n(gut, guts))
                .collect(),
        );
        let parsed = prompt::parse(
            &v.to_string(),
            "task",
            "item",
            "keeps the good value",
            &view,
        )
        .unwrap();
        assert_eq!(parsed.mutants.len(), 2 + guts.min(1));
        assert_eq!(parsed.refusals.len(), guts.saturating_sub(1));
    }
    let mut v = raw.clone();
    let b = v["mutants"][0].clone();
    let gut = v["mutants"][2].clone();
    v["mutants"] = serde_json::Value::Array(std::iter::repeat_n(b, 4).chain([gut]).collect());
    v["mutants"][0]["breaks"] = "KEEPS\u{2003}the\nGOOD value".into();
    assert_eq!(
        prompt::parse(
            &format!("Preamble\n````json\n{v}\n````\nAfter"),
            "task",
            "item",
            "keeps the good value",
            &view
        )
        .unwrap()
        .mutants
        .len(),
        5
    );
}
#[test]
fn replacement_fences_newlines_and_overlaps_are_preserved_or_refused() {
    let f = fixture("unused");
    let mut view = materialise_view(
        f.parent.path(),
        f.source.path(),
        f.snapshot(),
        &[(
            "value".into(),
            archon_workflow::base_snapshot::SnapshotKind::Blob,
        )],
        &[],
    )
    .unwrap();
    let mut v = proposals();
    v["mutants"][0]["edits"][0]["replace"] = "```\r\nchanged".into();
    let parsed = prompt::parse(
        &format!("````json\r\n{v}\r\n````\r\n"),
        "task",
        "item",
        "keeps the good value",
        &view,
    )
    .unwrap();
    assert!(
        parsed.mutants[0]
            .0
            .patch
            .contains("+value() { printf ```\r\n+changed; }\n")
    );
    view.files.get_mut("value").unwrap().bytes = b"aaa".to_vec();
    v["mutants"][0]["edits"][0]["find"] = "aa".into();
    v["mutants"][1]["edits"][0]["find"] = "aaa".into();
    let parsed = prompt::parse(
        &v.to_string(),
        "task",
        "item",
        "keeps the good value",
        &view,
    )
    .unwrap();
    assert!(
        parsed
            .refusals
            .iter()
            .any(|r| r.starts_with("find_ambiguous:"))
    );
}

#[test]
fn invalid_proposals_do_not_consume_valid_proposal_limits() {
    let f = fixture("unused");
    let view = materialise_view(
        f.parent.path(),
        f.source.path(),
        f.snapshot(),
        &[(
            "value".into(),
            archon_workflow::base_snapshot::SnapshotKind::Blob,
        )],
        &[],
    )
    .unwrap();
    let raw = proposals();
    let mut invalid = raw["mutants"][0].clone();
    invalid.as_object_mut().unwrap().remove("breaks");
    let values: Vec<_> = std::iter::once(invalid)
        .chain(std::iter::repeat_n(raw["mutants"][0].clone(), 6))
        .chain([raw["mutants"][2].clone()])
        .collect();
    let parsed = prompt::parse(
        &serde_json::json!({"mutants":values}).to_string(),
        "task",
        "item",
        "keeps the good value",
        &view,
    )
    .unwrap();
    assert_eq!(parsed.mutants.len(), 6);
    assert_eq!(parsed.refusals.len(), 2);
    assert!(parsed.refusals[0].contains("missing field `breaks`"));
    assert!(parsed.refusals[1].starts_with("proposal_over_limit:"));
}
