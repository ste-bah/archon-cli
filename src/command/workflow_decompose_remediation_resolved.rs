//! Per-report proof that a check subject was published by resume reauthoring.

use super::Ledger;
use anyhow::{Context, Result};
use std::{collections::BTreeMap, io::Write, path::Path};

fn resolved_subjects(run_dir: &Path, report_id: &str) -> Result<BTreeMap<String, String>> {
    let path = run_dir.join("decomposition/reverify/resolved.jsonl");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let mut resolved = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let row: serde_json::Value = serde_json::from_str(line)
            .with_context(|| format!("parsing line {} of {}", index + 1, path.display()))?;
        if row["report_id"].as_str() != Some(report_id) {
            continue;
        }
        if let Some(subjects) = row["subjects"].as_object() {
            for (id, digest) in subjects {
                if let Some(digest) = digest.as_str() {
                    resolved.insert(id.clone(), digest.to_string());
                }
            }
        }
    }
    Ok(resolved)
}

pub(crate) fn entry_digest(
    contract: &archon_workflow::task_set_contract::AcceptanceContract,
    id: &str,
) -> Result<Option<String>> {
    let Some(entry) = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .find(|entry| entry.id == id)
    else {
        return Ok(None);
    };
    Ok(Some(archon_workflow::task_set_contract::content_digest(
        &serde_json::to_vec(entry)?,
    )))
}

pub(crate) fn append_resolved(
    run_dir: &Path,
    report_id: &str,
    subjects: &BTreeMap<String, String>,
) -> Result<()> {
    if subjects.is_empty() {
        return Ok(());
    }
    let path = run_dir.join("decomposition/reverify/resolved.jsonl");
    std::fs::create_dir_all(path.parent().expect("ledger parent"))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    serde_json::to_writer(
        &mut file,
        &serde_json::json!({"report_id": report_id, "subjects": subjects}),
    )?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

pub(crate) fn unresolved_pending_subjects(
    run_dir: &Path,
    ledger: &Ledger,
    contract: &archon_workflow::task_set_contract::AcceptanceContract,
) -> Result<std::collections::BTreeSet<String>> {
    let resolved = resolved_subjects(run_dir, &ledger.report_id)?;
    let mut unresolved = std::collections::BTreeSet::new();
    for id in &ledger.subjects {
        let current = entry_digest(contract, id)?;
        if current.is_none() || resolved.get(id) != current.as_ref() {
            unresolved.insert(id.clone());
        }
    }
    Ok(unresolved)
}
