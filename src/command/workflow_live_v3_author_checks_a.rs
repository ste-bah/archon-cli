/// Dry-run pre-flight: execute the authored script against the recording stub
/// host and require it to PLAN real work — with every universe task claimed
/// by EXACTLY ONE write call, mandatory map→reduce reviews present, and no
/// umbrella id-stuffing. Reports EVERY defect in one aggregated error.
use super::*;

pub(crate) async fn validate_authored_plan(
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

pub(crate) const MANDATED_RESULT_FIELDS: [&str; 2] =
    ["adversarial_findings", "uncovered_requirements"];
pub(crate) const CRITIC_TIER: &str = "critic";
pub(crate) const REVIEW_MAP_STAGE: &str = "map";
pub(crate) const REVIEW_REDUCE_FINAL_STAGE: &str = "reduce_final";
pub(super) const REVIEW_REDUCE_CHUNK_STAGE: &str = "reduce_chunk";
pub(super) const REVIEW_CONTRACT_MARKER: &str = "reviewContract";
pub(super) const REMEDIATION_CONTRACT_MARKER: &str = "remediationContract";
pub(super) const REMEDIATION_STAGE_FIX: &str = "remediate";
pub(super) const REMEDIATION_STAGE_VERIFY: &str = "verify";
/// Hard ceiling on post-review remediation rounds per task. The pass exists to
/// close findings, not to grind a task until something reports green.
pub(super) const REMEDIATION_MAX_ROUNDS: u64 = 3;
pub(super) const REVIEW_BOUNDS_HINT: &str = "maxInputBytes";
pub(super) const MANDATED_REVIEW_KINDS: [(&str, &str); 2] = [
    ("adversarial_findings", "adversarial findings review"),
    ("uncovered_requirements", "source-coverage audit"),
];

pub(super) fn review_contract(call: &WorkflowV2HostCall) -> Option<&serde_json::Value> {
    call.options
        .extra
        .get("reviewContract")
        .or_else(|| call.options.extra.get("review_contract"))
}

pub(super) fn review_contract_string<'a>(
    contract: &'a serde_json::Value,
    key: &str,
) -> Option<&'a str> {
    contract.get(key).and_then(serde_json::Value::as_str)
}

pub(super) fn review_contract_kind(call: &WorkflowV2HostCall) -> Option<&str> {
    review_contract(call).and_then(|contract| review_contract_string(contract, "kind"))
}

pub(super) fn review_contract_stage(call: &WorkflowV2HostCall) -> Option<&str> {
    review_contract(call).and_then(|contract| review_contract_string(contract, "stage"))
}

pub(super) fn is_review_contract_call(call: &WorkflowV2HostCall) -> bool {
    review_contract(call).is_some()
}

pub(super) fn remediation_contract(call: &WorkflowV2HostCall) -> Option<&serde_json::Value> {
    call.options
        .extra
        .get(REMEDIATION_CONTRACT_MARKER)
        .or_else(|| call.options.extra.get("remediation_contract"))
}

pub(super) fn remediation_contract_string<'a>(
    call: &'a WorkflowV2HostCall,
    key: &str,
) -> Option<&'a str> {
    remediation_contract(call).and_then(|contract| contract.get(key).and_then(|v| v.as_str()))
}

pub(super) fn is_review_remediation_call(call: &WorkflowV2HostCall) -> bool {
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
pub(super) fn is_task_work_call(call: &WorkflowV2HostCall) -> bool {
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

pub(super) fn is_critic(call: &WorkflowV2HostCall) -> bool {
    call.options
        .role
        .as_deref()
        .is_some_and(|role| role.eq_ignore_ascii_case(CRITIC_TIER))
}

pub(super) fn call_index(planned: &[WorkflowV2HostCall], call_id: &str) -> Option<usize> {
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
pub(super) fn review_remediation_defects(planned: &[WorkflowV2HostCall]) -> Vec<String> {
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
pub(crate) fn validate_map_reduce_review_calls(
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
