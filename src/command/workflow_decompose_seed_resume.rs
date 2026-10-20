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

/// The floor seed of a run with no harness transition, for one harness.
fn floor_seed_path(runtime: &FixedRunIdentityV1) -> String {
    format!("{SEEDS_DIR}/resume-floors-{}.json", runtime_key(runtime))
}

fn read_floor_seed(
    store: &WorkflowStore,
    run_id: &str,
    runtime: &FixedRunIdentityV1,
) -> Result<Option<PhaseSeed>> {
    let relative = floor_seed_path(runtime);
    let raw = match std::fs::read(store.run_dir(run_id).join(&relative)) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(seed_unmapped(&relative, &error.to_string())),
    };
    let value: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|e| seed_unmapped(&relative, &e.to_string()))?;
    if value["schema_version"] != SEED_SCHEMA_VERSION {
        return Err(seed_unmapped(
            &format!("{relative}.schema_version"),
            &format!(
                "found {}; this binary reads schema {SEED_SCHEMA_VERSION}",
                value["schema_version"]
            ),
        ));
    }
    let seed: PhaseSeed = crate::command::workflow_decompose::decode(value, &relative)?;
    if transitions::harness_changed(&seed.runtime, runtime) {
        return Err(seed_unmapped(
            &format!("{relative}.runtime"),
            "the floor seed was derived for another script, catalog or template",
        ));
    }
    Ok(Some(seed))
}

/// Replaced atomically and durably, as a harness seed is.
fn write_floor_seed(store: &WorkflowStore, run_id: &str, seed: &PhaseSeed) -> Result<()> {
    use crate::command::workflow_task_set::{create_dir_all_durably, sync_parent};
    let relative = floor_seed_path(&seed.runtime);
    let seeds = store.run_dir(run_id).join(SEEDS_DIR);
    create_dir_all_durably(&seeds)?;
    sync_parent(&seeds)?;
    store.write_run_json(run_id, &relative, seed)?;
    sync_parent(&store.run_dir(run_id).join(relative))
}

/// The seed a resume hands the script, and one line saying where it came
/// from and the floors it carries.
pub(crate) struct ScriptSeed {
    pub(crate) seed: Option<PhaseSeed>,
    pub(crate) line: String,
}

/// The seed handed to the script. A harness seed is used as it was derived:
/// its subjects and floors come from one derivation, fixed once per
/// transition (Issue 360), so attempts after it replay on every later
/// resume. A run with no harness seed whose frozen chain lets a phase be
/// skipped gets a floor seed derived ONCE, at the first such resume for its
/// harness, and read back on every later one in the same way. A read-only
/// caller derives a missing one in memory and records nothing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resume_script_seed(
    store: &WorkflowStore,
    run_id: &str,
    runtime: &FixedRunIdentityV1,
    criteria: &BTreeMap<String, String>,
    requirement_texts: &BTreeMap<String, String>,
    resume_frozen_chain: &serde_json::Value,
    seed: Option<&PhaseSeed>,
    read_only: bool,
) -> Result<ScriptSeed> {
    let (seed, source) = if let Some(seed) = seed {
        (
            Some(seed.clone()),
            format!("harness-transition-{}", seed.transition_index),
        )
    } else if resume_frozen_chain["acceptance"] != true && resume_frozen_chain["skeleton"] != true {
        (None, "none".to_string())
    } else if let Some(stored) = read_floor_seed(store, run_id, runtime)? {
        (
            Some(stored),
            format!("floor-seed {}", floor_seed_path(runtime)),
        )
    } else {
        let generation = store.load_state(run_id)?.generation;
        let derived = derive_seed(
            store,
            run_id,
            (generation as usize, runtime.clone()),
            criteria,
            requirement_texts,
            Some(resume_frozen_chain),
            &[],
        )?;
        let source = if read_only {
            format!(
                "floor-seed derived in memory (would record {})",
                floor_seed_path(runtime)
            )
        } else {
            write_floor_seed(store, run_id, &derived)?;
            format!("floor-seed recorded {}", floor_seed_path(runtime))
        };
        (Some(derived), source)
    };
    let line = match &seed {
        None => format!("script_seed source={source}"),
        Some(seed) => {
            let ordinals = |map: &BTreeMap<String, u64>| {
                map.iter()
                    .map(|(subject, ordinal)| format!("{subject}:{ordinal}"))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            format!(
                "script_seed source={source} derived_at={} pause_generation_floor={} history_attempts={} author_ordinals={} pause_ordinals={}",
                seed.derived_at,
                seed.pause_generation_floor,
                seed.history_attempts.len(),
                ordinals(&seed.author_ordinals),
                ordinals(&seed.pause_ordinals)
            )
        }
    };
    Ok(ScriptSeed { seed, line })
}

/// The script arguments a resume runs: launch arguments with the script
/// seed, the live frozen-chain decision (on every resume, even without a seed
/// or when an older seed carries a stale value) and the PRD requirement
/// texts. The live resume and its dry run both build them here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resume_script_arguments(
    store: &WorkflowStore,
    run_id: &str,
    runtime: &FixedRunIdentityV1,
    (criteria, requirement_texts): (&BTreeMap<String, String>, &BTreeMap<String, String>),
    resume_frozen_chain: serde_json::Value,
    seed: Option<&PhaseSeed>,
    (expected_arguments, prd_text): (&serde_json::Value, &str),
    read_only: bool,
) -> Result<(serde_json::Value, ScriptSeed)> {
    let script_seed = resume_script_seed(
        store,
        run_id,
        runtime,
        criteria,
        requirement_texts,
        &resume_frozen_chain,
        seed,
        read_only,
    )?;
    let mut seeded = seeded_arguments(expected_arguments, script_seed.seed.as_ref());
    seeded["frozenChain"] = resume_frozen_chain;
    let arguments = crate::command::workflow_task_set::script_arguments_with_prd_requirement_texts(
        &seeded, prd_text,
    );
    #[cfg(test)]
    capture::record(&arguments);
    Ok((arguments, script_seed))
}

/// The script arguments the last resume on this thread built (tests).
#[cfg(test)]
pub(crate) mod capture {
    thread_local! {
        static LAST: std::cell::RefCell<Option<serde_json::Value>> =
            const { std::cell::RefCell::new(None) };
    }

    pub(super) fn record(arguments: &serde_json::Value) {
        LAST.with(|last| *last.borrow_mut() = Some(arguments.clone()));
    }

    pub(crate) fn take() -> Option<serde_json::Value> {
        LAST.with(|last| last.borrow_mut().take())
    }
}
