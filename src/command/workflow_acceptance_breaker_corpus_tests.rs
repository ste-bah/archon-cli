//! Exact replies from the configured provider, including its invalid replies.
use super::tests::fixture;
use super::*;
fn recorded(name: &str) -> String {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/breaker_reply_corpus");
    let raw = std::fs::read_to_string(dir.join(format!("{name}.txt"))).unwrap();
    let sources: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("sources.json")).unwrap()).unwrap();
    assert_eq!(sources["status"], "recorded");
    let source = sources["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["case"] == name)
        .unwrap();
    assert!(source["resolved_model"].as_str().is_some());
    let transcript: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.join(source["transcript_path"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    assert_eq!(transcript["reply"], raw);
    raw
}
fn proposal_reply(name: &str, input: &str, gut: bool) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/breaker_reply_corpus");
    let data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join(format!("{input}.input.json"))).unwrap())
            .unwrap();
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
    view.files.clear();
    for (key, value) in data["files"].as_object().unwrap() {
        view.files.insert(
            key.clone(),
            super::super::super::breaker_view::ViewFile {
                object: String::new(),
                bytes: value.as_str().unwrap().as_bytes().to_vec(),
            },
        );
    }
    let raw = recorded(name);
    let result = if gut {
        prompt::parse_gut(
            &raw,
            "task",
            data["id"].as_str().unwrap(),
            data["text"].as_str().unwrap(),
            &view,
            view.files.keys().next().unwrap(),
        )
    } else {
        prompt::parse(
            &raw,
            "task",
            data["id"].as_str().unwrap(),
            data["text"].as_str().unwrap(),
            &view,
        )
    };
    if name == "breaker_r18_attempt3" {
        assert_eq!(
            result.err().unwrap(),
            "invalid_reply: EOF while parsing an object at line 1 column 2286"
        );
    } else if name == "breaker_r18_round5_attempt1_gut1" {
        assert!(result.err().unwrap().contains("patch_empty"));
    } else {
        let parsed = result.unwrap();
        assert!(!parsed.mutants.is_empty());
        if gut {
            assert_eq!(parsed.mutants.len(), 1);
        } else if name.contains("round5") {
            assert_eq!(parsed.mutants.len(), if input == "breaker" { 5 } else { 2 });
        } else if name.contains("round4") {
            assert_eq!(parsed.mutants.len(), if input == "breaker" { 4 } else { 5 });
        } else if input == "breaker" {
            assert_eq!(parsed.mutants.len(), 2);
        }
        for (mutant, breaks) in parsed.mutants {
            assert_eq!(
                mutant.kind,
                if gut {
                    MutantKind::BodyGut
                } else {
                    MutantKind::Behaviour
                }
            );
            if !gut {
                assert!(rule::verified_quote(
                    data["text"].as_str().unwrap(),
                    &breaks
                ));
            }
        }
    }
}
#[test]
fn recorded_toy_behaviour() {
    proposal_reply("breaker_attempt1", "breaker", false);
}
#[test]
fn recorded_toy_body_gut() {
    proposal_reply("breaker_attempt1_gut1", "breaker", true);
}
#[test]
fn recorded_deliverable_behaviour_attempt_one() {
    proposal_reply("breaker_r18_attempt1", "breaker_r18", false);
}
#[test]
fn recorded_deliverable_body_gut_attempt_one() {
    proposal_reply("breaker_r18_attempt1_gut1", "breaker_r18", true);
}
#[test]
fn recorded_deliverable_behaviour_attempt_two() {
    proposal_reply("breaker_r18_attempt2", "breaker_r18", false);
}
#[test]
fn recorded_deliverable_body_gut_attempt_two() {
    proposal_reply("breaker_r18_attempt2_gut1", "breaker_r18", true);
}
#[test]
fn recorded_deliverable_behaviour_attempt_three() {
    proposal_reply("breaker_r18_attempt3", "breaker_r18", false);
}
#[test]
fn recorded_deliverable_body_gut_attempt_three() {
    proposal_reply("breaker_r18_attempt3_gut1", "breaker_r18", true);
}

fn judge_reply(name: &str, role: &str) {
    let raw = recorded(name);
    for reply in [raw.clone(), format!("````json\r\n{raw}\r\n````\r\n")] {
        let object = prompt::object(&reply).unwrap();
        let expected = match name {
            "not_relevant_attempt1" => "relevant",
            "not_equivalent_attempt1" => "equivalent",
            other => other,
        };
        assert_eq!(object["verdict"], expected);
        assert!(judges::valid_answer(role, &object));
        assert!(object["reason"].as_str().is_some());
    }
    assert!(prompt::object(&raw[..raw.len() - 1]).is_err());
    let mut inner = prompt::object(&raw).unwrap();
    inner["reason"] = format!("{}\n```\n", inner["reason"].as_str().unwrap()).into();
    assert_eq!(
        prompt::object(&format!("````json\n{inner}\n````")).unwrap(),
        inner
    );
}
#[test]
fn recorded_relevance_accepts_the_stated_phrase() {
    judge_reply("relevant", "breaker_relevance");
}
#[test]
fn recorded_irrelevance_accepts_the_load_failure() {
    judge_reply("not_relevant", "breaker_relevance");
}
#[test]
fn recorded_equivalence_accepts_equal_output() {
    judge_reply("equivalent", "breaker_equivalence");
}
#[test]
fn recorded_non_equivalence_accepts_different_output() {
    judge_reply("not_equivalent", "breaker_equivalence");
}

#[test]
fn recorded_first_negative_stimulus_changed_the_requirement() {
    judge_reply("not_relevant_attempt1", "breaker_relevance");
}
#[test]
fn recorded_first_changed_constant_was_incorrectly_called_equivalent() {
    judge_reply("not_equivalent_attempt1", "breaker_equivalence");
}

#[test]
fn recorded_pair_replay_accepts_behaviour_and_each_body_gut() {
    for attempt in 1..=2 {
        proposal_reply(
            &format!("breaker_r18_attempt{attempt}"),
            "breaker_r18",
            false,
        );
    }
    for attempt in 1..=3 {
        proposal_reply(
            &format!("breaker_r18_attempt{attempt}_gut1"),
            "breaker_r18",
            true,
        );
    }
    proposal_reply("breaker_r18_attempt3", "breaker_r18", false);
}

#[test]
fn recorded_round_four_equivalence_misjudged_equal_observed_output() {
    let value = prompt::object(&recorded("equivalent_round4")).unwrap();
    assert_eq!(value["verdict"], "not_equivalent");
    let sources: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/breaker_reply_corpus/sources.json"
    ))
    .unwrap();
    let source = sources["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["case"] == "equivalent_round4")
        .unwrap();
    assert_eq!(
        source["observation"]["original_stdout"],
        source["observation"]["changed_stdout"]
    );
    assert_eq!(source["observation"]["original_exit"], 0);
    assert_eq!(source["observation"]["changed_exit"], 0);
}

#[test]
fn recorded_round_four_pairs_meet_the_bar() {
    for (name, input) in [
        ("breaker_round4_attempt1", "breaker"),
        ("breaker_r18_round4_attempt1", "breaker_r18"),
    ] {
        proposal_reply(name, input, false);
        proposal_reply(&format!("{name}_gut1"), input, true);
    }
    for name in ["breaker", "breaker_gut", "breaker_r18", "breaker_r18_gut"] {
        recorded(name);
    }
    for (name, role) in [
        ("relevant_round4", "breaker_relevance"),
        ("not_relevant_round4", "breaker_relevance"),
        ("not_equivalent_round4", "breaker_equivalence"),
    ] {
        assert!(judges::valid_answer(
            role,
            &prompt::object(&recorded(name)).unwrap()
        ));
    }
}
#[test]
fn quote_ignores_final_sentence_punctuation() {
    let data: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/breaker_reply_corpus/breaker_r18.input.json"
    ))
    .unwrap();
    let text = data["text"].as_str().unwrap();
    assert!(rule::verified_quote(text, text.trim_end_matches('.')));
}

#[test]
fn recorded_round_five_pairs_and_literal_gut_refusal() {
    proposal_reply("breaker_round5_attempt1", "breaker", false);
    proposal_reply("breaker_round5_attempt1_gut1", "breaker", true);
    proposal_reply("breaker_r18_round5_attempt1", "breaker_r18", false);
    proposal_reply("breaker_r18_round5_attempt1_gut1", "breaker_r18", true);
    proposal_reply("breaker_r18_round5_attempt1_gut2", "breaker_r18", true);
}

#[test]
fn recorded_round_five_judge_replies_parse_with_exact_verdicts() {
    for (name, role, expected) in [
        ("relevant_round5", "breaker_relevance", "relevant"),
        ("not_relevant_round5", "breaker_relevance", "not_relevant"),
        ("equivalent_round5", "breaker_equivalence", "equivalent"),
        (
            "not_equivalent_round5",
            "breaker_equivalence",
            "not_equivalent",
        ),
    ] {
        let value = prompt::object(&recorded(name)).unwrap();
        assert!(judges::valid_answer(role, &value));
        assert_eq!(value["verdict"], expected);
    }
}
