//! Removing a tree this host owns while a writer outside this process may
//! still add entries to it (#534). A process a check left running can create
//! an entry in a directory after the removal listed it and before the
//! removal deletes the directory (`DirectoryNotEmpty`): the removal lists the
//! directory again rather than failing the site. Those retries wait under
//! their own unrenewed window ([`control::outside_wait`]), so the removal of
//! the entries the writer keeps adding never renews it: a writer that never
//! stops ends as the phase's resumable stall, never as an endless retry or a
//! silent success. An entry another remover took first is removed.
//!
//! On Unix, everything under the given path is reached only through open
//! directory handles with no-follow semantics (`acceptance_scratch_io_remove_unix.rs`):
//! a writer that swaps an entry never redirects the removal or a permission
//! change outside the tree.
use super::*;

pub(crate) fn remove_owned_tree(path: &Path) -> WorkflowResult<()> {
    #[cfg(unix)]
    {
        unix::remove(path)
    }
    #[cfg(not(unix))]
    {
        other::remove(path)
    }
}

/// The outcome of removing `path`: absent is removed, whoever removed it.
fn removed(path: &Path, result: std::io::Result<()>) -> WorkflowResult<()> {
    match result {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(WorkflowError::io(path, e)),
        _ => Ok(()),
    }
}

/// One wait before listing a directory again after `DirectoryNotEmpty`.
fn wait_for_writer(waiting: &mut Option<control::Control>) -> WorkflowResult<()> {
    waiting
        .get_or_insert_with(control::outside_wait)
        .poll("a writer that keeps adding entries to an owned directory")?;
    std::thread::sleep(std::time::Duration::from_millis(10));
    Ok(())
}

#[cfg(unix)]
#[path = "acceptance_scratch_io_remove_unix.rs"]
mod unix;

#[cfg(not(unix))]
mod other {
    use super::*;

    /// The standard library's removal opens each entry by handle without
    /// following links on Windows; `DirectoryNotEmpty` lists it again.
    pub(super) fn remove(path: &Path) -> WorkflowResult<()> {
        control::check()?;
        let meta = match path.symlink_metadata() {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(WorkflowError::io(path, e)),
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return removed(path, std::fs::remove_file(path));
        }
        let mut waiting = None;
        loop {
            match std::fs::remove_dir_all(path) {
                Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
                    wait_for_writer(&mut waiting)?;
                }
                result => return removed(path, result),
            }
        }
    }
}

#[cfg(test)]
#[path = "acceptance_scratch_io_remove_tests.rs"]
pub(crate) mod tests;
