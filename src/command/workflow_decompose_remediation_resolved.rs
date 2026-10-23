//! Per-report proof that a check subject was published by resume reauthoring.

use super::Ledger;
use anyhow::{Context, Result};
use std::{collections::BTreeMap, path::Path};

fn resolved_text(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

fn resolved_rows(text: &str, path: &Path) -> Result<Vec<serde_json::Value>> {
    let mut rows = Vec::new();
    let lines = text.split_terminator('\n').collect::<Vec<_>>();
    let unterminated = !text.is_empty() && !text.ends_with('\n');
    for (index, raw_line) in lines.iter().enumerate() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        match serde_json::from_str(line) {
            Ok(row) => rows.push(row),
            Err(error) if unterminated && index + 1 == lines.len() => {
                tracing::info!(path = %path.display(), line = index + 1, %error, "ignoring interrupted unterminated final resolved journal line");
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("parsing line {} of {}", index + 1, path.display()));
            }
        }
    }
    Ok(rows)
}

fn resolved_subjects(run_dir: &Path, report_id: &str) -> Result<BTreeMap<String, String>> {
    let path = run_dir.join("decomposition/reverify/resolved.jsonl");
    let mut resolved = BTreeMap::new();
    for row in resolved_rows(&resolved_text(&path)?, &path)? {
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
    crate::command::workflow_task_set::create_dir_all_durably(
        path.parent().expect("ledger parent"),
    )?;
    let existing = resolved_text(&path)?;
    let rows = resolved_rows(&existing, &path)?;
    let mut bytes = Vec::new();
    for row in rows {
        serde_json::to_writer(&mut bytes, &row)?;
        bytes.push(b'\n');
    }
    serde_json::to_writer(
        &mut bytes,
        &serde_json::json!({"report_id": report_id, "subjects": subjects}),
    )?;
    bytes.push(b'\n');
    let temp = path.with_extension("jsonl.tmp");
    crate::command::workflow_task_set::write_durably(&temp, &bytes)
        .with_context(|| format!("writing {}", temp.display()))?;
    std::fs::rename(&temp, &path).with_context(|| format!("publishing {}", path.display()))?;
    crate::command::workflow_task_set::sync_parent(&path)?;
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

/// Recover resolution records when publication completed but the process
/// stopped before appending the run journal row. Only pin transitions after
/// the contract digest judged by this report count as publications here.
pub(crate) fn reconcile_pending_from_lineage(
    project_root: &Path,
    task_root: &Path,
    run_dir: &Path,
    ledger: &Ledger,
    contract: &archon_workflow::task_set_contract::AcceptanceContract,
) -> Result<()> {
    let report_path = run_dir.join(format!(
        "decomposition/reverify/reports/{}.json",
        ledger.report_id
    ));
    let report_bytes = match std::fs::read(&report_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("reading {}", report_path.display()));
        }
    };
    let report: serde_json::Value = serde_json::from_slice(&report_bytes)
        .with_context(|| format!("parsing {}", report_path.display()))?;
    let Some(report_digest) = report["contract_digest"].as_str() else {
        return Ok(());
    };
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project_root, task_root);
    let pin_bytes = match std::fs::read(&pin_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", pin_path.display())),
    };
    let pin: archon_workflow::task_set_lineage::AcceptancePin = serde_json::from_slice(&pin_bytes)
        .with_context(|| format!("parsing {}", pin_path.display()))?;

    let mut after_report = false;
    let mut published = std::collections::BTreeSet::new();
    for transition in &pin.lineage {
        if transition.from.acceptance_digest == report_digest {
            after_report = true;
        }
        if after_report {
            published.extend(
                transition
                    .reauthored_ids
                    .iter()
                    .filter(|id| ledger.subjects.contains(id))
                    .cloned(),
            );
        }
        if transition.to.acceptance_digest == report_digest {
            after_report = true;
        }
    }
    let resolved = published
        .into_iter()
        .filter_map(|id| {
            entry_digest(contract, &id)
                .transpose()
                .map(|digest| digest.map(|digest| (id, digest)))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    append_resolved(run_dir, &ledger.report_id, &resolved)
}
