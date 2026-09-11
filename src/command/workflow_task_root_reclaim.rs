//! Explicit, evidence-preserving revocation of a fixed run's task-root claim.
use std::{fs::{File, OpenOptions}, path::Path};
use anyhow::{Context, Result, anyhow};
use archon_workflow::{FixedDecompositionStateV1, RunStatus, WorkflowError, WorkflowStore};
use super::workflow_decompose::FIXED_DECOMPOSITION_STATE_PATH;
const RECLAIMED: &str = "decomposition/task-root-reclaimed.json";
const LEASE: &str = "decomposition/executor.lock";

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(anyhow!("invalid workflow run id"));
    }
    Ok(())
}
fn error(e: anyhow::Error) -> WorkflowError { WorkflowError::PolicyDenied(format!("{e:#}")) }
fn lease(store: &WorkflowStore, id: &str) -> Result<File> {
    let path = store.run_dir(id).join(LEASE);
    std::fs::create_dir_all(path.parent().unwrap())?;
    let file = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&path)?;
    file.try_lock().map_err(|e| anyhow!("executor for {id} is live or its lock cannot be acquired: {e}"))?;
    Ok(file)
}

/// Held for the complete launch/resume future. Kernel releases it on SIGKILL.
pub(crate) fn begin_execution(store: &WorkflowStore, id: &str) -> Result<File> {
    validate_id(id)?;
    store.with_store_lock(|locked| {
        locked.load_state(id)?;
        require_not_reclaimed(locked, id).map_err(error)?;
        lease(locked, id).map_err(error)
    }).map_err(Into::into)
}
pub(crate) fn is_reclaimed(store: &WorkflowStore, id: &str) -> Result<bool> {
    validate_id(id)?;
    let path = store.run_dir(id).join(RECLAIMED);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let record: serde_json::Value = serde_json::from_slice(&bytes).context("invalid reclaim record")?;
    let fixed: FixedDecompositionStateV1 = serde_json::from_slice(&std::fs::read(store.run_dir(id).join(FIXED_DECOMPOSITION_STATE_PATH))?)?;
    if record["schema_version"] != 1 || record["run_id"] != id
        || record["task_root"] != fixed.identity.task_root_identity {
        return Err(anyhow!("task-root reclaim record does not bind this run"));
    }
    Ok(true)
}
pub(crate) fn require_not_reclaimed(store: &WorkflowStore, id: &str) -> Result<()> {
    if is_reclaimed(store, id)? { return Err(anyhow!("run {id} task root was reclaimed; this run cannot resume")); }
    Ok(())
}

pub(crate) fn reclaim(cwd: &Path, id: &str, yes: bool) -> Result<String> {
    let root = cwd.canonicalize()?;
    let store = WorkflowStore::project(&root);
    // Legacy executors have no lease. Also catches lingering host subprocesses
    // from killed parents. Refuse conservatively, rather than infer ownership
    // from a substring in their arguments.
    reclaim_with_liveness(&store, id, yes, no_other_archon_process)?;
    Ok(format!("Released task-root ownership for {id}. Resume permanently disabled; all run and task files preserved."))
}

pub(crate) fn reclaim_with_liveness(
    store: &WorkflowStore, id: &str, yes: bool, check_dead: impl FnOnce() -> Result<()>,
) -> Result<()> {
    validate_id(id)?;
    if !yes { return Err(anyhow!("reclaim-task-root requires --yes; it permanently disables this run's resume but preserves evidence")); }
    store.with_store_lock(|locked| {
        let mut run = locked.load_state(id)?;
        let fixed: FixedDecompositionStateV1 = serde_json::from_slice(
            &std::fs::read(locked.run_dir(id).join(FIXED_DECOMPOSITION_STATE_PATH)).map_err(|e| error(e.into()))?
        )?;
        if fixed.run_kind != archon_workflow::WorkflowRunKind::FixedDecompositionV1 {
            return Err(error(anyhow!("only fixed decomposition owns reclaimable task roots")));
        }
        let project = Path::new(&fixed.identity.project_root_identity).canonicalize().map_err(|e| error(e.into()))?;
        if WorkflowStore::project(project).root().canonicalize().map_err(|e| error(e.into()))?
            != locked.root().canonicalize().map_err(|e| error(e.into()))? {
            return Err(error(anyhow!("fixed run belongs to a different project")));
        }
        let _lease = lease(locked, id).map_err(error)?;
        check_dead().map_err(error)?;
        locked.with_run_lock(id, |locked| {
            run = locked.load_state(id)?;
            if !is_reclaimed(locked, id).map_err(error)? {
                let next = run.generation.checked_add(1).ok_or_else(|| error(anyhow!("generation exhausted")))?;
                locked.write_run_json(id, RECLAIMED, &serde_json::json!({
                    "schema_version":1,"run_id":id,"task_root":fixed.identity.task_root_identity,
                    "previous_status":run.status,"previous_generation":run.generation,"revoked_generation":next,
                    "reclaimed_at":chrono::Utc::now().to_rfc3339(),"operator_confirmed":true
                }))?;
                run.generation = next;
                run.status = RunStatus::Failed;
                run.mark_updated();
                locked.save_state(&run)?;
            }
            Ok(())
        })
    }).map_err(Into::into)
}

fn no_other_archon_process() -> Result<()> {
    let output = std::process::Command::new("ps").args(["-Ao", "pid=,comm="]).output()
        .context("cannot verify executor liveness; reclaim refused")?;
    if !output.status.success() { return Err(anyhow!("process liveness check failed; reclaim refused")); }
    for line in std::str::from_utf8(&output.stdout)?.lines() {
        let line = line.trim();
        let Some((pid, command)) = line.split_once(char::is_whitespace) else { continue; };
        let pid: u32 = pid.parse().context("invalid process table; reclaim refused")?;
        if pid == std::process::id() { continue; }
        let name = Path::new(command.trim()).file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "archon" || name.starts_with("archon-") {
            return Err(anyhow!("Archon process {pid} is still running; stop its executor before reclaiming (no process was killed)"));
        }
    }
    Ok(())
}
