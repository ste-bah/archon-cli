use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use cozo::{DataValue, DbInstance, ScriptMutability};
use uuid::Uuid;

use crate::cozo_guard::run_script_guarded;
use crate::{ApplyResult, CanaryOutcome, CognitiveError, Proposal};

pub(crate) struct Evidence {
    pub count: u64,
    pub reflection_ids: Vec<String>,
}

pub(crate) fn query_reflection_evidence(
    db: &DbInstance,
    lesson: &str,
    domain: &str,
) -> Result<Evidence, CognitiveError> {
    let rows = query_reflection_rows_current(db).or_else(|_| query_reflection_rows_legacy(db))?;
    let lesson_key = normalize(lesson);
    let mut ids = BTreeSet::new();
    for row in rows.rows {
        let id = row[0].get_str().unwrap_or("");
        let stored_lesson = row[1].get_str().unwrap_or("");
        let stored_domain = row.get(2).and_then(DataValue::get_str).unwrap_or(domain);
        if normalize(stored_lesson) == lesson_key && domain_matches(stored_domain, domain) {
            ids.insert(id.to_string());
        }
    }
    Ok(Evidence {
        count: ids.len() as u64,
        reflection_ids: ids.into_iter().collect(),
    })
}

pub(crate) fn store_proposal(db: &DbInstance, proposal: &Proposal) -> Result<(), CognitiveError> {
    let mut params = BTreeMap::new();
    params.insert(
        "proposal_id".into(),
        DataValue::from(proposal.proposal_id.as_str()),
    );
    params.insert(
        "reflection_ids_json".into(),
        DataValue::from(serde_json::to_string(&proposal.reflection_ids)?.as_str()),
    );
    params.insert(
        "manifest_kind".into(),
        DataValue::from(proposal.manifest_kind.as_str()),
    );
    params.insert(
        "risk_level".into(),
        DataValue::from(proposal.risk_level.as_str()),
    );
    params.insert(
        "evidence_count".into(),
        DataValue::from(proposal.evidence_count as i64),
    );
    params.insert(
        "lesson_tag".into(),
        DataValue::from(proposal.lesson_tag.as_str()),
    );
    params.insert("domain".into(), DataValue::from(proposal.domain.as_str()));
    params.insert(
        "diff_summary".into(),
        DataValue::from(proposal.diff_summary.as_str()),
    );
    params.insert(
        "rollback_plan".into(),
        DataValue::from(proposal.rollback_plan.as_deref().unwrap_or("")),
    );
    params.insert(
        "created_at".into(),
        DataValue::from(proposal.created_at.to_rfc3339().as_str()),
    );
    run_script_guarded(
        db,
        "?[proposal_id, reflection_ids_json, manifest_kind, risk_level, evidence_count, lesson_tag, domain, diff_summary, rollback_plan, created_at] <- \
         [[$proposal_id, $reflection_ids_json, $manifest_kind, $risk_level, $evidence_count, $lesson_tag, $domain, $diff_summary, $rollback_plan, $created_at]]
         :put governed_proposals { proposal_id => reflection_ids_json, manifest_kind, risk_level, evidence_count, lesson_tag, domain, diff_summary, rollback_plan, created_at }",
        params,
        ScriptMutability::Mutable,
        "store governed proposal",
    )?;
    Ok(())
}

pub(crate) fn store_canary(db: &DbInstance, canary: &CanaryOutcome) -> Result<(), CognitiveError> {
    let mut params = BTreeMap::new();
    params.insert(
        "canary_id".into(),
        DataValue::from(canary.canary_id.as_str()),
    );
    params.insert(
        "proposal_id".into(),
        DataValue::from(canary.proposal_id.as_str()),
    );
    params.insert("passed".into(), DataValue::from(canary.passed));
    params.insert("details".into(), DataValue::from(canary.details.as_str()));
    params.insert(
        "snapshot_ref".into(),
        DataValue::from(canary.snapshot_ref.as_str()),
    );
    params.insert(
        "created_at".into(),
        DataValue::from(canary.created_at.to_rfc3339().as_str()),
    );
    run_script_guarded(
        db,
        "?[canary_id, proposal_id, passed, details, snapshot_ref, created_at] <- \
         [[$canary_id, $proposal_id, $passed, $details, $snapshot_ref, $created_at]]
         :put canary_outcomes { canary_id => proposal_id, passed, details, snapshot_ref, created_at }",
        params,
        ScriptMutability::Mutable,
        "store canary outcome",
    )?;
    Ok(())
}

pub(crate) fn store_apply_result(
    db: &DbInstance,
    result: &ApplyResult,
) -> Result<(), CognitiveError> {
    let (proposal_id, kind, reason, canary, rollback, created_at) = result_parts(result);
    let mut params = BTreeMap::new();
    params.insert(
        "apply_id".into(),
        DataValue::from(Uuid::new_v4().to_string().as_str()),
    );
    params.insert("proposal_id".into(), DataValue::from(proposal_id.as_str()));
    params.insert("result_kind".into(), DataValue::from(kind));
    params.insert("reason".into(), DataValue::from(reason.as_str()));
    params.insert(
        "canary_outcome_ref".into(),
        DataValue::from(canary.as_str()),
    );
    params.insert("rollback_ref".into(), DataValue::from(rollback.as_str()));
    params.insert(
        "created_at".into(),
        DataValue::from(created_at.to_rfc3339().as_str()),
    );
    run_script_guarded(
        db,
        "?[apply_id, proposal_id, result_kind, reason, canary_outcome_ref, rollback_ref, created_at] <- \
         [[$apply_id, $proposal_id, $result_kind, $reason, $canary_outcome_ref, $rollback_ref, $created_at]]
         :put autonomous_apply_results { apply_id => proposal_id, result_kind, reason, canary_outcome_ref, rollback_ref, created_at }",
        params,
        ScriptMutability::Mutable,
        "store autonomous apply result",
    )?;
    Ok(())
}

pub(crate) fn rollback_ref(proposal: &Proposal) -> String {
    rollback_ref_for_id(&proposal.proposal_id)
}

fn query_reflection_rows_current(db: &DbInstance) -> Result<cozo::NamedRows, CognitiveError> {
    run_script_guarded(
        db,
        "?[reflection_id, lesson, situation_kind] := *cognitive_reflections{reflection_id, lesson, situation_kind}",
        Default::default(),
        ScriptMutability::Immutable,
        "query reflection evidence",
    )
}

fn query_reflection_rows_legacy(db: &DbInstance) -> Result<cozo::NamedRows, CognitiveError> {
    run_script_guarded(
        db,
        "?[reflection_id, lesson] := *cognitive_reflections{reflection_id, lesson}",
        Default::default(),
        ScriptMutability::Immutable,
        "query legacy reflection evidence",
    )
}

fn result_parts(
    result: &ApplyResult,
) -> (String, &'static str, String, String, String, DateTime<Utc>) {
    match result {
        ApplyResult::AutoApplied {
            proposal_id,
            canary_outcome_ref,
            rollback_ref,
            applied_at,
        } => (
            proposal_id.clone(),
            "auto_applied",
            String::new(),
            canary_outcome_ref.clone(),
            rollback_ref.clone(),
            *applied_at,
        ),
        ApplyResult::PendingReview {
            proposal_id,
            reason,
        } => (
            proposal_id.clone(),
            "pending_review",
            reason.clone(),
            String::new(),
            String::new(),
            Utc::now(),
        ),
        ApplyResult::Denied {
            proposal_id,
            reason,
        } => (
            proposal_id.clone(),
            "denied",
            reason.clone(),
            String::new(),
            String::new(),
            Utc::now(),
        ),
        ApplyResult::RolledBack {
            proposal_id,
            reason,
            rolled_back_at,
        } => (
            proposal_id.clone(),
            "rolled_back",
            reason.clone(),
            String::new(),
            rollback_ref_for_id(proposal_id),
            *rolled_back_at,
        ),
    }
}

fn rollback_ref_for_id(proposal_id: &str) -> String {
    format!("rollback-{proposal_id}")
}

fn domain_matches(stored: &str, expected: &str) -> bool {
    stored == expected || situation_domain(stored) == expected
}

fn situation_domain(kind: &str) -> &'static str {
    match kind {
        "ci_debug" => "ci",
        "code_change" => "coding",
        "git_mutation" => "git",
        "pipeline_control" => "pipeline",
        "research" => "research",
        "world_model_task" => "world_model",
        "high_risk" => "safety",
        _ => "general",
    }
}

fn normalize(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
