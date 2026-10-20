use super::*;

pub(crate) fn current_seed_with_reply_markers(
    store: &WorkflowStore,
    run_id: &str,
    log_path: &Path,
    criteria: &BTreeMap<String, String>,
    requirement_texts: &BTreeMap<String, String>,
    resume_frozen_chain: Option<&serde_json::Value>,
    reply_leaked_markers: &[String],
) -> Result<Option<PhaseSeed>> {
    let Some((index, runtime)) = seeded_transition(store, run_id)? else {
        return Ok(None);
    };
    seed_for_transition(
        store,
        run_id,
        index,
        &runtime,
        criteria,
        requirement_texts,
        resume_frozen_chain,
        reply_leaked_markers,
        Some(log_path),
    )
}
