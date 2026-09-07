use super::*;

/// The live failure: a `]` written where the open object still needed its
/// `}`. One reading, so the closer is inserted and the document parses.
#[test]
fn a_closer_written_early_is_completed_with_the_one_the_document_demands() {
    let broken =
        r#"{"status":"accepted","residual_gaps":[{"id":"g","description":"a \"quoted\" ] brace"]}"#;
    let repaired = repair_mismatched_closers(broken).expect("repairable");
    let value: Value = serde_json::from_str(&repaired).expect("parses after repair");
    assert_eq!(value["residual_gaps"][0]["id"], "g");
    assert_eq!(
        value["residual_gaps"][0]["description"], "a \"quoted\" ] brace",
        "brackets inside strings are never touched"
    );
}

/// A reply that stops mid-value may be missing content: never completed.
#[test]
fn a_truncated_reply_is_not_completed() {
    assert!(repair_mismatched_closers(r#"{"status":"accepted","summary":"cut off"#).is_none());
    assert!(
        repair_mismatched_closers(r#"{"status":"accepted","evidence":[{"kind":"other"#).is_none()
    );
    assert!(
        repair_mismatched_closers(r#"{"status":"accepted"}"#).is_none(),
        "valid input: nothing to do"
    );
    assert!(
        repair_mismatched_closers(r#"{"status":"accepted"]"#).is_none(),
        "a closer nothing opened is not a repair"
    );
}

/// A gap without an id gets one minted from its description; existing ids
/// are untouched.
#[test]
fn a_residual_gap_without_an_id_is_named_from_its_description() {
    let mut object = serde_json::json!({
        "residual_gaps": [
            {"description": "Verifier could not run: no shell.", "severity": "low"},
            {"id": "keep-me", "description": "x"},
            {"id": "  ", "description": ""}
        ]
    });
    stamp_residual_gap_ids(object.as_object_mut().unwrap());
    let gaps = object["residual_gaps"].as_array().unwrap();
    assert_eq!(gaps[0]["id"], "gap-1-verifier-could-not-run-no-shell");
    assert_eq!(gaps[1]["id"], "keep-me");
    assert_eq!(gaps[2]["id"], "gap-3-gap");
}

/// An artifact without a path leaves the evidence list and becomes a visible
/// note; artifacts with paths are untouched.
#[test]
fn an_artifact_without_a_path_becomes_a_note_not_a_rejection() {
    let mut object = serde_json::json!({
        "artifacts": [
            {"id": "report", "description": "the audit report"},
            {"id": "real", "path": "artifacts/real.json"}
        ]
    });
    credit_pathless_artifacts_as_evidence(object.as_object_mut().unwrap());
    let artifacts = object["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0]["id"], "real");
    assert!(
        object.get("evidence").is_none(),
        "no agent evidence, no host note: {object}"
    );

    let mut with_evidence = serde_json::json!({
        "evidence": [{"kind": "inspection", "summary": "read it"}],
        "artifacts": [{"id": "report", "description": "the audit report"}]
    });
    credit_pathless_artifacts_as_evidence(with_evidence.as_object_mut().unwrap());
    let evidence = with_evidence["evidence"].as_array().unwrap();
    assert_eq!(evidence.len(), 2);
    assert!(
        evidence[1]["summary"]
            .as_str()
            .unwrap()
            .contains("the audit report")
    );
}

/// Live shapes from wf-8f52cffc and wf-16a20426: a complete envelope one `}`
/// short, and a repair reply that wrote `\'` inside a JSON string.
#[test]
fn a_reply_one_closer_short_is_completed_and_a_truncated_one_is_not() {
    let short = r#"{"status":"accepted","summary":"done","data":{"items":[{"id":"a"}]}"#;
    let fixed = complete_missing_closers(short).expect("one closer owed");
    assert!(serde_json::from_str::<serde_json::Value>(&fixed).is_ok());
    assert!(fixed.ends_with("}]}}"));
    let mid_string = r#"{"status":"accepted","summary":"cut off he"#;
    assert!(complete_missing_closers(mid_string).is_none());
    let mid_literal = r#"{"status":"accepted","count":12"#;
    assert!(complete_missing_closers(mid_literal).is_none());
    let complete = r#"{"status":"accepted"}"#;
    assert!(complete_missing_closers(complete).is_none());
}

#[test]
fn a_single_quote_escape_is_unescaped_and_real_escapes_are_kept() {
    let bad = r#"{"summary":"assert d[\'x\'] == 1 and path \"p\" and back\\slash"}"#;
    let fixed = unescape_single_quotes(bad).expect("changed");
    let value: serde_json::Value = serde_json::from_str(&fixed).expect("valid after repair");
    assert_eq!(
        value["summary"],
        "assert d['x'] == 1 and path \"p\" and back\\slash"
    );
    assert!(unescape_single_quotes(r#"{"summary":"fine"}"#).is_none());
}
