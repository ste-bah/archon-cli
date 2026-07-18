static ACTIVE_GUARDRAILS: OnceLock<Mutex<HashMap<String, VecDeque<RuntimeGuardrailRecord>>>> =
    OnceLock::new();
static ACTIVE_OBSERVATIONS: OnceLock<Mutex<HashMap<String, GuardrailRuntimeObservations>>> =
    OnceLock::new();
static ACTIVE_RECLASSIFICATION_FAILURES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
const HIGH_SURPRISE_STATUS_THRESHOLD: f32 = 0.30;

#[derive(Debug, Clone, Default)]
struct GuardrailRuntimeObservations {
    provider_incident_observed: bool,
    user_correction_observed: bool,
    plan_drift_observed: bool,
    reasoning_failure_observed: bool,
    retry_count: u32,
    evidence_refs: Vec<String>,
}

fn active_guardrails() -> &'static Mutex<HashMap<String, VecDeque<RuntimeGuardrailRecord>>> {
    ACTIVE_GUARDRAILS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn active_observations() -> &'static Mutex<HashMap<String, GuardrailRuntimeObservations>> {
    ACTIVE_OBSERVATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn active_reclassification_failures() -> &'static Mutex<HashMap<String, String>> {
    ACTIVE_RECLASSIFICATION_FAILURES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn reclassification_failure(action_id: &str) -> Option<String> {
    active_reclassification_failures()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(action_id)
        .cloned()
}

fn record_reclassification_failure(action_id: &str, message: String) {
    active_reclassification_failures()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(action_id.to_string(), message);
}

fn clear_reclassification_failure(action_id: &str) {
    active_reclassification_failures()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(action_id);
}

pub(crate) fn active_guardrail_for_session(session_id: &str) -> Option<RuntimeGuardrailRecord> {
    active_guardrails()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(session_id)
        .and_then(|records| records.front())
        .cloned()
}

fn active_guardrail_for_action(
    session_id: &str,
    action_id: &str,
) -> Option<RuntimeGuardrailRecord> {
    active_guardrails()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(session_id)
        .and_then(|records| {
            records
                .iter()
                .find(|record| record.action.action_id == action_id)
        })
        .cloned()
}

pub(crate) fn activate_guardrail_for_action(session_id: &str, action_id: &str) {
    let mut guard = active_guardrails()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(records) = guard.get_mut(session_id) else {
        return;
    };
    let Some(index) = records
        .iter()
        .position(|record| record.action.action_id == action_id)
    else {
        return;
    };
    if index != 0 {
        let record = records.remove(index).expect("located guardrail record");
        records.push_front(record);
    }
}

fn remember_active_guardrail(record: &RuntimeGuardrailRecord) {
    let mut guard = active_guardrails()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let records = guard.entry(record.action.session_id.clone()).or_default();
    if let Some(existing) = records
        .iter_mut()
        .find(|existing| existing.action.action_id == record.action.action_id)
    {
        *existing = record.clone();
    } else {
        records.push_back(record.clone());
    }
    drop(guard);
    active_observations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(record.action.action_id.clone())
        .or_default();
}

fn clear_active_guardrail(session_id: &str, action_id: &str) {
    let mut guard = active_guardrails()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let removed = guard.get_mut(session_id).is_some_and(|records| {
        let original_len = records.len();
        records.retain(|record| record.action.action_id != action_id);
        records.len() != original_len
    });
    if guard.get(session_id).is_some_and(VecDeque::is_empty) {
        guard.remove(session_id);
    }
    drop(guard);
    if removed {
        active_observations()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(action_id);
        clear_reclassification_failure(action_id);
    }
}

fn observations_for(action_id: &str) -> GuardrailRuntimeObservations {
    active_observations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(action_id)
        .cloned()
        .unwrap_or_default()
}

fn reclassify_active_guardrail_at_root(
    config: &archon_core::config::ArchonConfig,
    root: &std::path::Path,
    session_id: &str,
    action_id: &str,
    tool_name: &str,
    tool_use_id: &str,
    input: &serde_json::Value,
) {
    activate_guardrail_for_action(session_id, action_id);
    let policy = policy_from_config(config);
    let started_at = Instant::now();
    let revised = {
        let guard = active_guardrails()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(record) = guard.get(session_id).and_then(|records| records.front()) else {
            return;
        };
        if record.action.action_id != action_id || record.classified_from_tool {
            return;
        }

        let task_class =
            archon_world_model::classify_tool_action(tool_name, input, record.action.surface);
        let mode = archon_world_model::guardrail::mode_for_surface(&policy, record.action.surface);
        let scores = guardrail_scores_for_prediction(task_class, record.advisory.prediction.as_ref());
        let context = archon_world_model::WorldGuardrailPredictionContext::from_scores(
            task_class, mode, scores, &policy,
        );
        let mut decision = archon_world_model::guardrail::decide_guardrail(
            &record.action,
            record.advisory.prediction.as_ref(),
            context,
            &policy,
        );
        decision = archon_world_model::guardrail::enforce_guardrail_overhead_budget(
            decision,
            elapsed_ms_u64(started_at),
            policy.max_guardrail_overhead_ms,
        );
        decision.idempotency_key = format!(
            "world_guardrail:decision:{}:tool:{tool_use_id}",
            record.action.action_id
        );
        let mut revised = record.clone();
        revised.action.verification_plan =
            verification_plan_for_decision(&revised.action.action_id, &decision);
        revised.decision = decision;
        revised.task_class = task_class;
        revised.classified_from_tool = true;
        revised
    };

    let mut revised_action = revised.action.clone();
    revised_action.idempotency_key = format!(
        "world_guardrail:action:{}:tool:{tool_use_id}",
        revised.action.action_id
    );
    let revision_key = format!(
        "world_guardrail:revision:{}:tool:{tool_use_id}",
        revised.action.action_id
    );
    if let Err(error) = archon_world_model::guardrail::append_guardrail_revision(
        root,
        revised_action,
        revised.decision.clone(),
        revision_key,
    ) {
        tracing::warn!(
            %error,
            action_id = %revised.action.action_id,
            "failed to persist tool-classified guardrail revision"
        );
        record_reclassification_failure(
            &revised.action.action_id,
            format!("Tool-based guardrail reclassification could not be persisted: {error}"),
        );
        return;
    }
    clear_reclassification_failure(&revised.action.action_id);
    remember_active_guardrail(&revised);
}

pub(crate) fn reclassify_active_guardrail_for_session(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    action_id: &str,
    tool_name: &str,
    tool_use_id: &str,
    input: &serde_json::Value,
) {
    let root = match super::world_model_root() {
        Ok(root) => root,
        Err(error) => {
            record_reclassification_failure(
                action_id,
                format!("Tool-based guardrail reclassification storage is unavailable: {error}"),
            );
            return;
        }
    };
    reclassify_active_guardrail_at_root(
        config,
        &root,
        session_id,
        action_id,
        tool_name,
        tool_use_id,
        input,
    );
}

fn current_record_for_completion(stale: &RuntimeGuardrailRecord) -> RuntimeGuardrailRecord {
    active_guardrail_for_action(&stale.action.session_id, &stale.action.action_id)
        .unwrap_or_else(|| stale.clone())
}
