/// Dry-run pre-flight: execute the authored script against the recording stub
/// host and require it to PLAN real work — with every universe task claimed
/// by EXACTLY ONE write call, both mandated reviews present, and no
/// umbrella id-stuffing. Reports EVERY defect in one aggregated error.
async fn validate_authored_plan(
    source: &str,
    expected_task_ids: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    let (planned, write_task_claims) = dry_run_workflow_plan_details(source, None)
        .await
        .map_err(|err| format!("dry run failed: {err}"))?;
    let mut defects: Vec<String> = Vec::new();

    let mut claims_by_id: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
    let mut claims_by_call: std::collections::BTreeMap<&str, usize> = Default::default();
    for (task_id, call_id) in &write_task_claims {
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
    if let Err(review_defects) = validate_mandatory_review_calls(&planned) {
        defects.push(review_defects);
    }
    if defects.is_empty() {
        return Ok(());
    }
    Err(defects.join("; AND "))
}

pub(super) const MANDATED_REVIEWS: [(&str, &str, bool); 2] = [
    ("adversarial-review", "adversarial review", true),
    ("coverage-audit", "source-coverage audit", false),
];
pub(super) const MANDATED_RESULT_FIELDS: [&str; 2] =
    ["adversarial_findings", "uncovered_requirements"];
const CRITIC_TIER: &str = "critic";

pub(super) fn mandate_call_hint(label: &str, requires_critic: bool) -> String {
    if requires_critic {
        format!("await agent(..., {{ label: '{label}', tier: '{CRITIC_TIER}' }})")
    } else {
        format!("await agent(..., {{ label: '{label}' }})")
    }
}

/// The prelude mints agent ids as `<label>-<ordinal>`; a mandate matches only
/// that exact shape (or the bare label), never labels that merely extend it.
fn call_id_matches_label(id: &str, label: &str) -> bool {
    if id == label {
        return true;
    }
    id.strip_prefix(label)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|ordinal| !ordinal.is_empty() && ordinal.bytes().all(|b| b.is_ascii_digit()))
}

fn is_mandated_review_call(call: &WorkflowV2HostCall) -> bool {
    call.method == WorkflowV2HostMethod::Agent
        && MANDATED_REVIEWS
            .iter()
            .any(|(label, _, _)| call_id_matches_label(&call.id, label))
}

fn is_task_work_call(call: &WorkflowV2HostCall) -> bool {
    !is_mandated_review_call(call)
        && matches!(
            call.method,
            WorkflowV2HostMethod::Agent
                | WorkflowV2HostMethod::Fanout
                | WorkflowV2HostMethod::Implementation
                | WorkflowV2HostMethod::Parallel
        )
}

/// Enforce the mandated reviews on the EXECUTED/PLANNED call sequence:
/// present, as separate top-level read-only agent() calls, with the critic
/// tier where required, positioned AFTER all task work. Reports EVERY defect
/// in one error and names the ACTUAL cause for near-misses.
fn validate_mandatory_review_calls(planned: &[WorkflowV2HostCall]) -> Result<(), String> {
    let last_work = planned.iter().rposition(is_task_work_call);
    let mut defects: Vec<String> = Vec::new();
    for (label, purpose, requires_critic) in MANDATED_REVIEWS {
        let hint = mandate_call_hint(label, requires_critic);
        let matched = planned.iter().enumerate().find(|(_, call)| {
            call.method == WorkflowV2HostMethod::Agent && call_id_matches_label(&call.id, label)
        });
        let Some((index, call)) = matched else {
            // Near-miss diagnosis: the label exists but under the wrong call
            // kind (agents() batch → Parallel, write:true → Fanout), or only
            // as an extended label — name the real defect, not "omitted".
            if let Some(wrong_kind) = planned.iter().find(|call| {
                call.method != WorkflowV2HostMethod::Agent
                    && call_id_matches_label(&call.id, label)
            }) {
                defects.push(format!(
                    "the {purpose} labeled `{label}` ran as w.{} — the mandated reviews must be SEPARATE top-level read-only agent() calls, never inside agents() batches and never with write:true; use: {hint}",
                    wrong_kind.method.as_str()
                ));
            } else if let Some(extended) = planned.iter().find(|call| {
                call.method == WorkflowV2HostMethod::Agent
                    && call.id.starts_with(label)
                    && !call_id_matches_label(&call.id, label)
            }) {
                defects.push(format!(
                    "no {purpose} agent with the exact label `{label}` (found `{}` — extended labels do not count); use: {hint}",
                    extended.id
                ));
            } else {
                defects.push(format!(
                    "the mandatory {purpose} agent with exact label `{label}` is missing; add: {hint}"
                ));
            }
            continue;
        };
        if requires_critic
            && !call
                .options
                .role
                .as_deref()
                .is_some_and(|role| role.eq_ignore_ascii_case(CRITIC_TIER))
        {
            defects.push(format!(
                "the {purpose} agent `{}` must use tier '{CRITIC_TIER}' so it routes to the dedicated adversarial reviewer; use: {hint}",
                call.id
            ));
        }
        if let Some(last_work) = last_work
            && index < last_work
        {
            defects.push(format!(
                "the {purpose} agent `{}` runs BEFORE task work finishes — both mandated reviews must come after ALL agent, implementation, fanout, and parallel calls",
                call.id
            ));
        }
    }
    if defects.is_empty() {
        return Ok(());
    }
    Err(format!(
        "mandated-review defects (fix EVERY one): {}",
        defects.join("; AND ")
    ))
}
