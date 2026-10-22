//! Durable remediation ledger handling for fixed decomposition resumes.
use anyhow::{Context, Result, anyhow};
use std::{collections::HashSet, io::Write, path::Path};

pub(crate) fn resume_author_scope(
    project_root: &Path,
    task_root: &Path,
    prd_path: &Path,
) -> Result<crate::command::workflow_task_set::reauthor::AuthorScope> {
    Ok(
        crate::command::workflow_task_set::reauthor::AuthorScope::for_task_set(
            project_root,
            task_root,
            prd_path,
        )
        .map_err(anyhow::Error::msg)?,
    )
}

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

pub(crate) fn is_consumed(run_dir: &Path, report_id: &str) -> Result<bool> {
    let path = run_dir.join("decomposition/reverify/consumed.jsonl");
    if !path.exists() {
        return Ok(false);
    }
    Ok(std::fs::read_to_string(&path)?
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .any(|row| row["report_id"].as_str() == Some(report_id)))
}

/// A consumed ledger is reusable only while the live pin still proves the
/// ledger's check repairs. A whole-set publication can reset republish
/// lineage; in that case the records cannot distinguish a repaired entry
/// from an older version, so resume sends the named entries through the
/// ordinary judge-and-host-proof republish path again.
fn consumed_ledger_findings(
    project_root: &Path,
    task_root: &Path,
    ledger: &Ledger,
) -> Result<std::collections::BTreeMap<String, String>> {
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project_root, task_root);
    let pin = std::fs::read(&pin_path).ok().and_then(|bytes| {
        serde_json::from_slice::<archon_workflow::task_set_contract::AcceptancePin>(&bytes).ok()
    });
    let contract_path =
        task_root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let contract_bytes = std::fs::read(&contract_path)?;
    let current_digest = archon_workflow::task_set_contract::content_digest(&contract_bytes);
    let current: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&contract_bytes)?;
    let Some(pin) = pin else {
        return Ok(ledger.subjects.iter().map(|id| (
            id.clone(), format!("consumed reverify ledger {} names this check, but current pin {} is missing or unreadable; the current repaired bytes cannot be proven from pin lineage or filed chain history, so the check is being re-authored", ledger.report_id, pin_path.display())
        )).collect());
    };
    let history = archon_workflow::task_set_lineage::ChainHistory::for_pin(&pin_path);
    let mut findings = std::collections::BTreeMap::new();
    let lineage_ends_at_pin = pin
        .lineage
        .last()
        .is_some_and(|link| link.to == pin.identity());
    for id in &ledger.subjects {
        let current_entry = current
            .acceptance
            .iter()
            .chain(&current.supplementary)
            .find(|entry| &entry.id == id);
        let mut proof = None;
        let mut proves_current_repair = false;
        if lineage_ends_at_pin {
            for transition in pin
                .lineage
                .iter()
                .filter(|link| link.reauthored_ids.contains(id))
            {
                let before = history
                    .get(&transition.from.acceptance_digest)
                    .ok()
                    .flatten()
                    .and_then(|bytes| {
                        serde_json::from_slice::<
                            archon_workflow::task_set_contract::AcceptanceContract,
                        >(&bytes)
                        .ok()
                    });
                let after = if transition.to.acceptance_digest == current_digest {
                    Some(current.clone())
                } else {
                    history
                        .get(&transition.to.acceptance_digest)
                        .ok()
                        .flatten()
                        .and_then(|bytes| {
                            serde_json::from_slice::<
                                archon_workflow::task_set_contract::AcceptanceContract,
                            >(&bytes)
                            .ok()
                        })
                };
                let before_entry = before.as_ref().and_then(|contract| {
                    contract
                        .acceptance
                        .iter()
                        .chain(&contract.supplementary)
                        .find(|entry| &entry.id == id)
                });
                let after_entry = after.as_ref().and_then(|contract| {
                    contract
                        .acceptance
                        .iter()
                        .chain(&contract.supplementary)
                        .find(|entry| &entry.id == id)
                });
                if before_entry
                    .zip(after_entry)
                    .is_some_and(|(before, after)| before != after)
                    && current_entry == before_entry
                {
                    proof = Some(format!(
                        "current entry {id} matches the replaced version in transition {} -> {}, while the filed successor records different bytes",
                        transition.from.acceptance_digest, transition.to.acceptance_digest
                    ));
                    break;
                }
                if pin.acceptance_digest == current_digest
                    && before_entry
                        .zip(after_entry)
                        .is_some_and(|(before, after)| before != after)
                    && current_entry == after_entry
                {
                    proves_current_repair = true;
                }
            }
        }
        if let Some(evidence) = proof {
            findings.insert(id.clone(), format!("consumed reverify ledger {} and pin lineage prove a reverted entry: {evidence}; re-authoring it through judge and host proof", ledger.report_id));
        } else if !proves_current_repair {
            findings.insert(id.clone(), format!("consumed reverify ledger {} names this check, but current contract digest {current_digest} has no matching republish lineage proving the repaired entry; the available pin/history evidence is insufficient to establish its bytes, so re-authoring it through judge and host proof", ledger.report_id));
        }
    }
    Ok(findings)
}

fn validate_subjects(
    run_dir: &Path,
    ledger: &Ledger,
    task_ids: &std::collections::BTreeSet<String>,
    check_ids: &std::collections::BTreeSet<String>,
) -> Result<()> {
    let unknown = ledger
        .subjects
        .iter()
        .filter(|subject| !task_ids.contains(*subject) && !check_ids.contains(*subject))
        .cloned()
        .collect::<Vec<_>>();
    if unknown.is_empty() {
        return Ok(());
    }
    let path = run_dir.join(format!(
        "decomposition/reverify/remediation-{}.json",
        ledger.report_id
    ));
    Err(anyhow!(
        "reverify remediation ledger {} contains unknown subject(s): {}",
        path.display(),
        unknown.join(", ")
    ))
}

pub(crate) fn force_subjects(
    arguments: &mut serde_json::Value,
    ledger: Option<&Ledger>,
    task_ids: &std::collections::BTreeSet<String>,
) {
    if let Some(ledger) = ledger {
        let tasks = ledger
            .subjects
            .iter()
            .filter(|subject| task_ids.contains(*subject))
            .collect::<Vec<_>>();
        arguments["forceBodySubjects"] = serde_json::json!(tasks);
    }
}

pub(crate) async fn reauthor_check_subjects_on_resume(
    (project_root, prd_path, task_root, run_dir): (&Path, &Path, &Path, &Path),
    ledger: Option<&Ledger>,
    client: &dyn archon_workflow::WorkflowLlmClient,
    cargo_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
) -> Result<()> {
    let Some(ledger) = ledger else {
        return Ok(());
    };
    if is_consumed(run_dir, &ledger.report_id)?
        && consumed_ledger_findings(project_root, task_root, ledger)?.is_empty()
    {
        return Ok(());
    }
    let contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&std::fs::read(
            task_root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        )?)?;
    let ids = ledger
        .subjects
        .iter()
        .filter(|subject| {
            contract
                .acceptance
                .iter()
                .chain(&contract.supplementary)
                .any(|entry| &entry.id == *subject)
        })
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if ids.is_empty() {
        return Ok(());
    }
    let report_path = run_dir.join(format!(
        "decomposition/reverify/reports/{}.json",
        ledger.report_id
    ));
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(report_path)?)?;
    let uncertain = if is_consumed(run_dir, &ledger.report_id)? {
        consumed_ledger_findings(project_root, task_root, ledger)?
    } else {
        std::collections::BTreeMap::new()
    };
    let seeds = ids
        .iter()
        .map(|id| {
            let finding = uncertain
                .get(id)
                .cloned()
                .or_else(|| {
                    report["checks"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|check| check["check_id"].as_str() == Some(id.as_str()))
                        .and_then(|check| check["findings"].as_array())
                        .and_then(|findings| findings.first())
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "the frozen check failed on the current tree".to_string());
            (id.clone(), finding)
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let scope = resume_author_scope(project_root, task_root, prd_path)
        .map_err(|error| anyhow::anyhow!(error))?;
    let probe =
        crate::command::workflow_task_set::executability::HostProbe::for_task_set_with_limits(
            project_root,
            task_root,
            cargo_limits,
        );
    crate::command::workflow_task_set::republish::reauthor_and_republish(
        client,
        crate::command::workflow_task_set::republish::ReauthorRequest {
            project_root,
            tasks_root: task_root,
            prd_path,
            ids: &ids,
            gate: crate::command::workflow_task_set::reauthor::ReauthorGate {
                probe: &probe,
                seeds: &seeds,
            },
            trigger: "workflow resume: reverify routed a check defect to its frozen check entry",
        },
        &scope,
    )
    .await?;
    Ok(())
}

pub(crate) async fn prepare_resume_checks(
    (project_root, prd_path, task_root, run_dir): (&Path, &Path, &Path, &Path),
    target_root: Option<&Path>,
    ledger: Option<&Ledger>,
    client: &dyn archon_workflow::WorkflowLlmClient,
    cargo_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
) -> Result<()> {
    crate::command::workflow_read_scope::require_author_reads(
        client,
        "the decomposition authors",
        target_root,
        run_dir,
    )?;
    reauthor_check_subjects_on_resume(
        (project_root, prd_path, task_root, run_dir),
        ledger,
        client,
        cargo_limits,
    )
    .await
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
    if let Some(ledger) = ledger.as_ref() {
        if is_consumed(run_dir, &ledger.report_id)?
            && consumed_ledger_findings(
                Path::new(&runtime.project_root_identity),
                Path::new(&runtime.task_root_identity),
                ledger,
            )?
            .is_empty()
        {
            arguments["forceBodySubjects"] = serde_json::json!([]);
            return Ok(None);
        }
        let skeleton: archon_workflow::task_skeleton::TaskSkeleton =
            serde_json::from_slice(&std::fs::read(
                Path::new(&runtime.task_root_identity)
                    .join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
            )?)?;
        let task_ids = skeleton
            .tasks
            .iter()
            .map(|task| task.task_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let contract: archon_workflow::task_set_contract::AcceptanceContract =
            serde_json::from_slice(&std::fs::read(
                Path::new(&runtime.task_root_identity)
                    .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
            )?)?;
        let check_ids = contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .map(|entry| entry.id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        validate_subjects(run_dir, ledger, &task_ids, &check_ids)?;
        force_subjects(arguments, Some(ledger), &task_ids);
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
    } else {
        force_subjects(arguments, None, &Default::default());
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
