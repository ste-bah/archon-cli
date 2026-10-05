//! Atomic storage for secret-bearing run records. Unix directories are 0700
//! and files are 0600 from creation, including unpublished temporary files.
//! Windows inherits the enclosing user profile's ACL; the caller must keep
//! the store within that profile or an equivalent user-only ACL boundary.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::path::{Component, Path};

use crate::error::{WorkflowError, WorkflowResult};

pub(super) fn write_atomic(run_dir: &Path, relative: &Path, bytes: &[u8]) -> WorkflowResult<()> {
    prepare_parent(run_dir, relative)?;
    let target = run_dir.join(relative);
    // Exclusive creation avoids following or reusing an existing temporary
    // file, whose permissions or inode ownership may be inappropriate.
    let tmp = target.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp).map_err(|e| WorkflowError::io(&tmp, e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|e| WorkflowError::io(&tmp, e))?;
        }
        file.write_all(bytes)
            .map_err(|e| WorkflowError::io(&tmp, e))?;
        file.sync_all().map_err(|e| WorkflowError::io(&tmp, e))?;
    }
    // Issue 318: published like every other durable write.
    super::rename_durable(&tmp, &target)
}

/// Keep the same private boundary when records move to an archive. Validation
/// also applies here, so a move cannot create directories outside the run.
pub(crate) fn prepare_parent(run_dir: &Path, relative: &Path) -> WorkflowResult<()> {
    super::validate_run_relative_path(relative)?;
    fs::create_dir_all(run_dir).map_err(|e| WorkflowError::io(run_dir, e))?;
    let mut directory = run_dir.to_path_buf();
    if let Some(parent) = relative.parent() {
        for component in parent.components() {
            if let Component::Normal(part) = component {
                directory.push(part);
                private_directory(&directory)?;
            }
        }
    }
    Ok(())
}

fn private_directory(path: &Path) -> WorkflowResult<()> {
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    if let Err(error) = builder.create(path)
        && error.kind() != std::io::ErrorKind::AlreadyExists
    {
        return Err(WorkflowError::io(path, error));
    }
    let metadata = fs::symlink_metadata(path).map_err(|e| WorkflowError::io(path, e))?;
    if !metadata.file_type().is_dir() {
        return Err(WorkflowError::io(
            path,
            std::io::Error::other("private storage boundary must be a directory, not a symlink"),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|e| WorkflowError::io(path, e))?;
    }
    Ok(())
}
