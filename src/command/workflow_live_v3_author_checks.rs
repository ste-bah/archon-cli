/// Dry-run pre-flight: execute the authored script against the recording stub
/// host and require it to PLAN real work — with every universe task claimed
/// by EXACTLY ONE write call, mandatory map→reduce reviews present, and no
/// umbrella id-stuffing. Reports EVERY defect in one aggregated error.
async fn validate_authored_plan(
    source: &str,
    expected_task_ids: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    let details = dry_run_workflow_plan_full_details(source, None)
        .await
        .map_err(|err| format!("dry run failed: {err}"))?;
    let planned = &details.calls;
    let write_task_claims = &details.write_task_claims;
    let mut defects: Vec<String> = Vec::new();

    let mut claims_by_id: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
    let mut claims_by_call: std::collections::BTreeMap<&str, usize> = Default::default();
    for (task_id, call_id) in write_task_claims {
        claims_by_id.entry(task_id).or_default().push(call_id);
        *claims_by_call.entry(call_id).or_default() += 1;
    }
    let missing: Vec<&str> = expected_task_ids
        .iter()
        .filter(|id| !claims_by_id.contains_key(id.as_str()))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        defects.push(format!(
            "these task ids have NO write coverage — implement each, or prove it already-implemented through a write agent's typed noop: {}",
            missing.join(", ")
        ));
    }
    for (task_id, calls) in &claims_by_id {
        let mut calls = calls.clone();
        calls.sort();
        calls.dedup();
        if calls.len() > 1 {
            defects.push(format!(
                "task `{task_id}` is claimed by MULTIPLE write calls ({}) — exactly one write call per task",
                calls.join(", ")
            ));
        }
    }
    for (call_id, count) in &claims_by_call {
        if *count > 1 && *count * 2 >= expected_task_ids.len() {
            defects.push(format!(
                "write call `{call_id}` claims {count} of {} tasks — umbrella id-stuffing is not coverage; one write call per task",
                expected_task_ids.len()
            ));
        }
    }

    let work_calls = planned
        .iter()
        .filter(|call| {
            matches!(
                call.method,
                WorkflowV2HostMethod::Agent
                    | WorkflowV2HostMethod::Implementation
                    | WorkflowV2HostMethod::Fanout
                    | WorkflowV2HostMethod::Parallel
            )
        })
        .count();
    if work_calls == 0 {
        defects.push(format!(
            "the script plans ZERO agent calls across {} host call(s)",
            planned.len()
        ));
    }
    let accepted_for_review = if expected_task_ids.is_empty() {
        write_task_claims
            .iter()
            .map(|(task_id, _)| task_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
    } else {
        expected_task_ids.clone()
    };
    if let Err(review_defects) = validate_map_reduce_review_calls(&details, &accepted_for_review) {
        defects.push(review_defects);
    }
    if defects.is_empty() {
        return Ok(());
    }
    Err(defects.join("; AND "))
}

pub(super) const MANDATED_RESULT_FIELDS: [&str; 2] =
    ["adversarial_findings", "uncovered_requirements"];
const CRITIC_TIER: &str = "critic";
const REVIEW_MAP_STAGE: &str = "map";
const REVIEW_REDUCE_FINAL_STAGE: &str = "reduce_final";
const REVIEW_REDUCE_CHUNK_STAGE: &str = "reduce_chunk";
const REVIEW_CONTRACT_MARKER: &str = "reviewContract";
const REMEDIATION_CONTRACT_MARKER: &str = "remediationContract";
const REMEDIATION_STAGE_FIX: &str = "remediate";
const REMEDIATION_STAGE_VERIFY: &str = "verify";
/// Hard ceiling on post-review remediation rounds per task. The pass exists to
/// close findings, not to grind a task until something reports green.
const REMEDIATION_MAX_ROUNDS: u64 = 3;
const REVIEW_BOUNDS_HINT: &str = "maxInputBytes";
const MANDATED_REVIEW_KINDS: [(&str, &str); 2] = [
    ("adversarial_findings", "adversarial findings review"),
    ("uncovered_requirements", "source-coverage audit"),
];

fn review_contract(call: &WorkflowV2HostCall) -> Option<&serde_json::Value> {
    call.options
        .extra
        .get("reviewContract")
        .or_else(|| call.options.extra.get("review_contract"))
}

fn review_contract_string<'a>(contract: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    contract.get(key).and_then(serde_json::Value::as_str)
}

fn review_contract_kind(call: &WorkflowV2HostCall) -> Option<&str> {
    review_contract(call).and_then(|contract| review_contract_string(contract, "kind"))
}

fn review_contract_stage(call: &WorkflowV2HostCall) -> Option<&str> {
    review_contract(call).and_then(|contract| review_contract_string(contract, "stage"))
}

fn is_review_contract_call(call: &WorkflowV2HostCall) -> bool {
    review_contract(call).is_some()
}

fn remediation_contract(call: &WorkflowV2HostCall) -> Option<&serde_json::Value> {
    call.options
        .extra
        .get(REMEDIATION_CONTRACT_MARKER)
        .or_else(|| call.options.extra.get("remediation_contract"))
}

fn remediation_contract_string<'a>(call: &'a WorkflowV2HostCall, key: &str) -> Option<&'a str> {
    remediation_contract(call).and_then(|contract| contract.get(key).and_then(|v| v.as_str()))
}

fn is_review_remediation_call(call: &WorkflowV2HostCall) -> bool {
    remediation_contract(call).is_some()
}

/// Task work for the review-ordering rule.
///
/// Review-remediation calls are excluded: work a mandatory review ASKED for,
/// and which is re-verified afterwards, is categorically different from work
/// smuggled in after the reviewers looked. The exclusion is not a free pass —
/// `validate_review_remediation_calls` checks every claim such a call makes
/// against the actual plan, so declaring a contract without the real structure
/// behind it fails louder than omitting one.
fn is_task_work_call(call: &WorkflowV2HostCall) -> bool {
    !is_review_contract_call(call)
        && !is_review_remediation_call(call)
        && matches!(
            call.method,
            WorkflowV2HostMethod::Agent
                | WorkflowV2HostMethod::Fanout
                | WorkflowV2HostMethod::Implementation
                | WorkflowV2HostMethod::Parallel
        )
}

fn is_critic(call: &WorkflowV2HostCall) -> bool {
    call.options
        .role
        .as_deref()
        .is_some_and(|role| role.eq_ignore_ascii_case(CRITIC_TIER))
}

fn call_index(planned: &[WorkflowV2HostCall], call_id: &str) -> Option<usize> {
    planned.iter().position(|call| call.id == call_id)
}

/// Validate every call that claims to be review-driven remediation.
///
/// Such calls are exempt from the review-ordering rule, so the exemption has to
/// cost more than it grants. Each one must name final reduce calls that really
/// exist and really precede it, stay inside a bounded round count, and — for a
/// write — be followed by a verifier for the same task. An author who forges a
/// contract to slip work past the reviewers has to build a genuine, bounded,
/// re-verified remediation loop to do it, which is the thing we wanted anyway.
fn review_remediation_defects(planned: &[WorkflowV2HostCall]) -> Vec<String> {
    let mut defects = Vec::new();
    let reduce_final_indices: std::collections::BTreeMap<&str, usize> = planned
        .iter()
        .enumerate()
        .filter(|(_, call)| matches!(review_contract_stage(call), Some(REVIEW_REDUCE_FINAL_STAGE)))
        .map(|(index, call)| (call.id.as_str(), index))
        .collect();

    for (index, call) in planned.iter().enumerate() {
        let Some(contract) = remediation_contract(call) else {
            continue;
        };
        let stage = remediation_contract_string(call, "stage").unwrap_or("<missing-stage>");
        if !matches!(stage, REMEDIATION_STAGE_FIX | REMEDIATION_STAGE_VERIFY) {
            defects.push(format!(
                "review remediation `{}` declares unknown stage `{stage}` (expected `{REMEDIATION_STAGE_FIX}` or `{REMEDIATION_STAGE_VERIFY}`)",
                call.id
            ));
        }
        if stage == REMEDIATION_STAGE_VERIFY && call.write_mode.is_some() {
            defects.push(format!(
                "review remediation verifier `{}` must be read-only and must not set write mode",
                call.id
            ));
        }
        let Some(task_id) = remediation_contract_string(call, "taskId") else {
            defects.push(format!(
                "review remediation `{}` must name the canonical task it remediates via `taskId`",
                call.id
            ));
            continue;
        };

        let sources: Vec<&str> = contract
            .get("sourceReduceCallIds")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .collect();
        if sources.is_empty() {
            defects.push(format!(
                "review remediation `{}` must name the final reduce call(s) whose findings it acts on via `sourceReduceCallIds`",
                call.id
            ));
        }
        for source in &sources {
            match reduce_final_indices.get(source) {
                None => defects.push(format!(
                    "review remediation `{}` names `{source}` as a source, but no final review reduce with that id is planned",
                    call.id
                )),
                Some(&reduce_index) if reduce_index > index => defects.push(format!(
                    "review remediation `{}` runs BEFORE its source reduce `{source}` — remediation may only act on findings that already exist",
                    call.id
                )),
                Some(_) => {}
            }
        }

        let max_rounds = contract
            .get("maxRounds")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if max_rounds == 0 || max_rounds > REMEDIATION_MAX_ROUNDS {
            defects.push(format!(
                "review remediation `{}` must declare `maxRounds` between 1 and {REMEDIATION_MAX_ROUNDS} (found {max_rounds})",
                call.id
            ));
        }
        let round = contract
            .get("round")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if round == 0 || (max_rounds > 0 && round > max_rounds) {
            defects.push(format!(
                "review remediation `{}` declares round {round}, outside its own bound of {max_rounds}",
                call.id
            ));
        }

        // A fix that nothing re-checks is exactly the unreviewed work the
        // ordering rule exists to prevent.
        if stage == REMEDIATION_STAGE_FIX {
            let verified_later = planned.iter().skip(index + 1).any(|later| {
                remediation_contract_string(later, "stage") == Some(REMEDIATION_STAGE_VERIFY)
                    && remediation_contract_string(later, "taskId") == Some(task_id)
            });
            if !verified_later {
                defects.push(format!(
                    "review remediation `{}` changes {task_id} after review but no later remediation verifier re-checks that task",
                    call.id
                ));
            }
        }
    }
    defects
}

/// Enforce the mandatory reviews on the EXECUTED/PLANNED call sequence:
/// read-only critic map reviewers cover every accepted task exactly once,
/// then bounded critic reducers preserve map findings into the accounting
/// fields. Reports EVERY defect in one error and names near-misses.
fn validate_map_reduce_review_calls(
    details: &WorkflowDryRunPlanDetails,
    accepted_task_ids: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    let planned = &details.calls;
    let first_review = planned.iter().position(is_review_contract_call);
    let last_work = planned.iter().rposition(is_task_work_call);
    let mut defects: Vec<String> = Vec::new();

    if let (Some(first_review), Some(last_work)) = (first_review, last_work)
        && first_review < last_work
    {
        defects.push(
            "mandatory map→reduce reviews start BEFORE task work finishes — all review map/reduce calls must run after implementation, remediation, and verification work"
                .to_string(),
        );
    }

    defects.extend(review_remediation_defects(planned));

    for call in planned.iter().filter(|call| is_review_contract_call(call)) {
        let kind = review_contract_kind(call).unwrap_or("<missing-kind>");
        let stage = review_contract_stage(call).unwrap_or("<missing-stage>");
        if call.write_mode.is_some() {
            defects.push(format!(
                "review call `{}` ({kind}/{stage}) must be read-only and must not set write mode",
                call.id
            ));
        }
        if !is_critic(call) {
            defects.push(format!(
                "review call `{}` ({kind}/{stage}) must use tier '{CRITIC_TIER}'",
                call.id
            ));
        }
    }

    for call in planned
        .iter()
        .filter(|call| matches!(review_contract_stage(call), Some(REVIEW_MAP_STAGE)))
    {
        if !matches!(
            call.method,
            WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel
        ) {
            defects.push(format!(
                "review map `{}` must run as read-only w.parallel or w.fanout, not w.{}",
                call.id,
                call.method.as_str()
            ));
        }
    }
    for call in planned.iter().filter(|call| {
        matches!(
            review_contract_stage(call),
            Some(REVIEW_REDUCE_FINAL_STAGE | REVIEW_REDUCE_CHUNK_STAGE)
        )
    }) {
        if call.method != WorkflowV2HostMethod::Reduce {
            defects.push(format!(
                "review reducer `{}` must run as w.reduce, not w.{}",
                call.id,
                call.method.as_str()
            ));
        }
    }

    let legacy_labels = ["adversarial-review", "coverage-audit"];
    for label in legacy_labels {
        if planned.iter().any(|call| {
            call.method == WorkflowV2HostMethod::Agent
                && (call.id == label
                    || call
                        .id
                        .strip_prefix(label)
                        .and_then(|rest| rest.strip_prefix('-'))
                        .is_some_and(|ordinal| {
                            !ordinal.is_empty() && ordinal.bytes().all(|b| b.is_ascii_digit())
                        }))
        }) {
            defects.push(format!(
                "legacy monolithic review `{label}` no longer satisfies the mandate — use read-only critic {REVIEW_CONTRACT_MARKER} map→reduce calls"
            ));
        }
    }

    let map_call_ids = details
        .review_map_claims
        .iter()
        .map(|claim| claim.call_id.clone())
        .collect::<std::collections::BTreeSet<_>>();

    for (review_kind, purpose) in MANDATED_REVIEW_KINDS {
        validate_review_kind_shape(
            details,
            accepted_task_ids,
            review_kind,
            purpose,
            &map_call_ids,
            &mut defects,
        );
    }

    if defects.is_empty() {
        return Ok(());
    }
    Err(format!(
        "mandatory map→reduce review defects (fix EVERY one): {}",
        defects.join("; AND ")
    ))
}

fn validate_review_kind_shape(
    details: &WorkflowDryRunPlanDetails,
    accepted_task_ids: &std::collections::BTreeSet<String>,
    review_kind: &str,
    purpose: &str,
    all_map_call_ids: &std::collections::BTreeSet<String>,
    defects: &mut Vec<String>,
) {
    let map_call_ids_for_kind = details
        .calls
        .iter()
        .filter(|call| {
            review_contract_kind(call) == Some(review_kind)
                && review_contract_stage(call) == Some(REVIEW_MAP_STAGE)
        })
        .map(|call| call.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let maps = details
        .review_map_claims
        .iter()
        .filter(|claim| claim.review_kind == review_kind)
        .collect::<Vec<_>>();
    if map_call_ids_for_kind.is_empty() {
        defects.push(format!(
            "missing {purpose} map review — add read-only critic map calls with {REVIEW_CONTRACT_MARKER}.kind='{review_kind}' and stage='{REVIEW_MAP_STAGE}'"
        ));
    }

    let mut by_task: std::collections::BTreeMap<&str, Vec<String>> = Default::default();
    for claim in &maps {
        if call_index(&details.calls, &claim.call_id).is_none() {
            defects.push(format!(
                "{purpose} map call `{}` was planned but did not execute in the live call sequence",
                claim.call_id
            ));
        }
        if claim.task_ids.len() != 1 {
            defects.push(format!(
                "{purpose} map item in call `{}` item {:?} covers {} task ids ({:?}) — each map item must cover exactly one accepted task",
                claim.call_id,
                claim.item_id,
                claim.task_ids.len(),
                claim.task_ids
            ));
            continue;
        }
        let task_id = claim.task_ids[0].as_str();
        if !accepted_task_ids.contains(task_id) {
            defects.push(format!(
                "{purpose} map item in call `{}` covers unknown or non-accepted task `{task_id}`",
                claim.call_id
            ));
        }
        by_task
            .entry(task_id)
            .or_default()
            .push(format!("{}:{:?}", claim.call_id, claim.item_id));
    }
    for missing in accepted_task_ids
        .iter()
        .filter(|task_id| !by_task.contains_key(task_id.as_str()))
    {
        defects.push(format!(
            "{purpose} map coverage omitted accepted task `{missing}`"
        ));
    }
    for (task_id, claims) in by_task {
        if claims.len() > 1 {
            defects.push(format!(
                "{purpose} map coverage includes accepted task `{task_id}` more than once ({})",
                claims.join(", ")
            ));
        }
    }

    let reducers = details
        .review_reduce_edges
        .iter()
        .filter(|edge| edge.review_kind == review_kind)
        .collect::<Vec<_>>();
    let finals = reducers
        .iter()
        .filter(|edge| edge.stage == REVIEW_REDUCE_FINAL_STAGE)
        .collect::<Vec<_>>();
    if finals.len() != 1 {
        defects.push(format!(
            "{purpose} must have exactly one final reducer with {REVIEW_CONTRACT_MARKER}.stage='{REVIEW_REDUCE_FINAL_STAGE}' (found {})",
            finals.len()
        ));
    }
    for edge in &reducers {
        if !edge.preserve_map_findings {
            defects.push(format!(
                "{purpose} reducer `{}` must declare preserveMapFindings: true",
                edge.call_id
            ));
        }
        if edge.max_input_bytes.is_none() && edge.max_findings_per_reduce.is_none() {
            defects.push(format!(
                "{purpose} reducer `{}` must declare a reduce bound such as {REVIEW_BOUNDS_HINT} or maxFindingsPerReduce",
                edge.call_id
            ));
        }
        if let Some(index) = call_index(&details.calls, &edge.call_id) {
            for source in edge
                .source_map_call_ids
                .iter()
                .chain(edge.source_reduce_call_ids.iter())
            {
                match call_index(&details.calls, source) {
                    Some(source_index) if source_index > index => {
                        defects.push(format!(
                            "{purpose} reducer `{}` references source `{source}` that runs after it",
                            edge.call_id
                        ));
                    }
                    Some(_) => {}
                    None => defects.push(format!(
                        "{purpose} reducer `{}` references source `{source}` that did not execute",
                        edge.call_id
                    )),
                }
            }
        }
    }

    if let Some(final_reduce) = finals.first() {
        if final_reduce.accounting_field.as_deref() != Some(review_kind) {
            defects.push(format!(
                "{purpose} final reducer `{}` must declare accountingField: '{review_kind}'",
                final_reduce.call_id
            ));
        }
        let direct_maps = final_reduce
            .source_map_call_ids
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let chunk_sources = final_reduce
            .source_reduce_call_ids
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let expected_maps = map_call_ids_for_kind.clone();
        if chunk_sources.is_empty() {
            if direct_maps != expected_maps {
                defects.push(format!(
                    "{purpose} final reducer `{}` must reference every {review_kind} map call exactly once: expected={expected_maps:?} actual={direct_maps:?}",
                    final_reduce.call_id
                ));
            }
        } else {
            for source in &chunk_sources {
                if !reducers
                    .iter()
                    .any(|edge| edge.stage == REVIEW_REDUCE_CHUNK_STAGE && edge.call_id == *source)
                {
                    defects.push(format!(
                        "{purpose} final reducer `{}` references unknown chunk reducer `{source}`",
                        final_reduce.call_id
                    ));
                }
            }
            let chunked_maps = reducers
                .iter()
                .filter(|edge| chunk_sources.contains(&edge.call_id))
                .flat_map(|edge| edge.source_map_call_ids.iter().cloned())
                .collect::<std::collections::BTreeSet<_>>();
            if chunked_maps != expected_maps {
                defects.push(format!(
                    "{purpose} chunk reducers must cover every {review_kind} map call exactly once before final reduce: expected={expected_maps:?} actual={chunked_maps:?}"
                ));
            }
        }
        for source in direct_maps.iter().chain(chunk_sources.iter()) {
            if all_map_call_ids.contains(source) && !expected_maps.contains(source) {
                defects.push(format!(
                    "{purpose} final reducer `{}` references map call `{source}` from another review kind",
                    final_reduce.call_id
                ));
            }
        }
    }
}

fn validate_review_accounting_from_reducers(
    script_result: Option<&str>,
    details: &WorkflowDryRunPlanDetails,
    store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<()> {
    let raw = script_result.ok_or_else(|| {
        WorkflowError::SpecInvalid("authored workflow returned no task accounting".to_string())
    })?;
    let accounting: serde_json::Value = serde_json::from_str(raw).map_err(|err| {
        WorkflowError::SpecInvalid(format!(
            "authored workflow task accounting was not JSON: {err}"
        ))
    })?;
    for (review_kind, purpose) in MANDATED_REVIEW_KINDS {
        let final_reduce = details
            .review_reduce_edges
            .iter()
            .find(|edge| edge.review_kind == review_kind && edge.stage == REVIEW_REDUCE_FINAL_STAGE)
            .ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "{purpose} accounting has no final reducer to bind `{review_kind}`"
                ))
            })?;
        let map_findings = collect_map_findings(details, store, review_kind)?;
        let reduce_record = store
            .load_call_record(&final_reduce.call_id)?
            .ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "{purpose} final reducer record `{}` is missing",
                    final_reduce.call_id
                ))
            })?;
        if reduce_record.invalidated_by.is_some() {
            return Err(WorkflowError::SpecInvalid(format!(
                "{purpose} final reducer `{}` was invalidated and cannot back accounting",
                final_reduce.call_id
            )));
        }
        let reduce_findings = extract_review_findings_from_record(&reduce_record)?;
        assert_multiset_contains(
            &reduce_findings,
            &map_findings,
            &format!(
                "{purpose} final reducer `{}` dropped map findings",
                final_reduce.call_id
            ),
        )?;
        let accounting_findings = accounting
            .get(review_kind)
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "authored workflow accounting omitted `{review_kind}` — it must come from the {purpose} reducer"
                ))
            })?
            .clone();
        assert_multiset_equal(
            &accounting_findings,
            &reduce_findings,
            &format!(
                "authored workflow accounting field `{review_kind}` does not match final reducer `{}`",
                final_reduce.call_id
            ),
        )?;
    }
    Ok(())
}

fn collect_map_findings(
    details: &WorkflowDryRunPlanDetails,
    store: &WorkflowV2ResultStore,
    review_kind: &str,
) -> archon_workflow::WorkflowResult<Vec<serde_json::Value>> {
    let mut findings = Vec::new();
    let call_ids = details
        .review_map_claims
        .iter()
        .filter(|claim| claim.review_kind == review_kind)
        .map(|claim| claim.call_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for call_id in call_ids {
        let record = store.load_call_record(&call_id)?.ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "mandatory review map record `{call_id}` is missing"
            ))
        })?;
        if record.invalidated_by.is_some() {
            return Err(WorkflowError::SpecInvalid(format!(
                "mandatory review map record `{call_id}` was invalidated"
            )));
        }
        findings.extend(extract_review_findings_from_record(&record)?);
    }
    Ok(findings)
}

fn extract_review_findings_from_record(
    record: &WorkflowV2CallRecord,
) -> archon_workflow::WorkflowResult<Vec<serde_json::Value>> {
    let mut findings = Vec::new();
    collect_findings_arrays(&record.result.data, &mut findings);
    if findings.is_empty() {
        return Ok(Vec::new());
    }
    Ok(findings)
}

fn collect_findings_arrays(value: &serde_json::Value, findings: &mut Vec<serde_json::Value>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_findings_arrays(item, findings);
            }
        }
        serde_json::Value::Object(object) => {
            for key in ["findings", "adversarial_findings", "uncovered_requirements"] {
                if let Some(array) = object.get(key).and_then(serde_json::Value::as_array) {
                    findings.extend(array.iter().cloned());
                }
            }
            for key in ["data", "result", "items", "outcomes"] {
                if let Some(child) = object.get(key) {
                    collect_findings_arrays(child, findings);
                }
            }
        }
        _ => {}
    }
}

fn assert_multiset_contains(
    haystack: &[serde_json::Value],
    needles: &[serde_json::Value],
    context: &str,
) -> archon_workflow::WorkflowResult<()> {
    let haystack = finding_multiset(haystack)?;
    let needles = finding_multiset(needles)?;
    for (finding, count) in needles {
        let have = haystack.get(&finding).copied().unwrap_or(0);
        if have < count {
            return Err(WorkflowError::SpecInvalid(format!(
                "{context}: missing finding {finding} expected {count} found {have}"
            )));
        }
    }
    Ok(())
}

fn assert_multiset_equal(
    left: &[serde_json::Value],
    right: &[serde_json::Value],
    context: &str,
) -> archon_workflow::WorkflowResult<()> {
    let left = finding_multiset(left)?;
    let right = finding_multiset(right)?;
    if left != right {
        return Err(WorkflowError::SpecInvalid(format!(
            "{context}: left={left:?} right={right:?}"
        )));
    }
    Ok(())
}

fn finding_multiset(
    values: &[serde_json::Value],
) -> archon_workflow::WorkflowResult<std::collections::BTreeMap<String, usize>> {
    let mut out = std::collections::BTreeMap::new();
    for value in values {
        let key = serde_json::to_string(value)?;
        *out.entry(key).or_default() += 1;
    }
    Ok(out)
}
