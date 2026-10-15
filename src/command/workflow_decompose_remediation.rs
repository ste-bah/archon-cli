//! Durable remediation ledger handling for fixed decomposition resumes.
use anyhow::{Context, Result, anyhow};
use std::{collections::HashSet, io::Write, path::Path};

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct Ledger {
    pub report_id: String,
    pub subjects: Vec<String>,
}

pub(crate) fn deduplicate_subjects(subjects: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    subjects
        .into_iter()
        .filter(|subject| seen.insert(subject.clone()))
        .collect()
}

pub(crate) fn read(run_dir: &Path, report_id: &str) -> Result<Option<Ledger>> {
    let path = run_dir.join(format!(
        "decomposition/reverify/remediation-{report_id}.json"
    ));
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let mut ledger: Ledger =
        serde_json::from_slice(&bytes).context("parsing reverify remediation ledger")?;
    ledger.subjects = deduplicate_subjects(ledger.subjects);
    if ledger.subjects.is_empty() {
        return Err(anyhow!("reverify remediation ledger has no subjects"));
    }
    Ok(Some(ledger))
}

pub(crate) fn force_subjects(arguments: &mut serde_json::Value, ledger: Option<&Ledger>) {
    if let Some(ledger) = ledger {
        arguments["forceBodySubjects"] = serde_json::json!(ledger.subjects);
    }
}

fn advance_forced_author_ordinals(
    run_dir: &Path,
    arguments: &mut serde_json::Value,
    ledger: &Ledger,
) -> Result<()> {
    let results = archon_workflow::WorkflowV2ResultStore::new(run_dir.join("v2"));
    let records = results.load_call_records()?;
    let seed = arguments
        .get_mut("phaseSeed")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| anyhow!("reverify remediation resume has no phase seed"))?;
    let ordinals = seed
        .entry("author_ordinals")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("reverify remediation phase seed has invalid author ordinals"))?;
    for subject in &ledger.subjects {
        let phase = format!("body-{subject}");
        let prefix = format!("{phase}-author-");
        let latest = records
            .iter()
            .filter_map(|record| record.call.id.strip_prefix(&prefix))
            .filter_map(|ordinal| ordinal.parse::<u64>().ok())
            .max();
        if let Some(latest) = latest {
            let seeded = ordinals
                .get(&phase)
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            ordinals.insert(phase, serde_json::json!(seeded.max(latest)));
        }
    }
    Ok(())
}

pub(crate) fn prepare(
    store: &archon_workflow::WorkflowStore,
    run_id: &str,
    run_dir: &Path,
    arguments: &mut serde_json::Value,
    runtime: &archon_workflow::FixedRunIdentityV1,
    criteria: &std::collections::BTreeMap<String, String>,
    requirement_texts: &std::collections::BTreeMap<String, String>,
) -> Result<Option<Ledger>> {
    let latest = crate::command::workflow_decompose_latest_report::latest_report(store, run_id)?;
    let ledger = match latest.as_ref() {
        Some(latest) => read(run_dir, &latest.id)?,
        None => None,
    };
    if let Some(latest) = latest
        && latest.outcome == "needs_remediation"
        && ledger
            .as_ref()
            .is_none_or(|ledger| ledger.report_id != latest.id)
    {
        return Err(anyhow!(
            "latest reverify report {} needs remediation but its matching ledger is missing",
            latest.id
        ));
    }
    force_subjects(arguments, ledger.as_ref());
    if let Some(ledger) = ledger.as_ref() {
        let missing_phase_seed = arguments
            .get("phaseSeed")
            .is_none_or(serde_json::Value::is_null);
        if let Some(seed) = missing_phase_seed
            .then(|| {
                crate::command::workflow_decompose_seed::completed_remediation_seed(
                    store,
                    run_id,
                    runtime,
                    criteria,
                    requirement_texts,
                    &arguments["frozenChain"],
                    Some(ledger),
                )
            })
            .transpose()?
            .flatten()
        {
            let frozen_chain = arguments["frozenChain"].clone();
            *arguments =
                crate::command::workflow_decompose_seed::seeded_arguments(arguments, Some(&seed));
            arguments["frozenChain"] = frozen_chain;
        }
        advance_forced_author_ordinals(run_dir, arguments, ledger)?;
    }
    Ok(ledger)
}

pub(crate) fn append_consumed(run_dir: &Path, ledger: &Ledger) -> Result<()> {
    let path = run_dir.join("decomposition/reverify/consumed.jsonl");
    std::fs::create_dir_all(path.parent().expect("ledger parent"))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    serde_json::to_writer(
        &mut file,
        &serde_json::json!({
            "report_id": ledger.report_id,
            "subjects": ledger.subjects,
            "consumed_at": chrono::Utc::now(),
        }),
    )?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

pub(crate) fn finish_resume(
    run_dir: &Path,
    ledger: Option<Ledger>,
    result: String,
) -> Result<String> {
    if let Some(ledger) = ledger {
        append_consumed(run_dir, &ledger)?;
    }
    Ok(result)
}

pub(crate) fn finish_resume_for_store(
    store: &archon_workflow::WorkflowStore,
    run_id: &str,
    ledger: Option<Ledger>,
    result: String,
) -> Result<String> {
    finish_resume(&store.run_dir(run_id), ledger, result)
}

#[cfg(test)]
#[path = "workflow_decompose_remediation_tests.rs"]
mod tests;
