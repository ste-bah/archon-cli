//! What a supervised child inherits besides its stdio (Issue 334).
//!
//! A descriptor is inherited across `exec` unless it is close-on-exec. Where
//! std has no `pipe2` (macOS), it creates every pipe with `pipe()` and only
//! then sets `FD_CLOEXEC`, so a fork on another thread between the two calls
//! carries the new pipe into its child for good. A sibling command's output
//! pipe held that way by a long-lived descendant never reaches end of file,
//! and that sibling's clean teardown reads as a stall. The fix is on the
//! inheriting side: a supervised child marks every descriptor above its
//! stdio close-on-exec before it execs, whatever thread created it and when.

use std::io;

/// The first descriptor above stdin, stdout and stderr.
const FIRST_INHERITED: libc::c_int = 3;

/// One past the highest descriptor this process can hold, for
/// [`inherit_only_stdio`]. Read before the fork: nothing that reads it is
/// async-signal safe.
pub fn descriptor_ceiling() -> io::Result<libc::c_int> {
    if let Some(ceiling) = crate::process_nofile::ceiling() {
        return Ok(ceiling);
    }
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes only the struct it is given.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let soft = libc::c_int::try_from(limit.rlim_cur).unwrap_or(libc::c_int::MAX);
    #[cfg(target_vendor = "apple")]
    {
        // SAFETY: getdtablesize takes no arguments and cannot fail.
        let table = unsafe { libc::getdtablesize() };
        Ok(bounded_ceiling(soft, max_files_per_proc().ok(), table))
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        Ok(soft)
    }
}

/// The kernel also caps a process at `kern.maxfilesperproc` (`per_process`),
/// which is what bounds the table when the soft limit is unlimited. A sandbox
/// can deny that sysctl (Issue 340); the soft limit then still holds, bounded
/// by the descriptor table size (`table`, from `getdtablesize`, which the
/// kernel answers from the same two limits without a sysctl). Failing closed
/// there would refuse every spawn of a process that can still run children.
#[cfg_attr(not(target_vendor = "apple"), allow(dead_code))]
pub(crate) fn bounded_ceiling(
    soft: libc::c_int,
    per_process: Option<libc::c_int>,
    table: libc::c_int,
) -> libc::c_int {
    match per_process {
        Some(cap) => soft.min(cap),
        None => soft.min(table),
    }
}

#[cfg(target_vendor = "apple")]
fn max_files_per_proc() -> io::Result<libc::c_int> {
    let mut value: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: the name is NUL-terminated and the output buffer is `size`
    // bytes of a c_int this frame owns.
    let read = unsafe {
        libc::sysctlbyname(
            c"kern.maxfilesperproc".as_ptr(),
            (&raw mut value).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if read != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(value)
}

/// Marks every descriptor from 3 up to `ceiling` (from
/// [`descriptor_ceiling`]) close-on-exec, so the program a `pre_exec` hook
/// is about to exec inherits only its stdio. Async-signal safe: `fcntl`
/// (and on Linux `close_range`) only. Descriptors are flagged, not closed,
/// so std's own close-on-exec pipe that reports an exec failure still works.
pub fn inherit_only_stdio(ceiling: libc::c_int) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: close_range takes plain integers; with CLOSE_RANGE_CLOEXEC
        // it only sets a flag on this process's descriptors.
        let flagged = unsafe {
            libc::syscall(
                libc::SYS_close_range,
                FIRST_INHERITED as libc::c_uint,
                libc::c_uint::MAX,
                libc::CLOSE_RANGE_CLOEXEC,
            )
        };
        if flagged == 0 {
            return crate::process_nofile::restore();
        }
        // An older kernel: the loop below does the same, one by one.
    }
    for fd in FIRST_INHERITED..ceiling {
        // SAFETY: F_GETFD/F_SETFD only read and set this descriptor's flags.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 || flags & libc::FD_CLOEXEC != 0 {
            continue;
        }
        // SAFETY: as above.
        if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    crate::process_nofile::restore()
}

#[cfg(test)]
#[path = "process_tree_descriptors_tests.rs"]
mod tests;
