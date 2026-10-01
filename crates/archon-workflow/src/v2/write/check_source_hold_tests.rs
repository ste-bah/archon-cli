use super::*;

fn request() -> SourceChangeRequest {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1, "request_id": "csr-1", "origin": "landing",
        "check_ids": ["AC-1"], "root": "repository", "path": "tests/it.rs",
        "was_pinned": true, "pinned_digest": null, "proposed_digest": null,
        "created_at": "2026-01-01T00:00:00Z"
    }))
    .unwrap()
}

/// Review minor 1: the held-change gap never says the rest landed when the
/// branch was refused after the hold.
#[test]
fn the_held_gap_says_what_happened_to_the_rest_of_the_branch() {
    let hold = Hold {
        held: vec![request()],
        ..Hold::default()
    };
    let mut refused = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        ..WorkflowV2Result::default()
    };
    report_check_source_holds(&mut refused, "w-0", &hold);
    let text = &refused.residual_gaps[0].description;
    assert!(
        !text.contains("goes on to land") && !text.contains("landed as usual"),
        "{text}"
    );
    assert!(text.contains("nothing of it lands"), "{text}");
    let mut accepted = WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        ..WorkflowV2Result::default()
    };
    report_check_source_holds(&mut accepted, "w-0", &hold);
    assert!(
        accepted.residual_gaps[0]
            .description
            .contains("goes on to land")
    );
}

/// Review minor 10: a run whose launch record names a task set, but whose
/// landing policy cannot be read, refuses its landings rather than landing
/// them unpinned; a run that names none holds nothing.
#[test]
fn a_run_naming_a_task_set_without_a_readable_policy_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let run = dir.path().join("run");
    assert!(
        landing_policy(&run).unwrap().is_none(),
        "no record: nothing to pin"
    );
    std::fs::create_dir_all(run.join("v2")).unwrap();
    std::fs::write(
        run.join("v2/generated-metadata.json"),
        r#"{"observer_snapshot": {"canonical_task_root_identity": "/nowhere/tasks"}}"#,
    )
    .unwrap();
    assert!(
        landing_policy(&run)
            .unwrap_err()
            .contains("names the run's task set")
    );
    std::fs::write(run.join("v2/generated-metadata.json"), "not json").unwrap();
    assert!(landing_policy(&run).unwrap_err().contains("malformed"));
}
