//! Directory-handle-relative, no-follow file operations for the staging
//! anchor (#297 round 7). Every name is one component, resolved against a
//! directory handle; no call here follows a link the child left.
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// A child can nest directories without bound; removal stops (and the run
/// pauses) instead of exhausting descriptors or the stack.
const MAX_DEPTH: usize = 256;
/// Passes over one entry or directory a still-running writer keeps changing.
const RETRIES: usize = 16;
const DIR_FLAGS: libc::c_int =
    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

fn cvt(ret: libc::c_int) -> io::Result<libc::c_int> {
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

fn component(name: &OsStr) -> io::Result<CString> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "'{}' is not one staging path component",
                name.to_string_lossy()
            ),
        ));
    }
    CString::new(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

fn errno_is(error: &io::Error, codes: &[libc::c_int]) -> bool {
    error
        .raw_os_error()
        .is_some_and(|code| codes.contains(&code))
}

/// The entry exists but is not a directory: a file, a link (`O_NOFOLLOW`
/// refuses it with `ELOOP`, or `EMLINK` on some BSDs), a FIFO or a socket.
pub(super) fn is_not_dir(error: &io::Error) -> bool {
    errno_is(error, &[libc::ENOTDIR, libc::ELOOP, libc::EMLINK])
}

pub(super) fn open_dir_path(path: &Path) -> io::Result<OwnedFd> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    // SAFETY: a valid NUL-terminated path; the returned descriptor is owned.
    let fd = cvt(unsafe { libc::open(path.as_ptr(), DIR_FLAGS) })?;
    // SAFETY: `fd` was just opened and is owned by nothing else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

pub(super) fn open_dir_at(dir: &OwnedFd, name: &OsStr) -> io::Result<OwnedFd> {
    let name = component(name)?;
    // SAFETY: a valid directory descriptor and one NUL-terminated component.
    let fd = cvt(unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), DIR_FLAGS) })?;
    // SAFETY: `fd` was just opened and is owned by nothing else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

pub(super) fn mkdir_at(dir: &OwnedFd, name: &OsStr) -> io::Result<()> {
    let name = component(name)?;
    // SAFETY: as for `open_dir_at`. `mkdirat` never follows a final link.
    cvt(unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), 0o777) }).map(drop)
}

pub(super) fn unlink_at(dir: &OwnedFd, name: &OsStr, directory: bool) -> io::Result<()> {
    let name = component(name)?;
    let flags = if directory { libc::AT_REMOVEDIR } else { 0 };
    // SAFETY: as for `open_dir_at`. `unlinkat` removes a link, never its target.
    match cvt(unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), flags) }) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other.map(drop),
    }
}

pub(super) fn fstat(fd: &OwnedFd) -> io::Result<libc::stat> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: a valid descriptor and a buffer `fstat` fully initializes on success.
    cvt(unsafe { libc::fstat(fd.as_raw_fd(), stat.as_mut_ptr()) })?;
    // SAFETY: `fstat` succeeded.
    Ok(unsafe { stat.assume_init() })
}

fn kind_at(dir: &OwnedFd, name: &OsStr) -> io::Result<libc::mode_t> {
    let name = component(name)?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: as for `open_dir_at`; `AT_SYMLINK_NOFOLLOW` describes a link itself.
    cvt(unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    })?;
    // SAFETY: `fstatat` succeeded.
    Ok(unsafe { stat.assume_init() }.st_mode & libc::S_IFMT)
}

/// Give the owner traversal and write access to an opened directory again.
pub(super) fn restore_owner_access(fd: &OwnedFd) -> io::Result<()> {
    let mode = fstat(fd)?.st_mode & 0o7777;
    if mode & 0o700 != 0o700 {
        // SAFETY: a valid descriptor; the inode is pinned by it.
        cvt(unsafe { libc::fchmod(fd.as_raw_fd(), (mode | 0o700) as libc::mode_t) })?;
    }
    Ok(())
}

/// Make the directory `name` openable without following a link: a child's
/// `chmod 000` must not stop removal, and a swapped-in link is never chmodded.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn unlock_dir_at(dir: &OwnedFd, name: &OsStr) -> io::Result<()> {
    let name = component(name)?;
    let flags = libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: as for `open_dir_at`. An `O_PATH` handle needs no permission.
    let fd = cvt(unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) })?;
    // SAFETY: `fd` was just opened and is owned by nothing else.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    if fstat(&fd)?.st_mode & libc::S_IFMT != libc::S_IFDIR {
        return Ok(());
    }
    // Linux has no fchmod on an O_PATH handle; the /proc link names exactly
    // the pinned inode (it needs /proc, as glibc's own fchmodat does).
    let proc = CString::new(format!("/proc/self/fd/{}", fd.as_raw_fd()))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    // SAFETY: a valid NUL-terminated path.
    cvt(unsafe { libc::chmod(proc.as_ptr(), 0o700) }).map(drop)
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn unlock_dir_at(dir: &OwnedFd, name: &OsStr) -> io::Result<()> {
    if kind_at(dir, name)? != libc::S_IFDIR {
        return Ok(());
    }
    let name = component(name)?;
    // SAFETY: as for `open_dir_at`. With `AT_SYMLINK_NOFOLLOW` a link swapped
    // in since the check is changed itself, never its target.
    cvt(unsafe {
        libc::fchmodat(
            dir.as_raw_fd(),
            name.as_ptr(),
            0o700,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    })
    .map(drop)
}

#[cfg(any(target_os = "linux", target_os = "emscripten", target_os = "dragonfly"))]
fn errno() -> *mut libc::c_int {
    // SAFETY: the thread's errno location.
    unsafe { libc::__errno_location() }
}
#[cfg(any(target_vendor = "apple", target_os = "freebsd"))]
fn errno() -> *mut libc::c_int {
    // SAFETY: the thread's errno location.
    unsafe { libc::__error() }
}
#[cfg(any(target_os = "android", target_os = "netbsd", target_os = "openbsd"))]
fn errno() -> *mut libc::c_int {
    // SAFETY: the thread's errno location.
    unsafe { libc::__errno() }
}

/// The names in `dir`, without `.` and `..`. A read error is an error, never
/// a shorter listing: the secret scan must see every entry.
#[cfg(any(
    target_os = "linux",
    target_os = "emscripten",
    target_os = "dragonfly",
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "android",
    target_os = "netbsd",
    target_os = "openbsd"
))]
pub(super) fn list(dir: &OwnedFd) -> io::Result<Vec<OsString>> {
    // SAFETY: duplicating a valid descriptor; the copy is owned by the stream.
    let copy = cvt(unsafe { libc::fcntl(dir.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) })?;
    // SAFETY: `copy` is a valid directory descriptor the stream takes over.
    let stream = unsafe { libc::fdopendir(copy) };
    if stream.is_null() {
        let error = io::Error::last_os_error();
        // SAFETY: `copy` was not taken over.
        unsafe { libc::close(copy) };
        return Err(error);
    }
    // The copy shares the handle's offset, which an earlier listing moved.
    // SAFETY: `stream` is a valid open stream until `closedir` below.
    unsafe { libc::rewinddir(stream) };
    let mut names = Vec::new();
    let result = loop {
        // SAFETY: the errno location of this thread.
        unsafe { *errno() = 0 };
        // SAFETY: `stream` is valid; the entry lives until the next call.
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            // SAFETY: as above.
            let code = unsafe { *errno() };
            break if code == 0 {
                Ok(())
            } else {
                Err(io::Error::from_raw_os_error(code))
            };
        }
        // SAFETY: `d_name` is a NUL-terminated name inside the live entry.
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name != b"." && name != b".." {
            names.push(OsStr::from_bytes(name).to_owned());
        }
    };
    // SAFETY: `stream` is valid and closed exactly once (with `copy`).
    unsafe { libc::closedir(stream) };
    result.map(|()| names)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "emscripten",
    target_os = "dragonfly",
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "android",
    target_os = "netbsd",
    target_os = "openbsd"
)))]
pub(super) fn list(_: &OwnedFd) -> io::Result<Vec<OsString>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "listing staging without following links is not implemented on this platform",
    ))
}

/// Opens the regular file `name` without following a link or blocking on a
/// FIFO. `None`: absent, or not a regular file.
pub(super) fn open_file_at(dir: &OwnedFd, name: &OsStr) -> io::Result<Option<File>> {
    let c_name = component(name)?;
    let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
    // SAFETY: as for `open_dir_at`.
    let fd = match cvt(unsafe { libc::openat(dir.as_raw_fd(), c_name.as_ptr(), flags) }) {
        Ok(fd) => fd,
        Err(error) if error.kind() == io::ErrorKind::NotFound || is_not_dir(&error) => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    // SAFETY: `fd` was just opened and is owned by nothing else.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    if fstat(&fd)?.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Ok(None);
    }
    Ok(Some(File::from(fd)))
}

pub(super) fn read_file_at(dir: &OwnedFd, name: &OsStr) -> io::Result<Option<Vec<u8>>> {
    let Some(mut file) = open_file_at(dir, name)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// Restricts the regular file `name` to its owner through its own handle.
pub(super) fn owner_only_at(dir: &OwnedFd, name: &OsStr) -> io::Result<()> {
    if let Some(file) = open_file_at(dir, name)? {
        // SAFETY: a valid descriptor; the inode is pinned by it.
        cvt(unsafe { libc::fchmod(file.as_raw_fd(), 0o600) })?;
    }
    Ok(())
}

/// Replaces `name` with an owner-only regular file holding `bytes`: written
/// to a fresh file in the same directory, then renamed over the entry, so a
/// link, FIFO or directory the child left is replaced, never written through.
pub(super) fn replace_file_at(dir: &OwnedFd, name: &OsStr, bytes: &[u8]) -> io::Result<()> {
    let mut temporary = OsString::from(".host-seal.");
    temporary.push(name);
    remove_entry(dir, &temporary, 0)?;
    let c_temporary = component(&temporary)?;
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: as for `open_dir_at`; `O_EXCL` never opens an existing entry.
    let fd = cvt(unsafe { libc::openat(dir.as_raw_fd(), c_temporary.as_ptr(), flags, 0o600) })?;
    // SAFETY: `fd` was just opened and is owned by nothing else.
    let mut file = File::from(unsafe { OwnedFd::from_raw_fd(fd) });
    // SAFETY: a valid descriptor.
    cvt(unsafe { libc::fchmod(file.as_raw_fd(), 0o600) })?;
    file.write_all(bytes)?;
    file.sync_all()?;
    let c_name = component(name)?;
    let rename = |dir: &OwnedFd| {
        // SAFETY: two components of one valid directory descriptor.
        cvt(unsafe {
            libc::renameat(
                dir.as_raw_fd(),
                c_temporary.as_ptr(),
                dir.as_raw_fd(),
                c_name.as_ptr(),
            )
        })
    };
    if let Err(error) = rename(dir) {
        // A directory under the name cannot be renamed over: remove it first.
        if !errno_is(&error, &[libc::EISDIR, libc::ENOTEMPTY, libc::EEXIST]) {
            return Err(error);
        }
        remove_entry(dir, name, 0)?;
        rename(dir)?;
    }
    Ok(())
}

/// Removes the entry `name` of `dir` and, for a directory, everything in
/// it, restoring owner access a child took away. Links are removed, never
/// followed; a directory that keeps changing is an error, never skipped.
pub(super) fn remove_entry(dir: &OwnedFd, name: &OsStr, depth: usize) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Err(io::Error::other(format!(
            "staging is nested deeper than {MAX_DEPTH} directories"
        )));
    }
    for _ in 0..RETRIES {
        let kind = match kind_at(dir, name) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            kind => kind?,
        };
        if kind != libc::S_IFDIR {
            match unlink_at(dir, name, false) {
                // Swapped for a directory since: the next pass removes it.
                Err(_) if kind_at(dir, name).ok() == Some(libc::S_IFDIR) => continue,
                other => return other,
            }
        }
        match open_dir_at(dir, name) {
            Ok(child) => {
                restore_owner_access(&child)?;
                empty(&child, depth + 1)?;
                match unlink_at(dir, name, true) {
                    Err(error) if errno_is(&error, &[libc::ENOTEMPTY, libc::EEXIST]) => continue,
                    other => return other,
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            // Swapped for a link or file since: the next pass unlinks it.
            Err(error) if is_not_dir(&error) => continue,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                unlock_dir_at(dir, name)?;
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other(format!(
        "staging entry '{}' kept changing while it was removed",
        name.to_string_lossy()
    )))
}

fn empty(dir: &OwnedFd, depth: usize) -> io::Result<()> {
    for _ in 0..RETRIES {
        let names = list(dir)?;
        if names.is_empty() {
            return Ok(());
        }
        for name in names {
            remove_entry(dir, &name, depth)?;
        }
    }
    Err(io::Error::other(
        "a staging directory kept refilling while it was removed",
    ))
}

/// Visits every non-directory entry under `dir` (`rel` is its path from the
/// call root). With `read`, only regular files, with their bytes. The
/// visitor returns whether to remove the entry; the result is whether any
/// entry was removed. An unreadable directory is an error.
pub(super) fn scan(
    dir: &OwnedFd,
    rel: &Path,
    read: bool,
    visit: &mut dyn FnMut(&Path, Option<&[u8]>) -> bool,
    depth: usize,
) -> io::Result<bool> {
    if depth > MAX_DEPTH {
        return Err(io::Error::other(format!(
            "staging is nested deeper than {MAX_DEPTH} directories"
        )));
    }
    let mut removed = false;
    for name in list(dir)? {
        let path = rel.join(&name);
        let kind = match kind_at(dir, &name) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            kind => kind?,
        };
        if kind == libc::S_IFDIR {
            match open_dir_at(dir, &name) {
                Ok(child) => {
                    removed |= scan(&child, &path, read, visit, depth + 1)?;
                    continue;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                // Swapped for a link or file since: visited as one.
                Err(error) if is_not_dir(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let bytes = if read {
            match read_file_at(dir, &name)? {
                Some(bytes) => Some(bytes),
                None => continue,
            }
        } else {
            None
        };
        if visit(&path, bytes.as_deref()) {
            unlink_at(dir, &name, false)?;
            removed = true;
        }
    }
    Ok(removed)
}
