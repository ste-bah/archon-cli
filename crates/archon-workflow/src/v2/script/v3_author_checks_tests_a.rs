use super::*;

#[test]
fn map_reduce_review_rejects_reviews_before_task_work() {
    let expected = task_set(["TASK-EX-001"]);
    let mut details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    details.calls.push(work_call("verify-task-late"));
    let error =
        validate_map_reduce_review_calls(&details, &expected).expect_err("late work rejected");
    assert!(error.contains("BEFORE task work"), "{error}");
}

#[test]
fn legacy_monolithic_reviews_no_longer_satisfy_mandate() {
    let expected = task_set(["TASK-EX-001"]);
    let details = WorkflowDryRunPlanDetails {
        calls: vec![
            work_call("implement-task-1"),
            agent_call("adversarial-review-2", Some("critic")),
            agent_call("coverage-audit-3", Some("critic")),
        ],
        ..Default::default()
    };
    let error = validate_map_reduce_review_calls(&details, &expected).expect_err("legacy rejected");
    assert!(error.contains("legacy monolithic review"), "{error}");
    assert!(error.contains("map→reduce"), "{error}");
}

#[test]
fn reducer_bound_accounting_accepts_preserved_map_findings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    save_review_record(
        &store,
        "adversarial-review-map",
        serde_json::json!(["map finding"]),
    );
    save_review_record(
        &store,
        "adversarial-review-reduce",
        serde_json::json!(["map finding", "cross finding"]),
    );
    save_review_record(&store, "coverage-audit-map", serde_json::json!([]));
    save_review_record(
        &store,
        "coverage-audit-reduce",
        serde_json::json!(["coverage gap"]),
    );
    let accounting = serde_json::json!({
        "accepted": ["TASK-EX-001"],
        "blocked": [],
        "adversarial_findings": ["map finding", "cross finding"],
        "uncovered_requirements": ["coverage gap"],
    })
    .to_string();

    validate_review_accounting_from_reducers(Some(&accounting), &details, &store)
        .expect("reducer-bound accounting passes");
}

#[test]
fn reducer_bound_accounting_rejects_dropped_map_findings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    save_review_record(
        &store,
        "adversarial-review-map",
        serde_json::json!(["map finding"]),
    );
    save_review_record(&store, "adversarial-review-reduce", serde_json::json!([]));
    save_review_record(&store, "coverage-audit-map", serde_json::json!([]));
    save_review_record(&store, "coverage-audit-reduce", serde_json::json!([]));
    let accounting = serde_json::json!({
        "accepted": ["TASK-EX-001"],
        "blocked": [],
        "adversarial_findings": [],
        "uncovered_requirements": [],
    })
    .to_string();

    let error = validate_review_accounting_from_reducers(Some(&accounting), &details, &store)
        .expect_err("dropped map finding rejected")
        .to_string();
    assert!(error.contains("dropped map findings"), "{error}");
}

#[test]
fn reducer_bound_accounting_rejects_accounting_that_drops_reduce_findings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    save_review_record(
        &store,
        "adversarial-review-map",
        serde_json::json!(["map finding"]),
    );
    save_review_record(
        &store,
        "adversarial-review-reduce",
        serde_json::json!(["map finding", "cross finding"]),
    );
    save_review_record(&store, "coverage-audit-map", serde_json::json!([]));
    save_review_record(&store, "coverage-audit-reduce", serde_json::json!([]));
    let accounting = serde_json::json!({
        "accepted": ["TASK-EX-001"],
        "blocked": [],
        "adversarial_findings": ["map finding"],
        "uncovered_requirements": [],
    })
    .to_string();

    let error = validate_review_accounting_from_reducers(Some(&accounting), &details, &store)
        .expect_err("accounting must match reduce")
        .to_string();
    assert!(error.contains("does not match final reducer"), "{error}");
}

#[test]
fn reference_and_validator_share_the_mandate_contract() {
    for field in MANDATED_RESULT_FIELDS {
        assert!(
            V3_PRIMITIVE_REFERENCE.contains(field),
            "reference must document the {field} return field"
        );
    }
    for token in [
        "reviewContract",
        "adversarial_findings",
        "uncovered_requirements",
        "reduce_final",
        "preserveMapFindings",
        "maxInputBytes",
        "canonical_task_ids",
    ] {
        assert!(
            V3_PRIMITIVE_REFERENCE.contains(token),
            "reference must document mandatory review contract token {token}"
        );
    }
    assert!(
        V3_PRIMITIVE_REFERENCE.contains("'critic'   // 'critic' routes"),
        "the tier enum must include critic"
    );
}

pub(super) fn agent_call(id: &str, role: Option<&str>) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: WorkflowV2HostOptions {
            role: role.map(str::to_string),
            ..Default::default()
        },
    }
}

pub(super) fn work_call(id: &str) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions::default(),
    }
}

pub(super) fn task_set<const N: usize>(ids: [&str; N]) -> std::collections::BTreeSet<String> {
    ids.into_iter().map(str::to_string).collect()
}

pub(super) fn save_review_record(
    store: &WorkflowV2ResultStore,
    call_id: &str,
    findings: serde_json::Value,
) {
    let mut result = WorkflowV2Result::accepted("review complete");
    result.data = serde_json::json!({ "findings": findings });
    let record = WorkflowV2CallRecord::new(
        store.run_id(),
        WorkflowV2HostCall {
            id: call_id.to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        1,
        String::new(),
        result,
        Vec::new(),
    );
    store.save_call_record(&record).expect("save review record");
}

pub(super) fn review_details(
    mut calls: Vec<WorkflowV2HostCall>,
    review_map_claims: Vec<WorkflowReviewMapClaim>,
    review_reduce_edges: Vec<WorkflowReviewReduceEdge>,
) -> WorkflowDryRunPlanDetails {
    let mut review_calls = std::collections::BTreeMap::new();
    for claim in &review_map_claims {
        review_calls
            .entry(claim.call_id.clone())
            .or_insert_with(|| review_map_call(&claim.call_id, &claim.review_kind));
    }
    for edge in &review_reduce_edges {
        review_calls
            .entry(edge.call_id.clone())
            .or_insert_with(|| review_reduce_call(&edge.call_id, &edge.review_kind, &edge.stage));
    }
    calls.extend(review_calls.into_values());
    WorkflowDryRunPlanDetails {
        calls,
        write_task_claims: Vec::new(),
        review_map_claims,
        review_reduce_edges,
    }
}

pub(super) fn review_map_claim(
    review_kind: &str,
    call_id: &str,
    task_id: &str,
) -> WorkflowReviewMapClaim {
    WorkflowReviewMapClaim {
        review_kind: review_kind.to_string(),
        call_id: call_id.to_string(),
        item_id: Some(format!("review-{}", task_id.to_ascii_lowercase())),
        task_ids: vec![task_id.to_string()],
    }
}

pub(super) fn review_reduce<const M: usize, const R: usize>(
    review_kind: &str,
    call_id: &str,
    accounting_field: &str,
    source_map_call_ids: [&str; M],
    source_reduce_call_ids: [&str; R],
) -> WorkflowReviewReduceEdge {
    WorkflowReviewReduceEdge {
        review_kind: review_kind.to_string(),
        call_id: call_id.to_string(),
        stage: REVIEW_REDUCE_FINAL_STAGE.to_string(),
        accounting_field: Some(accounting_field.to_string()),
        source_map_call_ids: source_map_call_ids
            .into_iter()
            .map(str::to_string)
            .collect(),
        source_reduce_call_ids: source_reduce_call_ids
            .into_iter()
            .map(str::to_string)
            .collect(),
        preserve_map_findings: true,
        max_input_bytes: Some(48_000),
        max_findings_per_reduce: None,
    }
}

pub(super) fn review_map_call(call_id: &str, review_kind: &str) -> WorkflowV2HostCall {
    let mut extra = std::collections::BTreeMap::new();
    extra.insert(
        "reviewContract".to_string(),
        serde_json::json!({
            "version": 1,
            "kind": review_kind,
            "stage": REVIEW_MAP_STAGE,
            "findingsPath": "data.findings",
            "maxFindingsPerItem": 25,
        }),
    );
    WorkflowV2HostCall {
        id: call_id.to_string(),
        method: WorkflowV2HostMethod::Parallel,
        write_mode: None,
        options: WorkflowV2HostOptions {
            role: Some(CRITIC_TIER.to_string()),
            item_kind: Some("review_map".to_string()),
            extra,
            ..Default::default()
        },
    }
}

pub(super) fn review_reduce_call(
    call_id: &str,
    review_kind: &str,
    stage: &str,
) -> WorkflowV2HostCall {
    let mut extra = std::collections::BTreeMap::new();
    extra.insert(
        "reviewContract".to_string(),
        serde_json::json!({
            "version": 1,
            "kind": review_kind,
            "stage": stage,
            "preserveMapFindings": true,
            "maxInputBytes": 48000,
        }),
    );
    WorkflowV2HostCall {
        id: call_id.to_string(),
        method: WorkflowV2HostMethod::Reduce,
        write_mode: None,
        options: WorkflowV2HostOptions {
            role: Some(CRITIC_TIER.to_string()),
            extra,
            ..Default::default()
        },
    }
}

/// Work a mandatory review ASKED for, and which is re-verified afterwards, is
/// categorically different from work smuggled in after the reviewers looked.
/// The ordering rule must allow the first without allowing the second.
#[test]
fn post_review_remediation_shape_is_accepted_by_the_mandate() {
    let details = remediation_details(vec![
        remediation_call(
            "review-remediate-task-ex-001-1",
            "remediate",
            "TASK-EX-001",
            1,
            2,
        ),
        remediation_call(
            "verification-wave-review-verify-task-ex-001-1",
            "verify",
            "TASK-EX-001",
            1,
            2,
        ),
    ]);
    let result = validate_map_reduce_review_calls(&details, &task_set(["TASK-EX-001"]));
    assert!(
        result.is_ok(),
        "review-driven remediation must be allowed: {}",
        result.unwrap_err()
    );
}
