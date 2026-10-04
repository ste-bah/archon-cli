//! Task-set authority for external host-command receipt destinations.
use super::super::workflow_task_set::{validate_destination, validate_existing_parents};
use anyhow::{Context, Result, anyhow};
use std::path::{Component, Path, PathBuf};

const RECEIPT_BINDING: &str = "publish-task-set.json";

/// Before starting its journal, the trusted host binds an external receipt
/// destination to this task root. Journal data cannot grant this authority.
/// The binding contains only a root digest; it cannot authorize arbitrary paths.
pub(crate) fn authorize_receipt(pin: &Path, tasks: &Path, target: &Path) -> Result<()> {
    if target.file_name() != Some(std::ffi::OsStr::new("gate-envelope.json")) {
        return Ok(());
    }
    let Some(project) = project_for_pin(pin) else {
        return Err(anyhow!("external receipt has no project publication scope"));
    };
    let store = archon_workflow::WorkflowStore::project(project);
    let relative = target
        .strip_prefix(store.root())
        .context("receipt is outside the project's run store")?;
    let components = relative.components().collect::<Vec<_>>();
    if components.len() != 4
        || components[1].as_os_str() != "host-command-results"
        || components
            .iter()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(anyhow!(
            "{} is not a host receipt destination",
            target.display()
        ));
    }
    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("receipt has no parent"))?;
    // Check existing parents before mkdir, then validate the complete path.
    validate_existing_parents(target, project)?;
    super::super::workflow_task_set::create_dir_all_durably(parent)?;
    validate_destination(target, &[(project.to_path_buf(), None)])?;
    let binding = parent.join(RECEIPT_BINDING);
    validate_destination(&binding, &[(project.to_path_buf(), None)])?;
    let digest = root_digest(tasks)?;
    // A call belongs to one set. Refuse to rebind a directory another set owns.
    if binding.exists() {
        let prior: String = serde_json::from_slice(&std::fs::read(&binding)?)?;
        if prior != digest {
            return Err(anyhow!(
                "receipt {} belongs to another task set",
                target.display()
            ));
        }
    } else {
        let temp = parent.join(format!(".publish-authority-{}.tmp", uuid::Uuid::new_v4()));
        validate_destination(&temp, &[(project.to_path_buf(), None)])?;
        super::super::workflow_task_set::write_durably(&temp, &serde_json::to_vec(&digest)?)?;
        // Exclusive linking publishes a complete immutable binding, including
        // when another task set races to claim the same call directory.
        let claimed = std::fs::hard_link(&temp, &binding);
        std::fs::remove_file(&temp)?;
        match claimed {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let prior: String = serde_json::from_slice(&std::fs::read(&binding)?)?;
                if prior != digest {
                    return Err(anyhow!("receipt belongs to another task set"));
                }
            }
            Err(error) => return Err(error.into()),
        }
        super::super::workflow_task_set::sync_parent(&binding)?;
    }
    Ok(())
}

/// Only call directories bound to this task set authorize their receipt file.
/// Existing decomposition state provides the binding for journals from HEAD
/// before the explicit call binding was introduced. A symlinked project, store
/// or run root is resolved; a link that leads out of the store is refused.
pub(crate) fn receipt_scopes(pin: &Path, tasks: &Path) -> Result<Vec<(PathBuf, Option<String>)>> {
    let Some(project) = project_for_pin(pin) else {
        return Ok(Vec::new());
    };
    let store = archon_workflow::WorkflowStore::project(project);
    let expected = match root_digest(tasks) {
        Ok(digest) => digest,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(Vec::new());
        }
        Err(error) => return Err(error),
    };
    let mut scopes = Vec::new();
    for run in directories(store.root())? {
        let prior_root = run.join(super::super::workflow_decompose::FIXED_DECOMPOSITION_STATE_PATH);
        validate_existing_parents(&prior_root, project)?;
        let legacy_bound = std::fs::read(&prior_root)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|state| {
                state
                    .get("identity")?
                    .get("task_root_identity")?
                    .as_str()
                    .map(PathBuf::from)
            })
            .is_some_and(|root| root_digest(&root).ok().as_ref() == Some(&expected));
        for call in directories(&run.join("host-command-results"))? {
            let binding = call.join(RECEIPT_BINDING);
            validate_existing_parents(&binding, project)?;
            let bound = match std::fs::read(&binding) {
                Ok(bytes) => {
                    serde_json::from_slice::<String>(&bytes)
                        .context("reading receipt task-set binding")?
                        == expected
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => legacy_bound,
                Err(error) => {
                    return Err(error).with_context(|| format!("reading {}", binding.display()));
                }
            };
            if bound {
                scopes.push((call, Some("gate-envelope.json".into())));
            }
        }
    }
    Ok(scopes)
}

fn project_for_pin(pin: &Path) -> Option<&Path> {
    let pins = pin.parent()?;
    (pins.file_name()? == archon_workflow::task_set_lineage::PIN_STORE_NAMESPACE)
        .then(|| pins.parent()?.parent())
        .flatten()
}

fn root_digest(tasks: &Path) -> Result<String> {
    let canonical = tasks.canonicalize().map(archon_shell::paths::plain)?;
    Ok(archon_workflow::task_set_contract::content_digest(
        canonical.to_string_lossy().as_bytes(),
    ))
}

fn directories(root: &Path) -> Result<Vec<PathBuf>> {
    // The root itself may be a link (a symlinked `.archon` or store); entries
    // below are listed without following links (`DirEntry::file_type`).
    match std::fs::metadata(root) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Ok(Vec::new()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("checking {}", root.display())),
    }
    let mut directories = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            directories.push(entry.path());
        }
    }
    Ok(directories)
}
