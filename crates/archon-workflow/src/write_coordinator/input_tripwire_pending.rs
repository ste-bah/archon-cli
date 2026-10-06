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
        crate::store::sync_dir(dir)?;
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
        let same_process = self.pending.session == session();
        let violation = self
            .tripwire
            .take()
            .expect("one owned check")
            .check_recording(&self.pending.label, same_process, |violation| {
                save(&report, &serde_json::to_vec(violation)?)
            })?;
        let mut violation = violation.or(remembered);
        if let Some(violation) = &mut violation {
            // A crash after restoring but before recording completion must
            // still report the already detected violation on the next owner.
            for change in &mut violation.changed {
                if state_of(&self.pending.policy.project.join(&change.path)).0 == change.before {
                    change.restored = true;
                    change.note.clear();
                }
            }
            if self.recovered {
                violation.attributed = false;
            }
            log(&self.tripwire_root(), violation)
                .map_err(|e| WorkflowError::io(self.tripwire_root(), e))?;
            crate::store::sync_dir(&self.tripwire_root().join("write-coordination"))?;
            if !violation.restored() {
                return Ok(Some(violation.clone()));
            }
        }
        std::fs::remove_file(&self.path).map_err(|e| WorkflowError::io(&self.path, e))?;
        crate::store::sync_dir(self.path.parent().unwrap())?;
        if report.exists() {
            std::fs::remove_file(&report).map_err(|e| WorkflowError::io(&report, e))?;
            crate::store::sync_dir(report.parent().unwrap())?;
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
        std::fs::File::open(&object)
            .and_then(|f| f.sync_all())
            .map_err(|e| WorkflowError::io(&object, e))?;
    }
    if objects_dir(root).exists() {
        crate::store::sync_dir(&objects_dir(root))?;
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
