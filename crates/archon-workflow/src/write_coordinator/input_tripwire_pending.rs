//! An owned evaluation's comparison survives pause, takeover and cancellation.
use super::*;
use crate::{WorkflowError, WorkflowResult};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Pending {
    policy: ProjectInputPolicy,
    files: BTreeMap<String, String>,
    armed_at: u64,
    session: String,
    label: String,
}

#[cfg(test)]
#[path = "input_tripwire_pending_tests.rs"]
mod tests;

static ACTIVE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
fn session() -> &'static str {
    static SESSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SESSION.get_or_init(|| uuid::Uuid::new_v4().to_string())
}
fn directory(root: &Path) -> PathBuf {
    root.join("write-coordination/input-tripwire/pending")
}
fn pause(error: impl std::fmt::Display) -> WorkflowError {
    WorkflowError::ControlPaused(format!(
        "pending input comparison must be reconciled before evaluation: {error}"
    ))
}
fn save(path: &Path, bytes: &[u8]) -> WorkflowResult<()> {
    crate::store::write_atomic(&path.with_extension("tmp"), path, bytes)?;
    // The pending directory and its parents can all be new.
    for dir in path.parent().into_iter().flat_map(Path::ancestors).take(4) {
        crate::durable_io::sync_dir(dir)?;
    }
    Ok(())
}
fn report_path(path: &Path) -> PathBuf {
    path.with_extension("detected")
}

pub(super) struct OwnedTripwire {
    tripwire: Option<InputTripwire>,
    pending: Pending,
    path: PathBuf,
    recovered: bool,
}
impl Drop for OwnedTripwire {
    fn drop(&mut self) {
        ACTIVE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|path| path != &self.path);
        // Only a completed comparison removes the durable registration.
    }
}
impl OwnedTripwire {
    pub(super) fn check(mut self) -> WorkflowResult<Option<EnvironmentViolation>> {
        let report = report_path(&self.path);
        let remembered = match std::fs::read(&report) {
            Ok(bytes) => Some(serde_json::from_slice::<EnvironmentViolation>(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(WorkflowError::io(&report, error)),
        };
        if let Some(violation) = &remembered {
            let root = self.tripwire_root();
            let origin = violation
                .backup_dir
                .ancestors()
                .nth(3)
                .ok_or_else(|| pause("a detected receipt has no backup root"))?;
            if origin
                .canonicalize()
                .map(archon_shell::paths::plain)
                .map_err(pause)?
                != root
                    .canonicalize()
                    .map(archon_shell::paths::plain)
                    .map_err(pause)?
            {
                return Err(pause("a detected receipt names another run's backup root"));
            }
            let backups = origin.join("write-coordination/environment-violations");
            for path in std::iter::once(&violation.backup_dir)
                .chain(violation.changed.iter().filter_map(|c| c.backup.as_ref()))
            {
                if !path.starts_with(&backups)
                    || path
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    return Err(pause("a detected receipt names an invalid backup path"));
                }
                refuse_links(origin, path).map_err(pause)?;
            }
        }
        let same_process = self.pending.session == session();
        let violation = self
            .tripwire
            .take()
            .expect("one owned check")
            .check_resuming(&self.pending.label, same_process, remembered, |violation| {
                save(&report, &serde_json::to_vec(violation)?)
            })?;
        let mut violation = violation;
        if let Some(violation) = &mut violation {
            // A crash after restoring but before recording completion must
            // still report the already detected violation on the next owner.
            let backup_root = violation
                .backup_dir
                .ancestors()
                .nth(3)
                .expect("backup under run root");
            for change in &mut violation.changed {
                let destination = self.pending.policy.project.join(&change.path);
                let backup = change.backup.clone().unwrap_or_else(|| {
                    violation
                        .backup_dir
                        .join(super::super::project_inputs::external::stored(&change.path))
                });
                if change.after.len() == 64 && change.after.bytes().all(|b| b.is_ascii_hexdigit()) {
                    crate::durable_io::sync_file(&backup)?;
                    for dir in backup
                        .parent()
                        .into_iter()
                        .flat_map(Path::ancestors)
                        .take_while(|dir| dir.starts_with(backup_root))
                    {
                        crate::durable_io::sync_dir(dir)?;
                    }
                }
                if state_of(&destination).0 == change.before {
                    let project = self
                        .pending
                        .policy
                        .external
                        .tree_of(&destination)
                        .unwrap_or(&self.pending.policy.project);
                    refuse_links(project, &destination)
                        .map_err(|e| WorkflowError::io(&destination, e))?;
                    if change.before.len() == 64
                        && change.before.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        crate::durable_io::sync_file(&destination)?;
                    }
                    for dir in destination
                        .parent()
                        .into_iter()
                        .flat_map(Path::ancestors)
                        .take_while(|dir| dir.starts_with(project))
                    {
                        match std::fs::metadata(dir) {
                            Ok(meta) if meta.is_dir() => crate::durable_io::sync_dir(dir)?,
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Ok(_) => {
                                return Err(pause("a repaired input parent is not a directory"));
                            }
                            Err(error) => return Err(WorkflowError::io(dir, error)),
                        }
                    }
                    change.restored = true;
                    change.note.clear();
                }
            }
            if self.recovered {
                violation.attributed = false;
            }
            log(&self.tripwire_root(), violation)
                .map_err(|e| WorkflowError::io(self.tripwire_root(), e))?;
            crate::durable_io::sync_dir(&self.tripwire_root().join("write-coordination"))?;
            if !violation.restored() {
                return Ok(Some(violation.clone()));
            }
        }
        std::fs::remove_file(&self.path).map_err(|e| WorkflowError::io(&self.path, e))?;
        crate::durable_io::sync_dir(self.path.parent().unwrap())?;
        if report.exists() {
            std::fs::remove_file(&report).map_err(|e| WorkflowError::io(&report, e))?;
            crate::durable_io::sync_dir(report.parent().unwrap())?;
        }
        Ok(violation)
    }
    fn tripwire_root(&self) -> PathBuf {
        self.path
            .ancestors()
            .nth(4)
            .expect("pending under run root")
            .to_path_buf()
    }
}

pub(super) fn reconcile(root: &Path) -> WorkflowResult<()> {
    let dir = directory(root);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(WorkflowError::io(&dir, error)),
    };
    let mut paths = entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| WorkflowError::io(&dir, e))?;
    paths.sort();
    for path in paths
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
    {
        if ACTIVE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&path)
        {
            return Err(pause(format!("{} is still in flight", path.display())));
        }
        let pending: Pending = serde_json::from_slice(
            &std::fs::read(&path).map_err(|e| WorkflowError::io(&path, e))?,
        )?;
        if ProjectInputPolicy::recorded(root).as_ref() != Some(&pending.policy) {
            return Err(pause(
                "the pending comparison's input policy changed or is unavailable",
            ));
        }
        for rel in pending.files.keys() {
            let rel_path = Path::new(rel);
            let external =
                rel_path.is_absolute() && pending.policy.external.tree_of(rel_path).is_some();
            if rel_path
                .components()
                .any(|p| matches!(p, std::path::Component::ParentDir))
                || !(external || !rel_path.is_absolute() && pending.policy.covers(rel))
            {
                return Err(pause(
                    "a pending comparison names a path outside its input policy",
                ));
            }
        }
        let tripwire = InputTripwire {
            run_root: root.to_path_buf(),
            policy: pending.policy.clone(),
            armed_at: pending.armed_at,
            files: pending.files.clone(),
            complete: true,
            exempt: vec![],
            window: records::Window::open(&pending.policy.project),
        };
        let owned = OwnedTripwire {
            tripwire: Some(tripwire),
            pending,
            path,
            recovered: true,
        };
        if let Some(violation) = owned.check()? {
            return Err(pause(violation.message()));
        }
    }
    Ok(())
}

pub(super) fn arm(root: &Path, label: &str) -> WorkflowResult<Option<OwnedTripwire>> {
    reconcile(root).map_err(pause)?;
    let Some(tripwire) = InputTripwire::arm(root) else {
        return Ok(None);
    };
    if !tripwire.complete {
        return Err(pause("the input snapshot exceeded its file limit"));
    }
    // Arming cannot continue if any pre-call bytes were not kept durably.
    for state in tripwire.files.values().filter(|s| s.len() == 64) {
        let object = objects_dir(root).join(state);
        if kept_object(root, state).is_none() {
            return Err(pause(format!("pre-call object {state} is missing")));
        }
        crate::durable_io::sync_file(&object)?;
    }
    if objects_dir(root).exists() {
        crate::durable_io::sync_dir(&objects_dir(root))?;
    }
    let pending = Pending {
        policy: tripwire.policy.clone(),
        files: tripwire.files.clone(),
        armed_at: tripwire.armed_at,
        session: session().into(),
        label: label.into(),
    };
    let path = directory(root).join(format!("{}.json", uuid::Uuid::new_v4()));
    save(&path, &serde_json::to_vec(&pending)?).map_err(pause)?;
    ACTIVE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(path.clone());
    Ok(Some(OwnedTripwire {
        tripwire: Some(tripwire),
        pending,
        path,
        recovered: false,
    }))
}
