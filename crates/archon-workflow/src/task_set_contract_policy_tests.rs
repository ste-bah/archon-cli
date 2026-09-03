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
    assert!(criterion_prescribes_check_shape("Freeze as a commandless floor."));
}

/// Outcome language leaves the shape to the author. Every acceptance row of a
/// real PRD reads like this, and none of them may be treated as prescribed.
#[test]
fn outcome_language_does_not_prescribe_the_shape() {
    for criterion in [
        "`trading data status` shows the existing project data root and registry.",
        "Coverage matrix reports all required instruments and timeframes.",
        "Derived datasets are marked non-production and rejected by production backtests.",
        "A provider capability command reports exact native support per provider/symbol/timeframe.",
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
        assert_eq!(back[token], value, "`{token}` is not a contract field the engine reads");
    }
}
