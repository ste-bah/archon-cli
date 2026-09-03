use super::*;

/// The synthetic proof fixture prescribes its floor field by field; that text
/// must read as prescribing, or the proof's byte-identical floor assertion and
/// the repair loop would fight each other.
#[test]
fn a_criterion_naming_contract_fields_prescribes_the_shape() {
    let fixture = "The project artifact `.archon/proof/x.json` exists. Freeze this criterion as a \
        commandless floor with `kind=\"x\"`, `artifact_path=\".archon/proof/x.json\"`, \
        `artifact_format=\"json\"`, `required_true_fields=[\"ready\"]`, and no `typed_verifier_command`.";
    assert!(criterion_prescribes_check_shape(fixture));
    assert!(criterion_prescribes_check_shape(
        "Freeze as a commandless floor."
    ));
}

/// Outcome language leaves the shape to the author. Every acceptance row of a
/// real PRD reads like this, and none of them may be treated as prescribed.
#[test]
fn outcome_language_does_not_prescribe_the_shape() {
    for criterion in [
        "`tool status` shows the existing project data root and catalogue.",
        "The coverage report lists every required source and interval.",
        "Derived outputs are marked non-production and rejected by production consumers.",
        "A capability command reports exact native support per source and interval.",
    ] {
        assert!(!criterion_prescribes_check_shape(criterion), "{criterion}");
    }
}

/// Every field token in the vocabulary must be a real field of the contract
/// type: a value set under that key must survive a deserialize/serialize round
/// trip. A misspelt or removed field would otherwise silently stop counting.
#[test]
fn the_vocabulary_names_real_contract_fields() {
    for token in CHECK_SHAPE_VOCABULARY {
        if token.contains(' ') {
            continue; // the prose phrase, not a field
        }
        let value = match token {
            "min_instances" => serde_json::json!(3),
            "required_true_fields" => serde_json::json!(["ready"]),
            _ => serde_json::json!("probe"),
        };
        let mut object = serde_json::json!({"kind": "k", "artifact_path": "p"});
        object[token] = value.clone();
        let contract: crate::task_universe::WorkflowV2DeliverableContract =
            serde_json::from_value(object).expect(token);
        let back = serde_json::to_value(&contract).unwrap();
        assert_eq!(
            back[token], value,
            "`{token}` is not a contract field the engine reads"
        );
    }
}

/// The judge's reason and counterexample must reach the author through the
/// finding text, on one line, because the repair prompt carries nothing else.
#[test]
fn a_refuted_check_finding_carries_the_judge_reason_and_counterexample() {
    let message = refuted_check_message(
        "AC-1",
        "only asserts a substring\nof stdout",
        "an empty  artifact file\twith the substring",
    );
    assert!(
        !message.contains('\n') && !message.contains('\t'),
        "{message}"
    );
    assert!(message.starts_with("check 'AC-1' was refuted by the host judge; reason: \"only asserts a substring of stdout\"; counterexample: \"an empty artifact file with the substring\";"), "{message}");
    assert!(message.ends_with("replace the check with one that fails in that state"));
}

/// A judge that returned blank prose still yields a finding that says so
/// instead of an empty clause the author would read as noise.
#[test]
fn a_refuted_check_with_blank_judge_prose_says_so() {
    let message = refuted_check_message("AC-2", "  ", "");
    assert!(message.contains("reason: \"(judge gave no reason)\"; counterexample: \"(judge gave no counterexample)\";"), "{message}");
}

/// The repair prompt repeats every earlier attempt's findings, so judge prose
/// is capped; the cut lands on a character boundary and is marked.
#[test]
fn judge_prose_is_capped() {
    let long = "é".repeat(JUDGE_PROSE_CAP + 50);
    let message = refuted_check_message("AC-3", &long, "x");
    let expected = format!("reason: \"{}…\"", "é".repeat(JUDGE_PROSE_CAP));
    assert!(message.contains(&expected), "{message}");
}
