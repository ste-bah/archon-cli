//! Fsynced fixed-decomposition lifecycle markers outside individual call transitions.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use archon_workflow::{FixedRunIdentityV1, WorkflowError, WorkflowResult};

pub(crate) fn validated_fixed_log_path(
    persisted: &Path,
    identity: &FixedRunIdentityV1,
) -> WorkflowResult<PathBuf> {
    let task_root = PathBuf::from(&identity.task_root_identity)
        .canonicalize()
        .map_err(|source| WorkflowError::Io {
            path: PathBuf::from(&identity.task_root_identity),
            source,
        })?;
    let expected = task_root.join(".decompose.log");
    let persisted_parent = persisted.parent().ok_or_else(|| {
        WorkflowError::StateCorrupt(format!(
            "fixed decomposition log path {} has no parent",
            persisted.display()
        ))
    })?;
    let persisted_parent = persisted_parent
        .canonicalize()
        .map_err(|source| WorkflowError::Io {
            path: persisted_parent.to_path_buf(),
            source,
        })?;
    if persisted.file_name().and_then(|name| name.to_str()) != Some(".decompose.log")
        || persisted_parent != task_root
    {
        return Err(WorkflowError::StateCorrupt(format!(
            "fixed decomposition log path {} differs from canonical task-root log {}",
            persisted.display(),
            expected.display()
        )));
    }
    if let Ok(metadata) = std::fs::symlink_metadata(&expected) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(WorkflowError::StateCorrupt(format!(
                "fixed decomposition log path {} is not a regular non-symlink file",
                expected.display()
            )));
        }
    }
    Ok(expected)
}

pub(crate) fn append_fixed_log_marker(
    log_path: &Path,
    event: &str,
    run_id: &str,
    identity: &FixedRunIdentityV1,
) -> WorkflowResult<()> {
    let log_path = validated_fixed_log_path(log_path, identity)?;
    let log_path = log_path.as_path();
    let fields = [
        ("event", event),
        ("run_id", run_id),
        ("binary_revision", &identity.starting_binary_revision),
        ("script_digest", &identity.script_digest),
        ("catalog_digest", &identity.catalog_digest),
    ];
    if fields.iter().any(|(_, value)| {
        value.is_empty()
            || value
                .chars()
                .any(|ch| ch.is_whitespace() || ch.is_control() || ch == '=')
    }) {
        return Err(WorkflowError::SpecInvalid(
            "fixed decomposition log marker contains an unsafe or empty field".into(),
        ));
    }
    let line = fields
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(" ");
    append_nofollow_line(log_path, &line)
}

pub(crate) fn append_nofollow_line(path: &Path, line: &str) -> WorkflowResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| WorkflowError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path).map_err(|source| WorkflowError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let metadata = file.metadata().map_err(|source| WorkflowError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(WorkflowError::StateCorrupt(format!(
            "fixed decomposition log path {} did not open as a regular file",
            path.display()
        )));
    }
    file.write_all(format!("{line}\n").as_bytes())
        .map_err(|source| WorkflowError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    file.sync_all().map_err(|source| WorkflowError::Io {
        path: path.to_path_buf(),
        source,
    })
}
