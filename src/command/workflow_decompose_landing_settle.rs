use std::path::Path;

use anyhow::{Context, Result, anyhow};
use archon_workflow::WorkflowV2CallRecord;
use archon_workflow::task_set_contract::content_digest;

use self::restore_temp::{next_sequence, prepare_restore_temp};
use super::{current_landing, rebuild};

#[path = "workflow_decompose_landing_settle/restore_temp.rs"]
pub(crate) mod restore_temp;

fn restore_with_captured_evidence(
    project_root: &Path,
    run_dir: &Path,
    landing: &WorkflowV2CallRecord,
    subject: &str,
    receipt_digest: &str,
    path: &Path,
    restored: &[u8],
) -> Result<()> {
    let task_dir = path
        .parent()
        .ok_or_else(|| anyhow!("task body path has no parent: {}", path.display()))?;
    let (temp_path, mut temp_cleanup) = prepare_restore_temp(path, restored)?;
    let result = (|| -> Result<()> {
        let evidence_dir = run_dir.join("landing-evidence");
        std::fs::create_dir_all(&evidence_dir)?;
        archon_workflow::store::sync_directory(run_dir)
            .with_context(|| format!("syncing run directory {}", run_dir.display()))?;

        #[cfg(test)]
        crate::command::workflow_test_seams::apply_before_landing_restore(&landing.call.id)?;

        let live = match std::fs::metadata(path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(error).with_context(|| format!("reading task body {}", path.display()));
            }
        };
        if live {
            let evidence_path = evidence_dir.join(format!(
                "{subject}-capture-{}-{}.bin",
                std::process::id(),
                next_sequence()
            ));
            // This refuses a name that already exists. A concurrent creator after
            // this check can still race the rename on filesystems where rename replaces
            // its destination; that check-to-rename window is accepted.
            if evidence_path.exists() {
                return Err(anyhow!(
                    "landing evidence path already exists: {}",
                    evidence_path.display()
                ));
            }
            match std::fs::rename(path, &evidence_path) {
                Ok(()) => {
                    archon_workflow::store::sync_directory(task_dir).with_context(|| {
                        format!("syncing task directory {}", task_dir.display())
                    })?;
                    let captured = std::fs::read(&evidence_path)?;
                    std::fs::File::open(&evidence_path)
                        .and_then(|file| file.sync_all())
                        .with_context(|| {
                            format!("syncing evidence file {}", evidence_path.display())
                        })?;
                    archon_workflow::store::sync_directory(&evidence_dir).with_context(|| {
                        format!("syncing evidence directory {}", evidence_dir.display())
                    })?;
                    emit_displaced_event(
                        project_root,
                        run_dir,
                        landing,
                        subject,
                        receipt_digest,
                        &content_digest(&captured),
                        &evidence_path,
                    )?;
                }
                Err(error) if error.raw_os_error() == Some(libc::EXDEV) => {
                    // Across filesystems rename cannot capture atomically. Copy observed bytes
                    // to evidence and compare again before replacing the live file. A write
                    // after this comparison but before the restore rename can be overwritten,
                    // just as in the direct-rename path. The post-restore reread captures only
                    // writes that land after the restore rename.
                    let observed = std::fs::read(path)
                        .with_context(|| format!("reading task body {}", path.display()))?;
                    keep_displaced_bytes(
                        project_root,
                        run_dir,
                        landing,
                        subject,
                        receipt_digest,
                        &observed,
                    )?;
                    let compared = std::fs::read(path)
                        .with_context(|| format!("comparing task body {}", path.display()))?;
                    if compared != observed {
                        keep_displaced_bytes(
                            project_root,
                            run_dir,
                            landing,
                            subject,
                            receipt_digest,
                            &compared,
                        )?;
                    }
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("capturing task body {} as evidence", path.display())
                    });
                }
            };
        }

        // A writer that does not take the chain lock can write before this restore rename.
        // In the direct path it can write after the live file is moved to evidence; in the
        // cross-filesystem fallback it can write after the final comparison. Either write
        // can be overwritten here. Only writes after this restore rename remain for the
        // post-restore reread to capture.
        std::fs::rename(&temp_path, path)
            .with_context(|| format!("restoring task body {}", path.display()))?;
        temp_cleanup.disarm();
        archon_workflow::store::sync_directory(task_dir)
            .with_context(|| format!("syncing task directory {}", task_dir.display()))?;
        Ok(())
    })();
    result
}

pub(crate) fn keep_displaced_bytes(
    project_root: &Path,
    run_dir: &Path,
    landing: &WorkflowV2CallRecord,
    subject: &str,
    receipt_digest: &str,
    bytes: &[u8],
) -> Result<String> {
    let disk_digest = content_digest(bytes);
    let evidence_dir = run_dir.join("landing-evidence");
    std::fs::create_dir_all(&evidence_dir)?;
    archon_workflow::store::sync_directory(run_dir)
        .with_context(|| format!("syncing run directory {}", run_dir.display()))?;
    let evidence_path = evidence_dir.join(format!("{subject}-{disk_digest}.bin"));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&evidence_path)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(bytes).with_context(|| {
                format!("writing landing evidence file {}", evidence_path.display())
            })?;
            file.sync_all().with_context(|| {
                format!("syncing landing evidence file {}", evidence_path.display())
            })?;
            // The evidence file's contents are durable only once its new
            // directory entry is durable as well.
            archon_workflow::store::sync_directory(&evidence_dir).with_context(|| {
                format!(
                    "syncing landing evidence directory {}",
                    evidence_dir.display()
                )
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::read(&evidence_path)? != bytes {
                return Err(anyhow!(
                    "landing evidence digest collision at {}",
                    evidence_path.display()
                ));
            }
            std::fs::File::open(&evidence_path)
                .and_then(|file| file.sync_all())
                .with_context(|| {
                    format!("syncing landing evidence file {}", evidence_path.display())
                })?;
            archon_workflow::store::sync_directory(&evidence_dir).with_context(|| {
                format!(
                    "syncing landing evidence directory {}",
                    evidence_dir.display()
                )
            })?;
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("writing landing evidence {}", evidence_path.display()));
        }
    }
    emit_displaced_event(
        project_root,
        run_dir,
        landing,
        subject,
        receipt_digest,
        &disk_digest,
        &evidence_path,
    )?;
    Ok(disk_digest)
}

fn emit_displaced_event(
    project_root: &Path,
    run_dir: &Path,
    landing: &WorkflowV2CallRecord,
    subject: &str,
    receipt_digest: &str,
    disk_digest: &str,
    evidence_path: &Path,
) -> Result<()> {
    let run_id = run_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("run directory has no id"))?;
    let store = archon_workflow::WorkflowStore::project(project_root);
    let seq = store.next_event_seq(run_id)?;
    archon_workflow::WorkflowEventLog::new(store).emit(run_id, seq, archon_workflow::WorkflowEventKind::LandingReceiptDisplaced, serde_json::json!({
        "subject": subject, "landing_call_id": landing.call.id,
        "receipt_blake3": receipt_digest, "disk_blake3": disk_digest,
        "evidence_path": evidence_path.strip_prefix(run_dir).unwrap_or(evidence_path).to_string_lossy(),
    }))?;
    Ok(())
}

/// Settle one current landing while holding the same chain lock used by
/// resume-start reconciliation. Every consumer uses this read/evidence/
/// restore/read sequence so a racing outside write is judged as the newest
/// candidate rather than becoming an operational pause.
pub(crate) fn repair_landing_mismatch(
    project_root: &Path,
    run_dir: &Path,
    task_root: &Path,
    landing: &WorkflowV2CallRecord,
    allow_restore: bool,
) -> Result<(Vec<u8>, Option<Vec<u8>>, bool)> {
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project_root, task_root);
    let _chain_lock =
        crate::command::workflow_task_set::ChainLock::acquire_waiting(&pin_path, task_root)?;
    let request = landing
        .call
        .options
        .host_command
        .as_ref()
        .ok_or_else(|| anyhow!("landing record has no host command request"))?;
    let target = request
        .target
        .as_ref()
        .ok_or_else(|| anyhow!("landing record has no subject target"))?;
    let path = task_root.join(&target.file_name);
    let records =
        archon_workflow::WorkflowV2ResultStore::new(run_dir.join("v2")).load_call_records()?;
    let Some(current) = current_landing(&records, &target.task_id) else {
        return Err(anyhow!("subject {} has no current landing", target.task_id));
    };
    if current.call.id != landing.call.id {
        // A newer accepted landing is authoritative. Do not overwrite it.
        let live = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(error).with_context(|| format!("reading task body {}", path.display()));
            }
        };
        return Ok((live, None, true));
    }
    let pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(&pin_path)?)?;
    let skeleton = archon_workflow::task_skeleton::validate_full_chain(task_root, &pin)?;
    let frozen = skeleton
        .tasks
        .iter()
        .find(|task| task.task_id == target.task_id)
        .ok_or_else(|| {
            anyhow!(
                "current landing subject {} is absent from frozen skeleton",
                target.task_id
            )
        })?;
    let receipt = &landing.result.data["publicationReceipt"];
    let entry = receipt["entries"]
        .as_array()
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry["relative_path"] == frozen.file_name)
        })
        .ok_or_else(|| {
            anyhow!(
                "current landing receipt has no entry for {}",
                frozen.file_name
            )
        })?;
    let receipt_digest = entry["blake3"].as_str().unwrap_or_default();
    let receipt_len = entry["byte_len"].as_u64().unwrap_or_default();
    let restored = rebuild(landing, frozen, &path).ok().filter(|bytes| {
        content_digest(bytes) == receipt_digest
            && Some(bytes.len() as u64) == entry["byte_len"].as_u64()
    });
    let live = match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| format!("reading task body {}", path.display()));
        }
    };
    if live.as_deref().is_some_and(|bytes| {
        content_digest(bytes) != receipt_digest || bytes.len() as u64 != receipt_len
    }) {
        keep_displaced_bytes(
            project_root,
            run_dir,
            landing,
            &target.task_id,
            receipt_digest,
            live.as_deref().unwrap_or_default(),
        )?;
    }
    if live.as_deref().is_none_or(|bytes| {
        content_digest(bytes) != receipt_digest || bytes.len() as u64 != receipt_len
    }) {
        if !allow_restore {
            return Ok((live.unwrap_or_default(), None, false));
        }
        if let Some(bytes) = restored {
            restore_with_captured_evidence(
                project_root,
                run_dir,
                landing,
                &target.task_id,
                receipt_digest,
                &path,
                &bytes,
            )?;
            #[cfg(test)]
            crate::command::workflow_test_seams::apply_after_landing_restore(&landing.call.id)?;
        }
    }
    let after = match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| format!("reading task body {}", path.display()));
        }
    };
    if let Some(after) = after {
        if content_digest(&after) != receipt_digest || after.len() as u64 != receipt_len {
            keep_displaced_bytes(
                project_root,
                run_dir,
                landing,
                &target.task_id,
                receipt_digest,
                &after,
            )?;
            return Ok((after, None, false));
        }
        return Ok((after.clone(), Some(after), false));
    }
    // No receipt body could be rebuilt. The caller falls through to the
    // ordinary author path, which asks for a fresh task body.
    Ok((Vec::new(), None, false))
}
