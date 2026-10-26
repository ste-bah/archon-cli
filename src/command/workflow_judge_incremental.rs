//! Reuse only unchanged accepted judgments saved for their exact inputs.
use super::*;
use archon_workflow::task_set_contract::{AcceptanceCriterion, JudgeDecision};

#[derive(serde::Serialize, serde::Deserialize)]
struct EntryVerdict {
    key: String,
    judgment: archon_workflow::task_set_contract::JudgeVerdict,
}

#[cfg(test)]
pub(super) fn entry_key(
    client: &dyn WorkflowLlmClient,
    tasks_root: &Path,
    prd_digest: &str,
    entry: &AcceptanceCriterion,
    roots: &super::judge::JudgeRoots,
) -> Option<String> {
    let mut remaining = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    entry_key_with_budget(client, tasks_root, prd_digest, entry, roots, &mut remaining)
}

fn entry_key_with_budget(
    client: &dyn WorkflowLlmClient,
    tasks_root: &Path,
    prd_digest: &str,
    entry: &AcceptanceCriterion,
    roots: &super::judge::JudgeRoots,
    remaining: &mut usize,
) -> Option<String> {
    let named_file_digest = super::judge::named_files_digest_with_budget(entry, roots, remaining);
    // Freeze-acceptance's closure digest is the authority for the full
    // judgment dependency graph, including prompt and verdict handling.
    let (version, digest, _) =
        crate::command::workflow_task_set::workflow_acceptance_check_reuse::logic_identity()?;
    Some(content_digest(
        serde_json::json!([
            "acceptance-entry-judge-v1",
            tasks_root,
            prd_digest,
            entry.criterion,
            entry.check,
            entry.covers,
            entry.gap_permitted,
            named_file_digest,
            client.resolve_model_alias("sonnet"),
            client.provider_id(),
            version,
            digest,
        ])
        .to_string()
        .as_bytes(),
    ))
}

fn sampling_matches(client: &dyn WorkflowLlmClient, entry: &AcceptanceCriterion) -> bool {
    entry.judgment.sampling.as_ref().is_some_and(|sampling| {
        sampling["model"] == serde_json::json!(client.resolve_model_alias("sonnet"))
            && sampling["provider"] == serde_json::json!(client.provider_id())
    })
}

fn current_logic_identity(client: &dyn WorkflowLlmClient) -> Option<serde_json::Value> {
    crate::command::workflow_task_set::workflow_acceptance_check_reuse::logic_identity().map(
        |logic| {
            crate::command::workflow_task_set::reauthor::cache::acceptance_logic_identity(
                logic,
                &client.resolve_model_alias("sonnet"),
                client.provider_id().as_deref(),
            )
        },
    )
}

pub(super) fn keep_pinned_entries(
    contract: &mut AcceptanceContract,
    base: &AcceptanceContract,
    client: &dyn WorkflowLlmClient,
    roots: &super::judge::JudgeRoots,
) -> BTreeSet<String> {
    if contract.prd.digest != base.prd.digest {
        return BTreeSet::new();
    }
    let Some(identity) = current_logic_identity(client) else {
        return BTreeSet::new();
    };
    let mut restored = BTreeSet::new();
    for entry in contract
        .acceptance
        .iter_mut()
        .chain(&mut contract.supplementary)
    {
        if entry.judgment.verdict == JudgeDecision::Accepted {
            continue;
        }
        let Some(old) = base
            .acceptance
            .iter()
            .chain(&base.supplementary)
            .find(|old| old.id == entry.id)
        else {
            continue;
        };
        if old.judgment.verdict == JudgeDecision::Accepted
            && old
                .judgment
                .sampling
                .as_ref()
                .and_then(|sampling| sampling.get("logic_identity"))
                == Some(&identity)
            && old
                .judgment
                .sampling
                .as_ref()
                .and_then(|sampling| sampling.get("named_file_digest"))
                .and_then(serde_json::Value::as_str)
                == Some(super::judge::named_files_digest(old, roots).as_str())
            && old.criterion == entry.criterion
            && old.check == entry.check
            && old.covers == entry.covers
            && old.gap_permitted == entry.gap_permitted
            && sampling_matches(client, old)
        {
            *entry = old.clone();
            restored.insert(entry.id.clone());
        }
    }
    restored
}

pub(super) fn save_judged_entries(
    project: &Path,
    tasks_root: &Path,
    prd_digest: &str,
    client: &dyn WorkflowLlmClient,
    contract: &AcceptanceContract,
    judged_ids: &BTreeSet<String>,
    pinned_ids: &BTreeSet<String>,
    roots: &super::judge::JudgeRoots,
) -> Result<()> {
    let mut remaining = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    for entry in contract.acceptance.iter().chain(&contract.supplementary) {
        if !judged_ids.contains(&entry.id) {
            continue;
        }
        let key =
            entry_key_with_budget(client, tasks_root, prd_digest, entry, roots, &mut remaining);
        if !pinned_ids.contains(&entry.id)
            && entry.judgment.verdict == JudgeDecision::Accepted
            && sampling_matches(client, entry)
            && let Some(key) = key
        {
            save_entry(project, &key, entry)?;
        }
    }
    Ok(())
}

fn entry_cache(project: &Path, key: &str) -> PathBuf {
    project
        .join(crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR)
        .join("judge-entry")
        .join(format!("{key}.json"))
}

pub(super) fn load_entry(
    project: &Path,
    key: &str,
) -> Option<archon_workflow::task_set_contract::JudgeVerdict> {
    let saved: EntryVerdict =
        serde_json::from_slice(&std::fs::read(entry_cache(project, key)).ok()?).ok()?;
    (saved.key == key).then_some(saved.judgment)
}

fn save_entry(project: &Path, key: &str, entry: &AcceptanceCriterion) -> Result<()> {
    let path = entry_cache(project, key);
    let parent = path.parent().context("judge entry cache has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("could not create judge entry cache {}", parent.display()))?;
    let saved = EntryVerdict {
        key: key.to_string(),
        judgment: entry.judgment.clone(),
    };
    let bytes = serde_json::to_vec(&saved).context("could not encode a saved judge entry")?;
    let staging = path.with_extension(format!("{}.tmp", std::process::id()));
    if let Err(error) = std::fs::write(&staging, bytes) {
        return Err(error)
            .with_context(|| format!("could not write judge entry cache {}", staging.display()));
    }
    if let Err(error) = std::fs::rename(&staging, &path) {
        let _ = std::fs::remove_file(&staging);
        return Err(error)
            .with_context(|| format!("could not publish judge entry cache {}", path.display()));
    }
    Ok(())
}

pub(super) async fn judge(
    project: &Path,
    tasks: &Path,
    client: &dyn WorkflowLlmClient,
    mut contract: AcceptanceContract,
    expected: &BTreeSet<String>,
    store: Option<&super::judge_store::JudgeStore>,
    roots: &super::judge::JudgeRoots,
) -> Result<AcceptanceContract> {
    let tasks_root = tasks
        .canonicalize()
        .with_context(|| format!("could not canonicalize task root {}", tasks.display()))?;
    // A candidate's own judgment fields never authorize reuse. Failed validation
    // of historical evidence means rejudge, not silently trust its contents.
    // Issue 294: pin and bundle are one version. A set that cannot be read
    // as one stops the freeze with the reason (Issue 338), never a silent
    // rejudge of everything over a publish nobody settled.
    let read =
        crate::command::workflow_task_set::ChainRead::of(project, tasks).map_err(|error| {
            error.context("the previous freeze's judgments cannot be read as one version")
        })?;
    let base = std::fs::read(acceptance_pin_path(project, tasks))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AcceptancePin>(&bytes).ok())
        .and_then(|pin| validate_acceptance_bundle(tasks, Some(&pin), expected).ok());
    drop(read);
    let mut reused = BTreeSet::new();
    let mut remaining = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    for entry in contract
        .acceptance
        .iter_mut()
        .chain(&mut contract.supplementary)
    {
        let before_entry = remaining;
        let Some(key) = entry_key_with_budget(
            client,
            &tasks_root,
            &contract.prd.digest,
            entry,
            roots,
            &mut remaining,
        ) else {
            continue;
        };
        // The exact task root and PRD digest scope this verdict to its history.
        if let Some(judgment) = load_entry(project, &key) {
            if judgment.verdict == JudgeDecision::Accepted {
                entry.judgment = judgment;
                reused.insert(entry.id.clone());
                remaining = before_entry;
            }
        }
    }
    let mut subset = contract.clone();
    subset
        .acceptance
        .retain(|entry| !reused.contains(&entry.id));
    subset
        .supplementary
        .retain(|entry| !reused.contains(&entry.id));
    let subset_ids: BTreeSet<_> = subset
        .acceptance
        .iter()
        .chain(&subset.supplementary)
        .map(|entry| entry.id.clone())
        .collect();
    subset
        .gap_policy
        .permitted_acceptance_ids
        .retain(|id| subset_ids.contains(id));
    let mut judged_ids = BTreeSet::new();
    if !subset.acceptance.is_empty() || !subset.supplementary.is_empty() {
        judged_ids.extend(subset_ids.iter().cloned());
        let judged = match store {
            Some(store) => store.judge_subset(client, subset, &subset_ids).await?,
            None => {
                judge::judge_contract_subset_resumable_with_roots(client, subset, None, roots)
                    .await?
            }
        };
        merge_judgments(&mut contract, &judged);
    }
    let mut pinned_ids = BTreeSet::new();
    if let Some(base) = &base {
        pinned_ids = keep_pinned_entries(&mut contract, base, client, roots);
    }
    save_judged_entries(
        project,
        &tasks_root,
        &contract.prd.digest,
        client,
        &contract,
        &judged_ids,
        &pinned_ids,
        roots,
    )?;
    validate_acceptance_structure(&contract, expected, true).map_err(anyhow::Error::new)?;
    Ok(contract)
}

fn merge_judgments(contract: &mut AcceptanceContract, judged: &AcceptanceContract) {
    for entry in contract
        .acceptance
        .iter_mut()
        .chain(&mut contract.supplementary)
    {
        if let Some(new) = judged
            .acceptance
            .iter()
            .chain(&judged.supplementary)
            .find(|new| new.id == entry.id)
        {
            entry.judgment = new.judgment.clone();
        }
    }
}

#[cfg(test)]
#[path = "workflow_judge_incremental_tests.rs"]
mod tests;
