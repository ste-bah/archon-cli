//! The Unix removal: only the tree's own path is resolved; every entry under
//! it is reached through an open directory handle, without following links
//! (`openat`/`fstatat`/`unlinkat`). A directory is listed again only through
//! the handle it was opened by, and its owner access is changed only through
//! a handle that is that very directory, never by path: a writer that swaps
//! an entry (for a symbolic link, a hard link or another directory) never
//! redirects the removal or a permission change outside the tree. A call the
//! kernel interrupted is made again under the phase's control.
//!
//! A directory no handle can open (no owner access at all, on systems
//! without `O_PATH`) cannot be moved to a private quarantine directory either:
//! moving a directory to another parent needs write access to it (its `..`
//! entry). It is renamed within its parent to a fresh unpredictable name,
//! matched there by device and inode, and only then given owner access by
//! that name. A writer racing at the known name swaps only what gets renamed
//! (which then fails the match, gets no permission change and is removed as an
//! entry of the tree). The remaining window: a writer that lists the parent,
//! finds the fresh name and replaces it between the match and the change.
use super::*;
use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;

/// Opens a directory for listing.
const LIST: libc::c_int = libc::O_RDONLY | libc::O_DIRECTORY;
/// Opens a directory as a handle that needs no read access to it, for a
/// permission change: `O_PATH` on Linux, search access (`O_SEARCH`) on
/// Apple systems. Elsewhere it is the listing open, refused again.
#[cfg(any(target_os = "linux", target_os = "android"))]
const HANDLE: libc::c_int = libc::O_PATH | libc::O_DIRECTORY;
#[cfg(target_vendor = "apple")]
const HANDLE: libc::c_int = libc::O_SEARCH | libc::O_DIRECTORY;
#[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
const HANDLE: libc::c_int = LIST;

pub(super) fn remove(path: &Path) -> WorkflowResult<()> {
    control::check()?;
    // Only the tree's own path is the caller's to resolve; nothing under
    // it is ever reached by path again.
    let Some(name) = path.file_name() else {
        return Err(invalid(format!(
            "owned tree has no name: {}",
            path.display()
        )));
    };
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let parent_c = CString::new(parent.as_os_str().as_bytes())
        .map_err(|_| invalid(format!("path holds a NUL: {}", path.display())))?;
    let parent_fd = match retried(|| open(&parent_c))? {
        Ok(fd) => fd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(WorkflowError::io(parent, e)),
    };
    let name = CString::new(name.as_bytes())
        .map_err(|_| invalid(format!("path holds a NUL: {}", path.display())))?;
    remove_at(&parent_fd, &name, path)
}

/// Removes the entry `name` of the directory `parent`, shown as `shown`.
/// One unrenewed watch spans every retry of this entry, a replaced identity
/// and each fresh name a permission recovery gives it included: a writer that
/// keeps substituting it or revoking its access ends as a resumable stall.
fn remove_at(parent: &OwnedFd, name: &CStr, shown: &Path) -> WorkflowResult<()> {
    let mut current = (name.to_owned(), shown.to_path_buf());
    let mut waiting = None;
    'entry: loop {
        let (name, shown): (&CStr, &Path) = (&current.0, &current.1);
        control::check()?;
        let stat = match retried(|| stat_at(parent, name))? {
            Ok(stat) => stat,
            Err(e) => return removed(shown, Err(e)),
        };
        if !is_dir(&stat) {
            return removed(shown, retried(|| unlink_at(parent, name, 0))?);
        }
        let dir = match open_dir_at(parent, name, &stat, shown)? {
            Ok(dir) => dir,
            // Replaced since its stat (a link, a file or another
            // directory now): judged again.
            Err(e) if matches!(e.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR)) => {
                wait_for_writer(&mut waiting)?;
                continue 'entry;
            }
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                let aside = match regain_aside(parent, name, &stat, shown)? {
                    Ok(aside) => aside,
                    Err(e) => return removed(shown, Err(e)),
                };
                // The same entry under its fresh name, in place (no
                // recursion), under the same watch.
                let aside_shown =
                    shown.with_file_name(std::ffi::OsStr::from_bytes(aside.to_bytes()));
                current = (aside, aside_shown);
                #[cfg(test)]
                tests::after_aside(&current.1);
                wait_for_writer(&mut waiting)?;
                continue 'entry;
            }
            Err(e) => return removed(shown, Err(e)),
        };
        let mode = (stat.st_mode & 0o7777) | 0o700;
        retried(|| fchmod(&dir, mode))?.map_err(|e| WorkflowError::io(shown, e))?;
        loop {
            let names = retried(|| names(&dir))?.map_err(|e| WorkflowError::io(shown, e))?;
            for child in names {
                let child_shown = shown.join(std::ffi::OsStr::from_bytes(child.to_bytes()));
                remove_at(&dir, &child, &child_shown)?;
            }
            #[cfg(test)]
            tests::after_listing(shown);
            match retried(|| unlink_at(parent, name, libc::AT_REMOVEDIR))? {
                Err(e) if e.kind() == io::ErrorKind::DirectoryNotEmpty => {
                    // A name that no longer holds this directory is
                    // judged again; this handle is never followed there.
                    let held = holds(parent, name, &dir, shown)?;
                    wait_for_writer(&mut waiting)?;
                    if !held {
                        continue 'entry;
                    }
                    #[cfg(test)]
                    tests::before_retry(shown);
                    if !holds(parent, name, &dir, shown)? {
                        continue 'entry;
                    }
                }
                Err(e) if e.raw_os_error() == Some(libc::ENOTDIR) => {
                    wait_for_writer(&mut waiting)?;
                    continue 'entry;
                }
                result => return removed(shown, result),
            }
        }
    }
}

/// `call` again while the kernel reports it interrupted; each retry is a
/// wait point of the active phase (cancellation, no-progress window).
fn retried<T>(mut call: impl FnMut() -> io::Result<T>) -> WorkflowResult<io::Result<T>> {
    loop {
        match call() {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                control::poll("an interrupted filesystem call")?;
            }
            result => return Ok(result),
        }
    }
}

/// The directory `name` of `parent` (seen as `stat`) that no handle opens:
/// renamed within `parent` to a fresh unpredictable name, and given owner
/// access by that name only if it is still the directory seen. Returns the
/// new name, which the caller removes as an entry of the tree whatever it is.
fn regain_aside(
    parent: &OwnedFd,
    name: &CStr,
    stat: &libc::stat,
    shown: &Path,
) -> WorkflowResult<io::Result<CString>> {
    let _ = shown;
    #[cfg(test)]
    tests::before_aside(shown);
    let aside = loop {
        let aside = aside_name()?;
        match retried(|| stat_at(parent, &aside))? {
            Err(e) if e.kind() == io::ErrorKind::NotFound => break aside,
            Err(e) => return Ok(Err(e)),
            Ok(_) => continue,
        }
    };
    if let Err(e) = retried(|| rename_at(parent, name, parent, &aside))? {
        return Ok(Err(e));
    }
    let now = match retried(|| stat_at(parent, &aside))? {
        Ok(now) => now,
        Err(e) => return Ok(Err(e)),
    };
    if is_dir(&now) && same(&now, stat) {
        let mode = (now.st_mode & 0o7777) | 0o700;
        if let Err(e) = retried(|| chmod_at(parent, &aside, mode))? {
            return Ok(Err(e));
        }
    }
    Ok(Ok(aside))
}

/// A fresh entry name nobody can predict: this process, a counter and a
/// randomly keyed hash of the time.
fn aside_name() -> WorkflowResult<CString> {
    use std::hash::{BuildHasher, Hasher};
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos()),
    );
    hasher.write_u64(n);
    CString::new(format!(
        ".archon-remove-{}-{n}-{:016x}",
        std::process::id(),
        hasher.finish()
    ))
    .map_err(|_| invalid("aside name holds a NUL"))
}

/// Opens the directory `name` for listing without following a link. A
/// directory this user may not list gets owner access through a handle that
/// is that same directory (opened as a directory, without following, and
/// matched by device and inode), then is opened for listing once more.
fn open_dir_at(
    parent: &OwnedFd,
    name: &CStr,
    stat: &libc::stat,
    shown: &Path,
) -> WorkflowResult<io::Result<OwnedFd>> {
    let _ = shown;
    match retried(|| open_at(parent, name, LIST))? {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {}
        result => return Ok(result),
    }
    #[cfg(test)]
    tests::before_chmod(shown);
    // A permission refusal here (no `O_PATH`, no owner search access)
    // leaves the directory to `regain_aside`.
    let handle = match retried(|| open_at(parent, name, HANDLE))? {
        Ok(handle) => handle,
        Err(e) => return Ok(Err(e)),
    };
    let own = match retried(|| fstat(&handle))? {
        Ok(own) => own,
        Err(e) => return Ok(Err(e)),
    };
    if !is_dir(&own) || !same(&own, stat) {
        return Ok(Err(replaced()));
    }
    if let Err(e) = retried(|| chmod_handle(&handle, (stat.st_mode & 0o7777) | 0o700))? {
        return Ok(Err(e));
    }
    let dir = match retried(|| open_at(parent, name, LIST))? {
        Ok(dir) => dir,
        Err(e) => return Ok(Err(e)),
    };
    Ok(match retried(|| fstat(&dir))? {
        Ok(now) if same(&now, &own) => Ok(dir),
        Ok(_) => Err(replaced()),
        Err(e) => Err(e),
    })
}

/// The error that has an entry judged again: its name holds another object.
fn replaced() -> io::Error {
    io::Error::from_raw_os_error(libc::ENOTDIR)
}

fn is_dir(stat: &libc::stat) -> bool {
    stat.st_mode & libc::S_IFMT == libc::S_IFDIR
}

fn same(a: &libc::stat, b: &libc::stat) -> bool {
    a.st_dev == b.st_dev && a.st_ino == b.st_ino
}

/// Whether `name` in `parent` is still the directory open as `dir`.
fn holds(parent: &OwnedFd, name: &CStr, dir: &OwnedFd, shown: &Path) -> WorkflowResult<bool> {
    let own = retried(|| fstat(dir))?.map_err(|e| WorkflowError::io(shown, e))?;
    match retried(|| stat_at(parent, name))? {
        Ok(now) => Ok(is_dir(&now) && same(&now, &own)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(WorkflowError::io(shown, e)),
    }
}

/// A test may have the next call report an interruption (EINTR).
fn injected() -> io::Result<()> {
    #[cfg(test)]
    if tests::interrupt() {
        return Err(io::Error::from_raw_os_error(libc::EINTR));
    }
    Ok(())
}

fn last(result: libc::c_int) -> io::Result<libc::c_int> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

fn owned(fd: libc::c_int) -> io::Result<OwnedFd> {
    // SAFETY: a non-negative result is a descriptor nothing else owns.
    last(fd).map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })
}

fn open(path: &CStr) -> io::Result<OwnedFd> {
    injected()?;
    // SAFETY: a valid C string and flags; the descriptor is owned.
    owned(unsafe { libc::open(path.as_ptr(), LIST | libc::O_CLOEXEC) })
}

fn open_at(parent: &OwnedFd, name: &CStr, flags: libc::c_int) -> io::Result<OwnedFd> {
    injected()?;
    let flags = flags | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: an open descriptor, a C string and flags; the result is owned.
    owned(unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) })
}

fn stat_at(parent: &OwnedFd, name: &CStr) -> io::Result<libc::stat> {
    injected()?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: an open descriptor, a C string and a stat buffer.
    last(unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    })?;
    // SAFETY: fstatat filled the buffer.
    Ok(unsafe { stat.assume_init() })
}

fn fstat(fd: &OwnedFd) -> io::Result<libc::stat> {
    injected()?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: an open descriptor and a stat buffer.
    last(unsafe { libc::fstat(fd.as_raw_fd(), stat.as_mut_ptr()) })?;
    // SAFETY: fstat filled the buffer.
    Ok(unsafe { stat.assume_init() })
}

fn unlink_at(parent: &OwnedFd, name: &CStr, flags: libc::c_int) -> io::Result<()> {
    injected()?;
    // SAFETY: an open descriptor, a C string and flags.
    last(unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) }).map(drop)
}

fn rename_at(from: &OwnedFd, name: &CStr, to: &OwnedFd, to_name: &CStr) -> io::Result<()> {
    injected()?;
    // SAFETY: open descriptors and C strings.
    last(unsafe {
        libc::renameat(
            from.as_raw_fd(),
            name.as_ptr(),
            to.as_raw_fd(),
            to_name.as_ptr(),
        )
    })
    .map(drop)
}

/// Changes the mode of `name` in `parent` by name, never following a link:
/// only for a directory just matched under its fresh name (`regain_aside`).
fn chmod_at(parent: &OwnedFd, name: &CStr, mode: libc::mode_t) -> io::Result<()> {
    injected()?;
    // SAFETY: an open descriptor, a C string, a mode and a flag.
    last(unsafe {
        libc::fchmodat(
            parent.as_raw_fd(),
            name.as_ptr(),
            mode,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    })
    .map(drop)
}

fn fchmod(fd: &OwnedFd, mode: libc::mode_t) -> io::Result<()> {
    injected()?;
    // SAFETY: an open descriptor and a mode.
    last(unsafe { libc::fchmod(fd.as_raw_fd(), mode) }).map(drop)
}

/// Changes the mode of the directory `handle` ([`HANDLE`]) is. Linux cannot
/// `fchmod` an `O_PATH` handle: its `/proc/self/fd` entry names exactly that
/// inode. Without `/proc` the change is refused, never made by path.
fn chmod_handle(handle: &OwnedFd, mode: libc::mode_t) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        injected()?;
        let path = CString::new(format!("/proc/self/fd/{}", handle.as_raw_fd()))
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        // SAFETY: a valid C string and a mode.
        match last(unsafe { libc::chmod(path.as_ptr(), mode) }) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "no /proc/self/fd to change a directory's mode through its handle",
            )),
            result => result.map(drop),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        fchmod(handle, mode)
    }
}

/// Clears `errno`, so a null `readdir` can be told apart: end or failure.
fn clear_errno() {
    // SAFETY: the calling thread's errno location is always valid.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    unsafe {
        *libc::__errno_location() = 0;
    }
    // SAFETY: the calling thread's errno location is always valid.
    #[cfg(any(target_vendor = "apple", target_os = "freebsd"))]
    unsafe {
        *libc::__error() = 0;
    }
}

/// The entry names of `dir`, listed from its start through its handle. A
/// failed read is an error, never the end of the listing.
fn names(dir: &OwnedFd) -> io::Result<Vec<CString>> {
    injected()?;
    // SAFETY: dup of an open descriptor; the copy is owned below.
    let copy = owned(unsafe { libc::dup(dir.as_raw_fd()) })?;
    // SAFETY: a fresh descriptor; on success the stream owns it.
    let stream = unsafe { libc::fdopendir(copy.as_raw_fd()) };
    if stream.is_null() {
        return Err(io::Error::last_os_error());
    }
    // The stream owns the copy from here on; closedir closes it.
    std::mem::forget(copy);
    struct Stream(*mut libc::DIR);
    impl Drop for Stream {
        fn drop(&mut self) {
            // SAFETY: a valid stream, closed once (with its descriptor).
            unsafe { libc::closedir(self.0) };
        }
    }
    let stream = Stream(stream);
    // The copy shares its offset with `dir`: list from the start.
    // SAFETY: a valid stream.
    unsafe { libc::rewinddir(stream.0) };
    let mut names = Vec::new();
    loop {
        #[cfg(test)]
        if let Some(errno) = tests::listing_fault() {
            return Err(io::Error::from_raw_os_error(errno));
        }
        clear_errno();
        // SAFETY: a valid stream; the entry is read before the next call.
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            let error = io::Error::last_os_error();
            if error.raw_os_error().is_some_and(|errno| errno != 0) {
                return Err(error);
            }
            return Ok(names);
        }
        // SAFETY: readdir returned a valid entry with a C string name.
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if name.to_bytes() != b"." && name.to_bytes() != b".." {
            names.push(name.to_owned());
        }
    }
}
