use super::*;

/// Run Tier 1 classification on a situation and persist the fingerprint.
///
/// When `llm` is provided, attempts real LLM-backed classification first.
/// Classify-only calls may use a labelled keyword fingerprint when LLM auth is
/// unavailable; full pipeline calls disable that fallback.
///
/// Returns the generated fingerprint after persistence.
pub async fn classify(
    db: &DbInstance,
    situation: &str,
    llm: Option<&dyn LlmClient>,
    project_root: &std::path::Path,
) -> Result<GameTheoryFingerprint, GameTheoryError> {
    classify_with_learning(db, situation, llm, None, project_root).await
}

pub async fn classify_with_learning(
    db: &DbInstance,
    situation: &str,
    llm: Option<&dyn LlmClient>,
    learning: Option<&mut LearningIntegration>,
    project_root: &std::path::Path,
) -> Result<GameTheoryFingerprint, GameTheoryError> {
    let (fingerprint, _) = classify_internal(
        db,
        situation,
        llm,
        &GameTheoryMemoryContext::default(),
        true,
        learning,
        project_root,
    )
    .await?;
    Ok(fingerprint)
}

pub(super) async fn classify_internal(
    db: &DbInstance,
    situation: &str,
    llm: Option<&dyn LlmClient>,
    memory_ctx: &GameTheoryMemoryContext,
    allow_keyword_fallback: bool,
    learning: Option<&mut LearningIntegration>,
    project_root: &std::path::Path,
) -> Result<(GameTheoryFingerprint, Vec<MemoryRecallAudit>), GameTheoryError> {
    let situation = situation.trim();
    if situation.is_empty() {
        return Err(GameTheoryError::EmptySituation);
    }

    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;

    let run_id = format!(
        "gt-{}",
        &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]
    );
    let now = Utc::now().to_rfc3339();

    // Insert run with status "running"
    insert_gt_run(db, &run_id, situation, &now, "running").map_err(|e| {
        GameTheoryError::Storage {
            message: e.to_string(),
        }
    })?;

    // Attempt real Tier 1 classification, fall back to keyword analysis
    let mut memory_audits = Vec::new();
    let fingerprint = if let Some(llm_client) = llm {
        match execute_tier1_real(
            llm_client,
            &run_id,
            situation,
            &now,
            memory_ctx,
            project_root,
            learning,
        )
        .await
        {
            Ok((fp, audits)) => {
                memory_audits.extend(audits);
                fp
            }
            Err(e) => {
                if allow_keyword_fallback {
                    tracing::warn!(run_id = %run_id, error = %e, "Tier 1 LLM classification failed, falling back to keyword");
                    keyword_fallback_fingerprint(&run_id, situation, &now)
                } else {
                    return Err(GameTheoryError::Tier1Execution {
                        message: e.to_string(),
                    });
                }
            }
        }
    } else if allow_keyword_fallback {
        keyword_fallback_fingerprint(&run_id, situation, &now)
    } else {
        return Err(GameTheoryError::LlmUnavailable {
            operation: "gametheory Tier 1 classification".to_string(),
        });
    };

    // Persist fingerprint
    let fingerprint_json =
        serde_json::to_string(&fingerprint).map_err(|e| GameTheoryError::FingerprintParse {
            message: e.to_string(),
        })?;

    insert_gt_fingerprint(
        db,
        &run_id,
        &fingerprint_json,
        &fingerprint.primary_family,
        &now,
    )
    .map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;

    // Update run status to completed
    let completed_at = Utc::now().to_rfc3339();
    update_gt_run_status(
        db,
        &run_id,
        situation,
        &now,
        &completed_at,
        "completed",
        0.0,
        0,
    )
    .map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;

    Ok((fingerprint, memory_audits))
}
