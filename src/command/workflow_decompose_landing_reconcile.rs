//! Putting each live task body back to its current body before a resume.
//!
//! An older binary re-ran superseded landing rounds after a logic bump and
//! published their old candidates over the bodies later rounds had accepted
//! (`archon_workflow::v2::script::landing_lineage`). A resume must never
//! build on such a file silently. Before the frozen-chain decision reads the
//! task root, every frozen task whose live file is not the current body of
//! its landing lineage is rebuilt from that landing's own recorded
//! candidate, stamped and packaged exactly as the landing was, and written
//! back only when the rebuilt bytes hash to the landing's receipt. When they
//! do not, the resume is refused with a message naming the receipt. Only a
//! body some landing of this run published is put back: a live file no
//! receipt names (an edit made outside the run) refuses the resume instead
//! of being overwritten. A task with no accepted landing on record is left
//! as it is.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_workflow::WorkflowV2CallRecord;
use archon_workflow::task_set_contract::content_digest;
use archon_workflow::task_skeleton::{FrozenTask, TaskSkeleton};
use archon_workflow::v2::script::landing_lineage::LandingLineage;

/// One task body the resume writes back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BodyRestore {
    pub(crate) task_id: String,
    pub(crate) path: PathBuf,
    /// The landing whose receipt the body is restored to.
    pub(crate) call_id: String,
    pub(crate) committed_at: String,
    pub(crate) live_blake3: Option<String>,
    pub(crate) receipt_blake3: String,
    pub(crate) bytes: Vec<u8>,
}

/// What the reconciliation found: the bodies to write back, and one line
/// per frozen task saying what it decided.
#[derive(Debug, Default)]
pub(crate) struct ReconcilePlan {
    pub(crate) restores: Vec<BodyRestore>,
    pub(crate) lines: Vec<String>,
}

/// Compare every frozen task's live body with its current landing.
pub(crate) fn plan(
    records: &[WorkflowV2CallRecord],
    task_root: &Path,
    skeleton: &TaskSkeleton,
) -> Result<ReconcilePlan> {
    let mut plan = ReconcilePlan::default();
    for frozen in &skeleton.tasks {
        let path = task_root.join(&frozen.file_name);
        let Some(current) = current_landing(records, &frozen.task_id) else {
            plan.lines.push(format!(
                "{}: no accepted landing on record; left as it is",
                frozen.task_id
            ));
            continue;
        };
        let receipt = &current.result.data["publicationReceipt"];
        let committed_at = receipt["committed_at"].as_str().unwrap_or_default();
        let entry = receipt["entries"]
            .as_array()
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|entry| entry["relative_path"] == frozen.file_name.as_str())
            })
            .ok_or_else(|| {
                anyhow!(
                    "resume refused: the current landing of {} (call {}, committed {committed_at}) has a receipt with no entry for {}",
                    frozen.task_id,
                    current.call.id,
                    frozen.file_name
                )
            })?;
        let blake3 = entry["blake3"].as_str().unwrap_or_default().to_string();
        let byte_len = entry["byte_len"].as_u64();
        let live = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error).with_context(|| format!("reading task body {}", path.display()));
            }
        };
        let live_blake3 = live.as_deref().map(content_digest);
        if live_blake3.as_deref() == Some(blake3.as_str())
            && live.as_ref().map(|bytes| bytes.len() as u64) == byte_len
        {
            plan.lines.push(format!(
                "{}: live body matches its current landing (call {}, committed {committed_at}, blake3 {blake3})",
                frozen.task_id, current.call.id
            ));
            continue;
        }
        let refused = |why: String| {
            anyhow!(
                "resume refused: the live task file {} (blake3 {}) is not the current body of {}: the landing of call {} committed {committed_at} with blake3 {blake3}, and {why}; put {} back to blake3 {blake3} before resuming",
                path.display(),
                live_blake3.as_deref().unwrap_or("absent"),
                frozen.task_id,
                current.call.id,
                path.display()
            )
        };
        // Only a body this run itself landed is put back: content no landing
        // of the task ever published (an edit made outside the run) is the
        // operator's to resolve, never overwritten. A body a landing newer
        // than the current one published before its record was saved (a
        // stop between publication and record) is that landing's own: the
        // resume asks it again, so it is left in place.
        if let Some(live) = live_blake3.as_deref()
            && !landed_digests(records, frozen).contains(live)
        {
            match unrecorded_landing(records, frozen, &path, live) {
                Some(newer) if newer.started_at > current.started_at => {
                    plan.lines.push(format!(
                        "{}: live body (blake3 {live}) is the candidate of call {}, started after the current landing and published without a receipt; left for the resume to judge again",
                        frozen.task_id, newer.call.id
                    ));
                    continue;
                }
                Some(_) => {}
                None => {
                    return Err(refused(
                        "the live file is no body any landing of this run published".into(),
                    ));
                }
            }
        }
        let rebuilt = rebuild(current, frozen, &path).map_err(&refused)?;
        let rebuilt_blake3 = content_digest(&rebuilt);
        if rebuilt_blake3 != blake3 || Some(rebuilt.len() as u64) != byte_len {
            return Err(refused(format!(
                "its recorded candidate rebuilds to blake3 {rebuilt_blake3}, not to its receipt"
            )));
        }
        plan.lines.push(format!(
            "{}: RESTORE {} from blake3 {} to blake3 {blake3}, the current landing (call {}, committed {committed_at})",
            frozen.task_id,
            path.display(),
            live_blake3.as_deref().unwrap_or("absent"),
            current.call.id
        ));
        plan.restores.push(BodyRestore {
            task_id: frozen.task_id.clone(),
            path,
            call_id: current.call.id.clone(),
            committed_at: committed_at.to_string(),
            live_blake3,
            receipt_blake3: blake3,
            bytes: rebuilt,
        });
    }
    Ok(plan)
}

/// Every task-file digest a receipt of a landing of `frozen` names.
fn landed_digests(records: &[WorkflowV2CallRecord], frozen: &FrozenTask) -> BTreeSet<String> {
    records
        .iter()
        .filter(|record| {
            record
                .call
                .options
                .host_command
                .as_ref()
                .and_then(|request| request.target.as_ref())
                .is_some_and(|target| target.task_id == frozen.task_id)
        })
        .filter_map(|record| record.result.data["publicationReceipt"]["entries"].as_array())
        .flatten()
        .filter(|entry| entry["relative_path"] == frozen.file_name.as_str())
        .filter_map(|entry| entry["blake3"].as_str().map(str::to_string))
        .collect()
}

/// The recorded landing of `frozen` whose candidate rebuilds to `live`, for
/// a body published without a receipt.
fn unrecorded_landing<'a>(
    records: &'a [WorkflowV2CallRecord],
    frozen: &FrozenTask,
    path: &Path,
    live: &str,
) -> Option<&'a WorkflowV2CallRecord> {
    records
        .iter()
        .filter(|record| {
            record
                .call
                .options
                .host_command
                .as_ref()
                .and_then(|request| request.target.as_ref())
                .is_some_and(|target| target.task_id == frozen.task_id)
        })
        .filter(|record| {
            rebuild(record, frozen, path).is_ok_and(|bytes| content_digest(&bytes) == live)
        })
        .max_by(|a, b| a.started_at.cmp(&b.started_at))
}

/// The latest current landing of `task_id` over every targeted command
/// that landed it.
fn current_landing<'a>(
    records: &'a [WorkflowV2CallRecord],
    task_id: &str,
) -> Option<&'a WorkflowV2CallRecord> {
    let commands: BTreeSet<&str> = records
        .iter()
        .filter_map(|record| record.call.options.host_command.as_ref())
        .filter(|request| {
            request
                .target
                .as_ref()
                .is_some_and(|target| target.task_id == task_id)
        })
        .map(|request| request.command_id.as_str())
        .collect();
    commands
        .into_iter()
        .filter_map(|command| LandingLineage::of(records, command, task_id).current())
        .max_by(|a, b| a.started_at.cmp(&b.started_at))
}

/// The bytes `record`'s landing published: its recorded candidate with the
/// frozen frontmatter stamped in at its task file anchor, as the executor
/// stamps it, then packaged as the body gate lands it.
fn rebuild(
    record: &WorkflowV2CallRecord,
    frozen: &FrozenTask,
    path: &Path,
) -> Result<Vec<u8>, String> {
    let candidate = record
        .call
        .options
        .host_command
        .as_ref()
        .and_then(|request| request.stdin.as_deref())
        .ok_or("its record holds no candidate")?;
    let subjects = [(frozen.task_id.as_str(), path)];
    let anchor = match crate::command::topology_lint::task_file_anchor(candidate, &subjects) {
        Ok(Some(anchor)) => anchor,
        Ok(None) => return Err("its recorded candidate has no task file anchor".into()),
        Err(_) => return Err("its recorded candidate holds several task files".into()),
    };
    let (stamped, _) = crate::command::workflow_host_command_binding::stamp_frozen_frontmatter(
        &candidate[anchor.opener..],
        frozen,
    )
    .map_err(|error| format!("its recorded candidate cannot be stamped: {error}"))?;
    let stamped = format!("{}{}", &candidate[..anchor.opener], stamped);
    crate::command::workflow::landed_task_file(stamped.into_bytes(), path)
        .map_err(|why| format!("its recorded candidate cannot be packaged: {why}"))
}

/// Reconcile the set under `task_root` with the landings recorded in
/// `run_dir`: the plan, written back when `write` (a resume), only reported
/// when not (a dry run). A set with no frozen skeleton has no bodies yet.
pub(crate) fn reconcile_task_root(
    project_root: &Path,
    run_dir: &Path,
    task_root: &Path,
    write: bool,
) -> Result<Vec<String>> {
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project_root, task_root);
    let pin: archon_workflow::task_set_contract::AcceptancePin = match std::fs::read(&pin_path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing acceptance pin {}", pin_path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(vec![
                "landing reconciliation: no acceptance pin yet; nothing landed".into(),
            ]);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading acceptance pin {}", pin_path.display()));
        }
    };
    if !task_root
        .join(archon_workflow::task_set_contract::TASK_SKELETON_FILE)
        .exists()
    {
        return Ok(vec![
            "landing reconciliation: no frozen skeleton yet; nothing landed".into(),
        ]);
    }
    let skeleton = match archon_workflow::task_skeleton::validate_full_chain(task_root, &pin) {
        Ok(skeleton) => skeleton,
        // The frozen-chain decision after this reports the chain itself; no
        // body can be rebuilt against a skeleton that does not validate.
        Err(error) => {
            return Ok(vec![format!(
                "landing reconciliation: the frozen chain does not validate ({error}); no body was compared"
            )]);
        }
    };
    let records =
        archon_workflow::WorkflowV2ResultStore::new(run_dir.join("v2")).load_call_records()?;
    let plan = plan(&records, task_root, &skeleton)?;
    if write {
        apply(&pin_path, task_root, &plan)?;
    }
    let verb = if write { "restored" } else { "would restore" };
    let mut lines = vec![format!(
        "landing reconciliation: {verb} {} of {} task bodies",
        plan.restores.len(),
        skeleton.tasks.len()
    )];
    lines.extend(plan.lines.into_iter().map(|line| format!("  {line}")));
    Ok(lines)
}

/// The resume's reconciliation: written back, each decision reported on
/// `ui_sink`, or the resume refused.
pub(crate) async fn resume(
    project_root: &Path,
    run_dir: &Path,
    task_root: &Path,
    ui_sink: &archon_workflow::SharedWorkflowUiSink,
) -> Result<()> {
    for line in reconcile_task_root(project_root, run_dir, task_root, true)? {
        ui_sink
            .emit(archon_workflow::WorkflowUiEvent::Text(format!("{line}\n")))
            .await
            .map_err(|error| anyhow!("reporting the landing reconciliation: {error}"))?;
    }
    Ok(())
}

/// What a resume does with every recorded landing request: per targeted
/// subject, each answer in start order, whether it is a rerun (neither
/// history nor authority), the current body (the one request a resume may
/// judge again) or a superseded round (replayed from its record, nothing
/// published).
pub(crate) fn landing_report(run_dir: &Path) -> Result<Vec<String>> {
    let records =
        archon_workflow::WorkflowV2ResultStore::new(run_dir.join("v2")).load_call_records()?;
    let subjects: BTreeSet<(String, String)> = records
        .iter()
        .filter_map(|record| record.call.options.host_command.as_ref())
        .filter_map(|request| {
            request
                .target
                .as_ref()
                .map(|target| (request.command_id.clone(), target.task_id.clone()))
        })
        .collect();
    let mut lines = Vec::new();
    for (command, subject) in &subjects {
        let lineage = LandingLineage::of(&records, command, subject);
        let current = lineage.current();
        lines.push(format!(
            "landing lineage {command} {subject}: current body {}",
            current.map_or("none", |record| record.call.id.as_str())
        ));
        for record in lineage.answers() {
            let Some(request) = record.call.options.host_command.as_ref() else {
                continue;
            };
            let occurrence = archon_workflow::v2::script::landing_lineage::occurrence_of(record);
            let replayed = lineage.superseded_answer(request, occurrence, |_| false);
            let role = if lineage.is_rerun(record) {
                "rerun: neither history nor authority".to_string()
            } else if current
                .is_some_and(|current| current.call.options.host_command.as_ref() == Some(request))
            {
                "current body's request: the one request a resume may judge again".to_string()
            } else {
                format!(
                    "superseded round: asked again it replays {} and publishes nothing",
                    replayed.map_or("none", |answer| answer.call.id.as_str())
                )
            };
            lines.push(format!(
                "  {} {} status={} published={} stdin_blake3={}: {role}",
                record.started_at,
                record.call.id,
                serde_json::to_value(record.status)?
                    .as_str()
                    .unwrap_or_default(),
                !record.result.data["publicationReceipt"].is_null(),
                content_digest(request.stdin.as_deref().unwrap_or_default().as_bytes()),
            ));
        }
    }
    Ok(lines)
}

/// Write every restore back in one publication under the set's chain lock.
pub(crate) fn apply(pin_path: &Path, task_root: &Path, plan: &ReconcilePlan) -> Result<()> {
    if plan.restores.is_empty() {
        return Ok(());
    }
    let _chain_lock =
        crate::command::workflow_task_set::ChainLock::acquire_waiting(pin_path, task_root)?;
    let files: Vec<(PathBuf, Vec<u8>)> = plan
        .restores
        .iter()
        .map(|restore| (restore.path.clone(), restore.bytes.clone()))
        .collect();
    crate::command::workflow_task_set::publish_files_atomically(
        pin_path,
        task_root,
        &files,
        "restore the listed task bodies to their current landing, then resume",
    )?;
    for restore in &plan.restores {
        let live = content_digest(&std::fs::read(&restore.path)?);
        if live != restore.receipt_blake3 {
            return Err(anyhow!(
                "restored task body {} hashes to {live}, not to the receipt of call {} ({})",
                restore.path.display(),
                restore.call_id,
                restore.receipt_blake3
            ));
        }
        tracing::warn!(
            task_id = %restore.task_id,
            path = %restore.path.display(),
            call_id = %restore.call_id,
            from = restore.live_blake3.as_deref().unwrap_or("absent"),
            to = %restore.receipt_blake3,
            "task body restored to its current landing before resume"
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "workflow_decompose_landing_reconcile_tests.rs"]
mod tests;
