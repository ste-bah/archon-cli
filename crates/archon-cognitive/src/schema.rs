use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::cozo_guard::run_script_guarded;
use crate::types::CognitiveError;

/// Column spec for `cognitive_tick_audit`, shared by the `:create` below and by
/// the `:replace` migration so the two shapes cannot drift apart. A macro
/// rather than a `const` because `concat!` only accepts literals.
///
/// `dead_letters_replayed` and `self_model_updated` are nullable because the
/// tick steps behind them are unimplemented: they measure nothing, and `null`
/// is the only value that says so without impersonating an observation.
macro_rules! tick_audit_spec {
    () => {
        "{
            tick_id: String =>
            dead_letters_replayed: Int?,
            proposals_evaluated: Int,
            proposals_auto_applied: Int,
            proposals_denied: Int,
            self_model_updated: Bool?,
            errors_json: String,
            duration_ms: Int,
            created_at: String,
        }"
    };
}

/// 2: `cognitive_tick_audit.dead_letters_replayed` and `.self_model_updated`
/// became nullable so a tick step that measured nothing stops being recorded as
/// a measured zero/success.
pub const CURRENT_SCHEMA_VERSION: i64 = 2;

pub fn ensure_cognitive_schema(db: &DbInstance) -> Result<(), CognitiveError> {
    for script in SCHEMA_RELATIONS {
        run_idempotent(db, script)?;
    }
    migrate_tick_audit_nullability(db)?;
    record_schema_version(db).or_else(|error| {
        if is_schema_version_relation_error(&error) {
            repair_schema_version_relation(db)?;
            record_schema_version(db)
        } else {
            Err(error)
        }
    })
}

pub fn cognitive_schema_version(db: &DbInstance) -> Result<i64, CognitiveError> {
    let rows = db
        .run_script(
            "?[version] := *cognitive_schema_version{version}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|err| CognitiveError::Schema(err.to_string()))?;
    rows.rows
        .first()
        .and_then(|row| row.first())
        .and_then(|value| value.get_int())
        .ok_or_else(|| CognitiveError::Schema("missing cognitive schema version".into()))
}

const SCHEMA_RELATIONS: &[&str] = &[
    r#":create cognitive_situations {
            situation_id: String =>
            session_id: String,
            turn_number: Int,
            user_text_hash: String,
            surface: String,
            kind: String,
            confidence_score: Float,
            confidence: String,
            evidence_refs: String,
            reason_summary: String,
            created_at: String,
        }"#,
    r#":create cognitive_tool_decisions {
            id: String =>
            situation_id: String,
            session_id: String,
            turn_number: Int,
            tool_name: String,
            verdict_json: String,
            reason: String,
            created_at: String,
        }"#,
    r#":create cognitive_action_candidates {
            candidate_id: String =>
            situation_id: String,
            action_kind: String,
            tool_name: String,
            risk: String,
            expected_evidence: String,
            expected_user_output: String,
            score: Float,
            score_source: String,
            rollback_path: String,
            rejected_reason: String,
            created_at: String,
        }"#,
    r#":create cognitive_decisions {
            decision_id: String =>
            situation_id: String,
            session_id: String,
            turn_number: Int,
            selected_candidate_id: String,
            rejected_candidates_json: String,
            heuristic_scores_json: String,
            policy_verdict_json: String,
            verification_contract_json: String,
            user_visible_summary: String,
            created_at: String,
        }"#,
    r#":create self_model_facts {
            fact_id: String =>
            domain: String,
            fact_kind: String,
            statement: String,
            confidence: Float,
            evidence_count: Int,
            last_seen_at: String,
            expires_at: String,
            created_at: String,
        }"#,
    r#":create cognitive_reflections {
            reflection_id: String =>
            session_id: String,
            turn_number: Int,
            decision_id: String,
            situation_kind: String,
            attempted: String,
            worked: String,
            failed: String,
            outcome: String,
            lesson: String,
            should_propose: Bool,
            proposed_rule_id: String,
            created_at: String,
        }"#,
    r#":create cognitive_prediction_links {
            link_id: String =>
            prediction_id: String,
            situation_id: String,
            decision_id: String,
            candidate_id: String,
            predicted_score: Float,
            actual_outcome: String,
            score_delta: Float,
            created_at: String,
        }"#,
    r#":create cognitive_policy_state {
            state_id: String =>
            rule_name: String,
            decision: String,
            reason: String,
            context_json: String,
            created_at: String,
        }"#,
    r#":create governed_proposals {
            proposal_id: String =>
            reflection_ids_json: String,
            manifest_kind: String,
            risk_level: String,
            evidence_count: Int,
            lesson_tag: String,
            domain: String,
            diff_summary: String,
            rollback_plan: String,
            created_at: String,
        }"#,
    r#":create autonomous_apply_results {
            apply_id: String =>
            proposal_id: String,
            result_kind: String,
            reason: String,
            canary_outcome_ref: String,
            rollback_ref: String,
            created_at: String,
        }"#,
    r#":create canary_outcomes {
            canary_id: String =>
            proposal_id: String,
            passed: Bool,
            details: String,
            snapshot_ref: String,
            created_at: String,
        }"#,
    concat!(":create cognitive_tick_audit ", tick_audit_spec!()),
    // R8 measurement foundation. `cognitive_metric_events` is append-only and
    // is the source of truth for every derived metric; `fingerprint` lets a
    // replayed write be told apart from a conflicting rewrite of the same id.
    // The event-kind-specific identity columns from the roadmap schema live in
    // `identities_json`, enforced per kind by `MetricEventKind`, rather than as
    // seventy mostly-null columns.
    r#":create cognitive_metric_events {
            metric_event_id: String =>
            idempotency_key: String,
            fingerprint: String,
            metric_name: String,
            metric_definition_version: Int,
            evaluation_dataset_version: String,
            evaluation_window_id: String,
            event_kind: String,
            session_id: String,
            turn_number: Int,
            task_class: String,
            model_id: String,
            policy_version: String,
            label_source: String,
            outcome_status: String,
            value: Float?,
            numerator: Float?,
            denominator: Float?,
            identities_json: String,
            evidence_refs_json: String,
            created_at: String,
        }"#,
    r#":create cognitive_evaluation_windows {
            evaluation_window_id: String =>
            label: String,
            started_at: String,
            ended_at: String,
            population_query_version: Int,
            segmentation_keys_json: String,
            cohort_role: String,
            cohort_identity: String,
            metric_definition_version: Int,
            created_at: String,
        }"#,
    SCHEMA_VERSION_RELATION,
];

const SCHEMA_VERSION_RELATION: &str = r#":create cognitive_schema_version {
        version: Int =>
        created_at: String,
    }"#;

/// Widen the two unmeasured tick-audit columns on databases created before
/// schema version 2.
///
/// `:create` is a no-op once a relation exists, so without this an installed
/// database would keep non-nullable columns and reject every tick write.
///
/// The two columns are rewritten to `null` for existing rows instead of being
/// carried over: under version 1 the tick hardcoded them to `0` and `true`, so
/// every historical value is a fabrication rather than an observation, and
/// preserving them would keep exactly the lie this migration exists to remove.
fn migrate_tick_audit_nullability(db: &DbInstance) -> Result<(), CognitiveError> {
    if !tick_audit_is_legacy(db) {
        return Ok(());
    }
    run_script_guarded(
        db,
        concat!(
            "?[tick_id, dead_letters_replayed, proposals_evaluated, proposals_auto_applied, \
             proposals_denied, self_model_updated, errors_json, duration_ms, created_at] := \
             *cognitive_tick_audit{tick_id, proposals_evaluated, proposals_auto_applied, \
             proposals_denied, errors_json, duration_ms, created_at}, \
             dead_letters_replayed = null, self_model_updated = null
             :replace cognitive_tick_audit ",
            tick_audit_spec!()
        ),
        Default::default(),
        ScriptMutability::Mutable,
        "migrate cognitive tick audit to nullable measurement columns",
    )
    .map(|_| ())
    .map_err(|error| CognitiveError::Schema(error.to_string()))
}

/// The relation name and key are unchanged across the two versions, so the
/// declared column type is the only evidence of which one is on disk.
fn tick_audit_is_legacy(db: &DbInstance) -> bool {
    let Ok(columns) = run_script_guarded(
        db,
        "::columns cognitive_tick_audit",
        Default::default(),
        ScriptMutability::Immutable,
        "inspect cognitive tick audit columns",
    ) else {
        // No relation, nothing to migrate; creation above already reported any
        // real failure.
        return false;
    };
    columns.rows.iter().any(|row| {
        row.first().and_then(DataValue::get_str) == Some("dead_letters_replayed")
            && row.get(3).and_then(DataValue::get_str) == Some("Int")
    })
}

/// `:replace` rather than `:put` because the relation is keyed by `version`:
/// putting a bumped version would leave the superseded row in place, and
/// [`cognitive_schema_version`] reads the first row, so an upgraded database
/// would report the version it used to be.
fn record_schema_version(db: &DbInstance) -> Result<(), CognitiveError> {
    let created_at = chrono::Utc::now().to_rfc3339();
    let script = format!(
        "?[version, created_at] <- [[{}, '{}']]
         :replace cognitive_schema_version {{ version => created_at }}",
        CURRENT_SCHEMA_VERSION, created_at
    );
    run_idempotent(db, script.as_str())
}

fn repair_schema_version_relation(db: &DbInstance) -> Result<(), CognitiveError> {
    run_idempotent(db, "{::remove cognitive_schema_version}")?;
    run_idempotent(db, SCHEMA_VERSION_RELATION)
}

fn is_schema_version_relation_error(error: &CognitiveError) -> bool {
    matches!(
        error,
        CognitiveError::Schema(message)
            if message.contains("cognitive_schema_version")
                || message.contains("required column created_at not found")
                || message.contains("when executing against relation")
    )
}

fn run_idempotent(db: &DbInstance, script: &str) -> Result<(), CognitiveError> {
    match crate::cozo_guard::run_script_guarded(
        db,
        script,
        Default::default(),
        ScriptMutability::Mutable,
        "initialize cognitive schema",
    ) {
        Ok(_) => Ok(()),
        Err(error) => {
            let message = error.to_string();
            if message.contains("already exists") || message.contains("conflicts") {
                Ok(())
            } else {
                Err(CognitiveError::Schema(message))
            }
        }
    }
}
