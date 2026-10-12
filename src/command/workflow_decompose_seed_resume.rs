//! Resume-time seed inputs the live resume and its dry run share, so the dry
//! run plans the same calls the resume would.

use super::*;

/// The host-owned acceptance criteria the launch arguments carry.
pub(crate) fn host_criteria(arguments: &serde_json::Value) -> BTreeMap<String, String> {
    arguments["acceptanceCriteria"]
        .as_object()
        .map(|criteria| {
            criteria
                .iter()
                .filter_map(|(id, text)| Some((id.clone(), text.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Ordinal floors for a resume that enters verification without a harness
/// transition. This seed is runtime input only; the next resume derives new
/// floors from the durable call and pause records.
fn resume_floor_seed(
    store: &WorkflowStore,
    run_id: &str,
    runtime: &FixedRunIdentityV1,
    criteria: &BTreeMap<String, String>,
    requirement_texts: &BTreeMap<String, String>,
    resume_frozen_chain: &serde_json::Value,
) -> Result<PhaseSeed> {
    let generation = store.load_state(run_id)?.generation;
    derive_seed(
        store,
        run_id,
        (generation as usize, runtime.clone()),
        criteria,
        requirement_texts,
        Some(resume_frozen_chain),
    )
}

/// The seed handed to the script: the harness seed, if any, with live
/// ordinal floors whenever the frozen chain lets a phase be skipped.
pub(crate) fn resume_script_seed(
    store: &WorkflowStore,
    run_id: &str,
    runtime: &FixedRunIdentityV1,
    criteria: &BTreeMap<String, String>,
    requirement_texts: &BTreeMap<String, String>,
    resume_frozen_chain: &serde_json::Value,
    seed: Option<&PhaseSeed>,
) -> Result<Option<PhaseSeed>> {
    if resume_frozen_chain["acceptance"] != true && resume_frozen_chain["skeleton"] != true {
        return Ok(seed.cloned());
    }
    let floors = resume_floor_seed(
        store,
        run_id,
        runtime,
        criteria,
        requirement_texts,
        resume_frozen_chain,
    )?;
    Ok(Some(match seed {
        Some(seed) => {
            let mut refreshed = seed.clone();
            refreshed.author_ordinals = floors.author_ordinals;
            refreshed.pause_ordinals = floors.pause_ordinals;
            refreshed.pause_generation_floor = floors.pause_generation_floor;
            refreshed.history_attempts = floors.history_attempts;
            refreshed
        }
        None => floors,
    }))
}
