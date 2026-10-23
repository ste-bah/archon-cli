//! Resume-time seed inputs the live resume and its dry run share, so the dry
//! run plans the same calls the resume would.

use anyhow::Context;

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
    task_root: &Path,
    runtime: &FixedRunIdentityV1,
    (criteria, requirement_texts): (&BTreeMap<String, String>, &BTreeMap<String, String>),
    resume_frozen_chain: serde_json::Value,
    seed: Option<&PhaseSeed>,
    (expected_arguments, prd_text): (&serde_json::Value, &str),
    read_only: bool,
) -> Result<(serde_json::Value, ScriptSeed)> {
    let mut script_seed = resume_script_seed(
        store,
        run_id,
        runtime,
        criteria,
        requirement_texts,
        &resume_frozen_chain,
        seed,
        read_only,
    )?;
    if let Some(seed) = script_seed.seed.as_mut() {
        let records = archon_workflow::WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"))
            .load_call_records()?;
        if let Some(digest) =
            refresh_acceptance_carried_entries(task_root, &store.run_dir(run_id), seed, &records)?
        {
            script_seed
                .line
                .push_str(&format!(" acceptance_refreshed_from_contract={digest}"));
        }
    }
    let mut seeded = seeded_arguments(expected_arguments, script_seed.seed.as_ref());
    seeded["frozenChain"] = resume_frozen_chain;
    let arguments = crate::command::workflow_task_set::script_arguments_with_prd_requirement_texts(
        &seeded, prd_text,
    );
    #[cfg(test)]
    capture::record(&arguments);
    Ok((arguments, script_seed))
}

/// Refresh the acceptance entries in the seed handed to a resumed script.
/// Carried entries become the published ones. A reply stays an overlay only
/// while its published entry is unchanged since the reply was authored
/// (`publications`): a repair published after it — by a resume heal, an
/// operator re-author, a launch repair or an in-round republish — wins over
/// the reply's older bytes, and so does an entry with no such evidence.
pub(crate) fn refresh_acceptance_carried_entries(
    task_root: &Path,
    run_root: &Path,
    seed: &mut PhaseSeed,
    records: &[archon_workflow::WorkflowV2CallRecord],
) -> Result<Option<String>> {
    use archon_workflow::task_set_contract::{
        ACCEPTANCE_CONTRACT_FILE, AcceptanceContract, content_digest,
    };
    let path = task_root.join(ACCEPTANCE_CONTRACT_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let digest = content_digest(&bytes);
    let contract: AcceptanceContract =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    let live = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|entry| (entry.id.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    if let Some(SubjectSeed::Entries {
        carried_entries,
        replies,
        invalid,
        ..
    }) = seed.subjects.get_mut("acceptance")
    {
        let publications =
            publications::Publications::read(task_root, run_root, (&contract, &digest), records)?;
        replies.retain(|reply| {
            let Some(current) = live.get(&reply.id) else {
                return true;
            };
            if crate::command::workflow_decompose_seed::reply_entry(&reply.text, &reply.id)
                .is_none()
            {
                return true;
            }
            let reply_at = records
                .iter()
                .find(|record| record.call.id == reply.call_id)
                .and_then(|record| chrono::DateTime::parse_from_rfc3339(&record.started_at).ok());
            reply_at.is_some_and(|reply_at| {
                publications
                    .unchanged_since(current)
                    .is_some_and(|since| since <= reply_at)
            })
        });
        let mut refreshed = Vec::with_capacity(carried_entries.len());
        for old in carried_entries.drain(..) {
            let id = old
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if let Some(current) = live.get(id) {
                refreshed.push(serde_json::to_value(current)?);
            } else if !id.is_empty() {
                // Existing seed rules route invalid carried entries back to
                // authoring; preserving the id here records why it was dropped.
                invalid
                    .entry(id.to_string())
                    .or_default()
                    .push("carried acceptance entry is no longer in the published contract".into());
            }
        }
        *carried_entries = refreshed;
    }
    seed.acceptance_refreshed_from_contract_digest = Some(digest.clone());
    Ok(Some(digest))
}

#[path = "workflow_decompose_seed_publications.rs"]
mod publications;

/// Refresh the arguments that will run after resume checks may have healed
/// the acceptance contract. The refresh owns only the acceptance subject:
/// every other phase-seed field the resume already set (the remediation's
/// advanced author ordinals among them) is kept as it is.
pub(crate) fn refresh_after_resume_heal(
    task_root: &Path,
    run_root: &Path,
    arguments: &mut serde_json::Value,
    seed: Option<&mut PhaseSeed>,
    expected_arguments: &serde_json::Value,
) -> Result<()> {
    if let Some(seed) = seed {
        let records =
            archon_workflow::WorkflowV2ResultStore::new(run_root.join("v2")).load_call_records()?;
        refresh_acceptance_carried_entries(task_root, run_root, seed, &records)?;
        let refreshed = crate::command::workflow_decompose_seed::seeded_arguments(
            expected_arguments,
            Some(seed),
        );
        let acceptance = refreshed["phaseSeed"]["subjects"]
            .get("acceptance")
            .cloned();
        match arguments["phaseSeed"]["subjects"].as_object_mut() {
            Some(subjects) => match acceptance {
                Some(acceptance) => {
                    subjects.insert("acceptance".into(), acceptance);
                }
                None => {
                    subjects.remove("acceptance");
                }
            },
            None => arguments["phaseSeed"] = refreshed["phaseSeed"].clone(),
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "workflow_decompose_seed_resume_tests.rs"]
mod tests;

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
