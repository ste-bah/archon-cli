//! A call's staging tree, anchored when the host creates it (#297 round 7).
//!
//! The child can rename and replace entries of its run directory, so a path
//! the host resolves later may lead anywhere. The anchor opens
//! `host-command-staging` once, refusing a link, and every later host
//! mutation of the call's tree -- the sealing rewrite, secret removal, the
//! clear before a retry, removal after a failure -- resolves one component
//! at a time against that handle and never follows a link. A child that
//! swaps an ancestor or the call directory for a link changes what it sees,
//! never what the host chmods, rewrites or deletes. Before the host hands
//! the path to a child it checks that the path still leads to the anchored
//! directory.
//!
//! Windows limitation: std has no handle-relative API, so there each step
//! checks the components with `symlink_metadata` before it acts and refuses
//! any reparse point (a symbolic link, a junction or a mount point) present
//! at the check. It cannot close a swap between the check and the act, and
//! a child needs no privilege for that: a junction (`mklink /J`) can be
//! created by any user. Closing it needs handle-relative NT calls
//! (`NtCreateFile` with a `RootDirectory`, or `FILE_FLAG_OPEN_REPARSE_POINT`
//! with `SetFileInformationByHandle`), which this module does not make yet.
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use super::workflow_host_staging_anchor_unix as at;

pub(crate) const STAGING_DIR: &str = "host-command-staging";

pub(crate) struct StagingAnchor {
    root: PathBuf,
    name: OsString,
    #[cfg(unix)]
    parent: std::os::fd::OwnedFd,
}

impl std::fmt::Debug for StagingAnchor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StagingAnchor")
            .field("root", &self.root)
            .finish()
    }
}

impl StagingAnchor {
    /// Opens (creating) `run_root/host-command-staging` without following a
    /// link -- a link or file a child left there is removed, never followed --
    /// and creates an empty call directory `name` in it.
    pub(crate) fn create(run_root: &Path, name: &str) -> io::Result<Self> {
        let anchor = Self::open(run_root, name)?;
        anchor.reset()?;
        Ok(anchor)
    }

    /// The path the child is given, and the one evidence names.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Removes the call directory, then creates it empty again and checks
    /// that its path still leads to it.
    pub(crate) fn reset(&self) -> io::Result<()> {
        self.remove_tree()?;
        self.make_call_dir()?;
        self.verify()
    }

    #[cfg(unix)]
    fn open(run_root: &Path, name: &str) -> io::Result<Self> {
        let run = at::open_dir_path(run_root)?;
        let staging = std::ffi::OsStr::new(STAGING_DIR);
        let mut parent = None;
        for _ in 0..3 {
            match at::mkdir_at(&run, staging) {
                Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
                _ => {}
            }
            match at::open_dir_at(&run, staging) {
                Ok(fd) => {
                    parent = Some(fd);
                    break;
                }
                Err(error) if at::is_not_dir(&error) => at::unlink_at(&run, staging, false)?,
                // A child's `chmod 000`: owner access is restored through the
                // run directory's handle, never through a link (#297 r8).
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    at::unlock_dir_at(&run, staging)?
                }
                Err(error) => return Err(error),
            }
        }
        let parent = parent.ok_or_else(|| {
            io::Error::other(format!(
                "'{}' kept being replaced while the host created it",
                run_root.join(STAGING_DIR).display()
            ))
        })?;
        Ok(Self {
            root: run_root.join(STAGING_DIR).join(name),
            name: name.into(),
            parent,
        })
    }

    #[cfg(unix)]
    fn make_call_dir(&self) -> io::Result<()> {
        at::mkdir_at(&self.parent, &self.name)
    }

    /// Fails unless the path a child is given leads to the anchored directory.
    #[cfg(unix)]
    pub(crate) fn verify(&self) -> io::Result<()> {
        use std::os::unix::fs::MetadataExt;
        let anchored = at::fstat(&at::open_dir_at(&self.parent, &self.name)?)?;
        let seen = std::fs::metadata(&self.root)?;
        if (seen.dev(), seen.ino()) != (anchored.st_dev as u64, anchored.st_ino as u64) {
            return Err(io::Error::other(format!(
                "'{}' no longer leads to the staging directory the host created",
                self.root.display()
            )));
        }
        Ok(())
    }

    /// Removes the whole call tree, restoring owner access a child removed.
    #[cfg(unix)]
    pub(crate) fn remove_tree(&self) -> io::Result<()> {
        at::restore_owner_access(&self.parent)?;
        at::remove_entry(&self.parent, &self.name, 0)
    }

    /// The call directory; `None` when it is gone. A link or file in its
    /// place is an error: it was swapped after creation.
    #[cfg(unix)]
    fn call_dir(&self) -> io::Result<Option<std::os::fd::OwnedFd>> {
        match at::open_dir_at(&self.parent, &self.name) {
            Ok(fd) => Ok(Some(fd)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) if at::is_not_dir(&error) => Err(self.replaced()),
            Err(error) => Err(error),
        }
    }

    fn replaced(&self) -> io::Error {
        io::Error::other(format!(
            "the staging directory '{}' was replaced by a link or file after the host created it",
            self.root.display()
        ))
    }

    /// The bytes of the regular file `name` at the call root; `None` when it
    /// is absent or not a regular file (a link is never followed).
    #[cfg(unix)]
    pub(crate) fn read_file(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        match self.call_dir()? {
            Some(dir) => at::read_file_at(&dir, name.as_ref()),
            None => Ok(None),
        }
    }

    /// Replaces `name` at the call root with an owner-only file of `bytes`.
    #[cfg(unix)]
    pub(crate) fn write_file(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let dir = self.call_dir()?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("staging '{}' is gone", self.root.display()),
            )
        })?;
        at::replace_file_at(&dir, name.as_ref(), bytes)
    }

    /// Restricts the regular file `name` at the call root to its owner.
    #[cfg(unix)]
    pub(crate) fn owner_only(&self, name: &str) -> io::Result<()> {
        match self.call_dir()? {
            Some(dir) => at::owner_only_at(&dir, name.as_ref()),
            None => Ok(()),
        }
    }

    /// Removes the entry at `relative` (from the call root), resolving each
    /// component against the parent's handle without following a link. An
    /// absent entry or parent is already removed; a parent replaced by a
    /// link or file is an error, never followed (#297 round 8).
    #[cfg(unix)]
    pub(crate) fn remove_file(&self, relative: &Path) -> io::Result<()> {
        let Some(dir) = self.call_dir()? else {
            return Ok(());
        };
        let names: Vec<_> = relative.iter().collect();
        let Some((last, parents)) = names.split_last() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "an empty staging path names nothing to remove",
            ));
        };
        let mut current = dir;
        for name in parents {
            current = match at::open_dir_at(&current, name) {
                Ok(fd) => fd,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) if at::is_not_dir(&error) => {
                    return Err(io::Error::other(format!(
                        "'{}' was replaced by a link or file at '{}'",
                        self.root.join(relative).display(),
                        name.to_string_lossy()
                    )));
                }
                Err(error) => return Err(error),
            };
        }
        at::remove_entry(&current, last, 0)
    }

    /// Visits every non-directory entry of the call tree by its relative
    /// path (with `read`: regular files only, with their bytes); entries the
    /// visitor accepts are removed. Returns whether any was removed.
    #[cfg(unix)]
    pub(crate) fn scan(
        &self,
        read: bool,
        visit: &mut dyn FnMut(&Path, Option<&[u8]>) -> bool,
    ) -> io::Result<bool> {
        match self.call_dir()? {
            Some(dir) => at::scan(&dir, Path::new(""), read, visit, 0),
            None => Ok(false),
        }
    }
}

#[cfg(not(unix))]
#[path = "workflow_host_staging_anchor_windows.rs"]
mod windows;

#[cfg(all(test, unix))]
#[path = "workflow_host_staging_anchor_tests.rs"]
mod tests;
