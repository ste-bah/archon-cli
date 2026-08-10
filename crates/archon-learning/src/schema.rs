//! CozoDB relation definitions for governed learning.
//!
//! Uses the same idempotent `:create` pattern as `archon-docs::schema`.
//! All `:create` calls use `key => values` syntax.

use anyhow::Result;
use cozo::{DbInstance, ScriptMutability};

use crate::cozo_guard::run_script_guarded;
use crate::errors::COZO_RELATION_ALREADY_EXISTS;

#[cfg(test)]
use crate::errors::COZO_RELATION_NOT_FOUND;

#[path = "schema_sandbox.rs"]
mod schema_sandbox;
use schema_sandbox::{
    ensure_sandbox_profiles, ensure_sandbox_runtime_events, ensure_sandbox_sessions,
};

/// Ensure all governed-learning relations exist. Idempotent.
pub fn ensure_learning_schema(db: &DbInstance) -> Result<()> {
    ensure_learning_events(db)?;
    ensure_behaviour_proposals(db)?;
    ensure_behaviour_manifest_versions(db)?;
    ensure_behaviour_policy_decisions(db)?;
    ensure_behaviour_approvals(db)?;
    ensure_provider_runtime_events(db)?;
    crate::llm_call_usage::ensure_schema(db)?;
    ensure_agent_performance_ledger(db)?;
    ensure_agent_evolution_proposals(db)?;
    ensure_agent_profile_versions(db)?;
    ensure_agent_shadow_evaluations(db)?;
    ensure_memory_promotion_candidates(db)?;
    ensure_memory_retirement_proposals(db)?;
    ensure_permission_runtime_events(db)?;
    ensure_provider_auth_profiles(db)?;
    ensure_provider_rate_limit_windows(db)?;
    ensure_provider_runtime_status_snapshots(db)?;
    ensure_sandbox_runtime_events(db)?;
    ensure_sandbox_profiles(db)?;
    ensure_sandbox_sessions(db)?;
    Ok(())
}

/// Run a `:create` script, ignoring "already exists" errors only.
fn run_create(db: &DbInstance, script: &str) -> Result<()> {
    match run_script_guarded(
        db,
        script,
        Default::default(),
        ScriptMutability::Mutable,
        "learning schema creation failed",
    ) {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if COZO_RELATION_ALREADY_EXISTS
                .iter()
                .any(|phrase| msg.contains(phrase))
            {
                Ok(())
            } else {
                Err(anyhow::anyhow!("learning schema creation failed: {msg}"))
            }
        }
    }
}

fn ensure_learning_events(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create learning_events {
            event_id: String =>
            workspace_id: String,
            event_type: String,
            source_artifact_id: String default "",
            outcome_artifact_id: String default "",
            signal: String default "{}",
            confidence: Float default 0.5,
            provenance_record_id: String default "",
            created_at: String,
        }"#,
    )?;
    run_create(
        db,
        "::index create learning_events:by_created_at {created_at}",
    )?;
    run_create(
        db,
        "::index create learning_events:by_type_created_at {event_type, created_at}",
    )
}

fn ensure_behaviour_proposals(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create behaviour_proposals {
            proposal_id: String =>
            workspace_id: String,
            manifest_kind: String,
            current_version: String,
            proposed_version: String,
            diff: String,
            evidence_ids_json: String default "[]",
            risk_level: String,
            policy_decision: String,
            status: String,
            created_at: String,
        }"#,
    )
}

fn ensure_behaviour_manifest_versions(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create behaviour_manifest_versions {
            version_id: String =>
            manifest_kind: String,
            version_number: Int default 1,
            content_json: String default "{}",
            diff: String default "",
            parent_version_id: String default "",
            created_by_proposal_id: String default "",
            is_rollback_target: Bool default false,
            created_at: String,
        }"#,
    )
}

fn ensure_behaviour_policy_decisions(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create behaviour_policy_decisions {
            decision_id: String =>
            proposal_id: String,
            rule_name: String,
            outcome: String,
            reason: String,
            evaluated_inputs_json: String default "{}",
            created_at: String,
        }"#,
    )
}

fn ensure_behaviour_approvals(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create behaviour_approvals {
            approval_id: String =>
            proposal_id: String,
            approver: String,
            approved: Bool,
            comment: String default "",
            created_at: String,
        }"#,
    )
}

fn ensure_provider_runtime_events(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create provider_runtime_events {
            event_id: String =>
            provider_id: String,
            profile_id: String default "",
            model_id: String default "",
            runtime_mode: String,
            event_type: String,
            severity: String,
            reason_code: String default "",
            message: String default "",
            retry_count: Int default 0,
            fallback_from: String default "",
            fallback_to: String default "",
            request_id: String default "",
            run_id: String default "",
            pipeline_id: String default "",
            raw_redacted_json: String default "{}",
            created_at: String,
        }"#,
    )
}

fn ensure_agent_performance_ledger(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create agent_performance_ledger {
            event_id: String =>
            agent_type: String,
            agent_version: String default "",
            run_id: String default "",
            pipeline_id: String default "",
            phase: String default "",
            task_hash: String default "",
            model_id: String default "",
            provider_id: String default "",
            profile_id: String default "",
            permission_mode: String default "",
            completion_status: String,
            applied_rate: Float default -1.0,
            completion_rate: Float default -1.0,
            quality_score: Float default -1.0,
            l_score: Float default -1.0,
            user_accepted: String default "",
            user_corrected: String default "",
            gate_failed: String default "",
            test_failed: Bool default false,
            provider_incident_id: String default "",
            evidence_ids_json: String default "[]",
            created_at: String,
        }"#,
    )
}

fn ensure_agent_evolution_proposals(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create agent_evolution_proposals {
            proposal_id: String =>
            agent_type: String,
            current_version: String,
            proposed_version: String,
            kind: String,
            diff: String,
            evidence_ids_json: String default "[]",
            risk_level: String,
            policy_decision: String,
            status: String,
            expected_impact: String default "",
            rollback_target_version: String default "",
            affects_provider_identity: Bool default false,
            affects_permissions: Bool default false,
            created_at: String,
        }"#,
    )
}

fn ensure_agent_profile_versions(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create agent_profile_versions {
            version_id: String =>
            agent_type: String,
            version_number: Int default 1,
            parent_version_id: String default "",
            source: String,
            created_by_proposal_id: String default "",
            profile_json: String default "{}",
            prompt_hash: String default "",
            tools_hash: String default "",
            model_hash: String default "",
            memory_hash: String default "",
            is_active: Bool default false,
            is_rollback_target: Bool default false,
            created_at: String,
        }"#,
    )
}

fn ensure_agent_shadow_evaluations(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create agent_shadow_evaluations {
            evaluation_id: String =>
            proposal_id: String,
            agent_type: String,
            candidate_version_id: String default "",
            baseline_version_id: String default "",
            task_set_id: String default "",
            baseline_score: Float default 0.0,
            candidate_score: Float default 0.0,
            regression_count: Int default 0,
            improvement_count: Int default 0,
            verdict: String,
            evidence_json: String default "{}",
            created_at: String,
        }"#,
    )
}

fn ensure_memory_promotion_candidates(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create memory_promotion_candidates {
            candidate_id: String =>
            agent_type: String,
            signal_source: String,
            target: String,
            claim: String,
            confidence: Float default 0.0,
            frequency_score: Float default 0.0,
            recency_score: Float default 0.0,
            diversity_score: Float default 0.0,
            evidence_quality: Float default 0.0,
            evidence_ids_json: String default "[]",
            proposal_required: Bool default true,
            created_at: String,
        }"#,
    )
}

/// Memories a background consolidation pass proposed retiring, and did not.
///
/// `status` defaults to `Pending` at the storage layer as well as in code: a row
/// that somehow arrives without one must read as undecided, never as consent.
fn ensure_memory_retirement_proposals(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create memory_retirement_proposals {
            proposal_id: String =>
            memory_id: String,
            memory_title: String default "",
            excerpt: String default "",
            memory_type: String default "fact",
            importance: Float default 0.0,
            reason_kind: String,
            reason_detail: String default "",
            run_id: String default "",
            status: String default "Pending",
            created_at: String,
        }"#,
    )
}

fn ensure_permission_runtime_events(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create permission_runtime_events {
            event_id: String =>
            session_id: String default "",
            run_id: String default "",
            agent_type: String default "",
            tool_name: String,
            permission_mode: String,
            decision: String,
            reason_code: String default "",
            rule_name: String default "",
            sandbox_backend: String default "",
            raw_redacted_json: String default "{}",
            created_at: String,
        }"#,
    )
}

fn ensure_provider_auth_profiles(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create provider_auth_profiles {
            profile_id: String =>
            provider_id: String,
            auth_kind: String,
            display_name: String default "",
            source: String,
            identity_fingerprint: String default "",
            created_at: String,
            updated_at: String,
            last_used_at: String default "",
            last_good_at: String default "",
            last_failed_at: String default "",
            failure_count: Int default 0,
            cooldown_until: String default "",
            disabled_reason: String default "",
            metadata_redacted_json: String default "{}",
        }"#,
    )
}

fn ensure_provider_rate_limit_windows(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create provider_rate_limit_windows {
            window_id: String =>
            provider_id: String,
            profile_id: String default "",
            model_id: String default "",
            limit_id: String default "",
            limit_name: String default "",
            window_kind: String,
            used_percent: Float default -1.0,
            resets_at: String default "",
            raw_redacted_json: String default "{}",
            observed_at: String,
        }"#,
    )
}

fn ensure_provider_runtime_status_snapshots(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create provider_runtime_status_snapshots {
            status_id: String =>
            provider_id: String,
            display_name: String default "",
            profile_id: String default "",
            model_id: String default "",
            runtime_mode: String,
            identity_status: String,
            health: String,
            last_success_at: String default "",
            last_failure_at: String default "",
            rate_limit_ids_json: String default "[]",
            metadata_redacted_json: String default "{}",
            observed_at: String,
        }"#,
    )
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
