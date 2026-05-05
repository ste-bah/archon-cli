//! Archon Completion Integrity — TSPEC §10.
//!
//! Prevents unsupported "done/tests pass/fixed/indexed/cited" claims by
//! turning completion-sensitive statements into structured claims checked
//! against evidence.
//!
//! ## Modules
//!
//! - `models` — All completion integrity types (§10.1–10.7)
//! - `errors` — `EvidenceEngineError` enum
//! - `schema` — CozoDB schema for 6 completion relations (§10.8)
//! - `store` — CRUD for all relations
//! - `claim_extractor` — Regex/heuristic scanner for completion-sensitive phrases
//! - `evidence_resolver` — Locates evidence from Cozo state
//! - `verification_gates` — `VerificationGate` trait + concrete gates
//! - `report_assembler` — Produces `CompletionReport` with calibrated summary
//! - `incident_recorder` — Records `FalseCompletionIncident` + learning events
//! - `trust` — Computes persisted agent/model/task trust scores

pub mod errors;
pub mod models;
pub mod schema;
pub mod store;

pub mod claim_extractor;
pub mod evidence_resolver;
pub mod incident_recorder;
pub mod report_assembler;
pub mod trust;
pub mod verification_gates;

use anyhow::Result;
use cozo::DbInstance;

#[derive(Clone, Debug)]
pub struct CompletionContext {
    pub workspace_id: String,
    pub agent_key: Option<String>,
    pub model: Option<String>,
}

impl Default for CompletionContext {
    fn default() -> Self {
        Self {
            workspace_id: trust::DEFAULT_WORKSPACE_ID.to_string(),
            agent_key: None,
            model: None,
        }
    }
}

/// Run the full completion integrity check against a pipeline run.
///
/// 1. Extract claims from the output text.
/// 2. Resolve evidence from Cozo state.
/// 3. Run verification gates.
/// 4. Assemble a completion report.
/// 5. Persist everything.
pub async fn check_completion(
    db: &DbInstance,
    run_id: &str,
    output_text: &str,
    task_type: &str,
) -> Result<models::CompletionReport> {
    check_completion_with_context(
        db,
        run_id,
        output_text,
        task_type,
        CompletionContext::default(),
    )
    .await
}

pub async fn check_completion_with_context(
    db: &DbInstance,
    run_id: &str,
    output_text: &str,
    task_type: &str,
    context: CompletionContext,
) -> Result<models::CompletionReport> {
    // Ensure schema
    schema::ensure_completion_schema(db).map_err(|e| errors::EvidenceEngineError::Storage {
        message: e.to_string(),
    })?;

    store::insert_completion_run_context(
        db,
        &models::CompletionRunContext {
            run_id: run_id.to_string(),
            workspace_id: context.workspace_id.clone(),
            agent_key: context.agent_key.clone(),
            model: context.model.clone(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .map_err(|e| errors::EvidenceEngineError::Storage {
        message: e.to_string(),
    })?;

    // 1. Extract claims
    let mut claims = claim_extractor::extract_claims(output_text, run_id);
    for claim in &mut claims {
        claim.task_type = task_type.to_string();
        if claim.agent_key.is_none() {
            claim.agent_key = context.agent_key.clone();
        }
        if claim.model.is_none() {
            claim.model = context.model.clone();
        }
    }

    // 2. Resolve evidence
    let evidence = evidence_resolver::resolve_evidence(db, &claims)?;

    // 3. Run gates
    let gate_results =
        verification_gates::run_all_gates(&claims, &evidence, run_id, task_type).await?;

    // 4. Update claims with verification status from gate results
    for claim in &mut claims {
        let is_blocked = gate_results
            .iter()
            .any(|g| g.blocked_claims.contains(&claim.claim_id));
        claim.verified = !is_blocked;
    }

    // 5. Assemble report
    let report = report_assembler::assemble_report(
        claims.clone(),
        evidence,
        &gate_results,
        run_id,
        Some(output_text),
    )?;

    // 6. Persist claims
    for claim in &report.claims {
        store::insert_completion_claim(db, claim).map_err(|e| {
            errors::EvidenceEngineError::Storage {
                message: e.to_string(),
            }
        })?;
    }

    // 7. Persist evidence
    for ev in &report.evidence {
        store::insert_completion_evidence(db, ev).map_err(|e| {
            errors::EvidenceEngineError::Storage {
                message: e.to_string(),
            }
        })?;
    }

    // 8. Persist gate results
    for gr in &gate_results {
        store::insert_gate_result(db, gr, run_id).map_err(|e| {
            errors::EvidenceEngineError::Storage {
                message: e.to_string(),
            }
        })?;
    }

    // 9. Persist report
    store::insert_completion_report(db, &report).map_err(|e| {
        errors::EvidenceEngineError::Storage {
            message: e.to_string(),
        }
    })?;

    // 10. Record false-completion incidents for any blocked claims with Failed state
    for claim in &report.claims {
        if !claim.verified {
            let actual_state = gate_results
                .iter()
                .find(|g| g.blocked_claims.contains(&claim.claim_id))
                .map(|g| g.resulting_state.clone())
                .unwrap_or(models::CompletionState::NotRun);

            let missing: Vec<models::EvidenceKind> = gate_results
                .iter()
                .filter(|g| g.blocked_claims.contains(&claim.claim_id))
                .flat_map(|g| g.required_missing_evidence.clone())
                .collect();

            if actual_state == models::CompletionState::Failed
                || actual_state == models::CompletionState::NotRun
            {
                let _ = incident_recorder::record_false_completion(
                    db,
                    claim,
                    actual_state,
                    missing,
                    None,
                );
            }
        }
    }

    // 11. Update trust scores from the source-of-truth claims/incidents rows.
    trust::recompute_trust_scores_for_run(db, run_id).map_err(|e| {
        errors::EvidenceEngineError::Storage {
            message: e.to_string(),
        }
    })?;

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cozo::DbInstance;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-completion-lib-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

    #[tokio::test]
    async fn test_full_completion_check_against_real_gt_run() {
        let db = test_db();

        // Set up gt_runs so evidence resolver can find it
        let run_id = "gt-abc123";
        let _ = db.run_script(
            &format!(
                "?[run_id, situation, started_at, completed_at, status] \
                 <- [[\"{run_id}\", \"test\", \"2026-01-01T00:00:00Z\", \
                 \"2026-01-01T00:01:00Z\", \"completed\"]] \
                 :put gt_runs {{ run_id => situation, started_at, completed_at, status }}"
            ),
            Default::default(),
            cozo::ScriptMutability::Mutable,
        );

        let output = "Task complete. All tests pass. Implementation is done. Build passes cleanly.";

        let report = check_completion_with_context(
            &db,
            run_id,
            output,
            "coding",
            CompletionContext {
                workspace_id: "workspace-test".into(),
                agent_key: Some("agent-test".into()),
                model: Some("model-test".into()),
            },
        )
        .await
        .unwrap();

        assert_eq!(report.run_id, run_id);
        assert!(!report.claims.is_empty(), "must extract claims from output");
        assert!(!report.evidence.is_empty(), "must resolve evidence");
        assert!(!report.calibrated_summary.is_empty());

        // Verify all 4 relations have rows
        let claims = store::get_completion_claims_by_run(&db, run_id).unwrap();
        assert!(!claims.is_empty(), "completion_claims must have rows");

        let trust_scores = store::find_trust_scores(&db, None, None).unwrap();
        assert!(
            !trust_scores.is_empty(),
            "completion verify must update agent_model_trust_scores"
        );
        assert_eq!(trust_scores[0].task_type, "coding");
        assert_eq!(trust_scores[0].workspace_id, "workspace-test");
        assert_eq!(trust_scores[0].agent_key.as_deref(), Some("agent-test"));
        assert_eq!(trust_scores[0].model.as_deref(), Some("model-test"));

        let evidence = store::get_evidence_by_run(&db, run_id).unwrap();
        assert!(!evidence.is_empty(), "completion_evidence must have rows");

        let report_row = store::get_completion_report(&db, &report.report_id).unwrap();
        assert!(report_row.is_some(), "completion_reports must have a row");

        // Verify uniqueness: report_id from check_completion matches stored
        let stored = report_row.unwrap();
        assert_eq!(stored.report_id, report.report_id);
    }

    #[tokio::test]
    async fn test_check_completion_async_does_not_panic_no_evidence() {
        let db = test_db();
        // No gt_runs, no evidence — call must return Ok, not panic.
        let report = check_completion(&db, "no-evidence-run", "All tests pass.", "coding")
            .await
            .unwrap();
        assert_eq!(report.run_id, "no-evidence-run");
        assert!(!report.claims.is_empty(), "must extract claim from output");
        // With no evidence, state should NOT be Verified.
        assert_ne!(report.final_state, crate::models::CompletionState::Verified);
    }

    #[tokio::test]
    async fn codex_completion_context_persists_provider_neutral_trust_metadata() {
        let db = test_db();
        let run_id = "codex-completion-run";

        let report = check_completion_with_context(
            &db,
            run_id,
            "Task complete. All tests pass.",
            "coding",
            CompletionContext {
                workspace_id: "workspace-codex".into(),
                agent_key: Some("codex-agent".into()),
                model: Some("gpt-5.4".into()),
            },
        )
        .await
        .unwrap();

        assert_eq!(report.run_id, run_id);
        assert!(
            report
                .claims
                .iter()
                .all(|claim| claim.agent_key.as_deref() == Some("codex-agent"))
        );
        assert!(
            report
                .claims
                .iter()
                .all(|claim| claim.model.as_deref() == Some("gpt-5.4"))
        );

        let contexts = store::get_all_completion_run_contexts(&db).unwrap();
        let context = contexts
            .iter()
            .find(|context| context.run_id == run_id)
            .expect("completion_run_contexts must include the Codex run");
        assert_eq!(context.workspace_id, "workspace-codex");
        assert_eq!(context.agent_key.as_deref(), Some("codex-agent"));
        assert_eq!(context.model.as_deref(), Some("gpt-5.4"));

        let trust_scores =
            store::find_trust_scores(&db, Some("codex-agent"), Some("gpt-5.4")).unwrap();
        assert_eq!(trust_scores.len(), 1);
        assert_eq!(trust_scores[0].workspace_id, "workspace-codex");
        assert_eq!(trust_scores[0].task_type, "coding");
    }
}
