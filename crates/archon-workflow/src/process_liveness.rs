//! Issue 339: whether a process id names a running process, on every platform.
//!
//! A marker or lock file that records its writer's pid is only safe to
//! reclaim when that writer is gone. A probe that answers "dead" for every
//! pid on one platform closes the work of a live owner there, so each
//! platform probes for real here and callers share this one answer.

/// Whether `pid` names a process that is still running.
///
/// The calling process counts as running; a caller that must not treat its
/// own pid as another owner checks `std::process::id()` itself. Pid 0 is never
/// a process the caller can own (on Unix `kill(0, 0)` probes the caller's own
/// process group, on Windows it is the System Idle Process), so it is not
/// running.
pub fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    platform_alive(pid)
}

#[cfg(unix)]
fn platform_alive(pid: u32) -> bool {
    // A pid above `pid_t::MAX` turns negative, and a negative pid names a
    // process group, not a process.
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 only probes whether the process exists; EPERM means it
    // exists and belongs to someone else.
    let probed = unsafe { libc::kill(pid, 0) };
    probed == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
fn platform_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, GetLastError, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: OpenProcess takes plain values and returns a null handle on
    // failure; nothing is borrowed.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        // SAFETY: reads this thread's last error, set by the failed call.
        // Access denied means the process exists but belongs to someone the
        // caller may not query; every other failure (ERROR_INVALID_PARAMETER
        // for a pid with no process) means there is no such process.
        return unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
    }
    let mut code: u32 = 0;
    // SAFETY: `handle` is a valid process handle opened above with query
    // rights, and `code` outlives the call.
    let queried = unsafe { GetExitCodeProcess(handle, &mut code) };
    // SAFETY: `handle` was opened above and is closed exactly once here.
    unsafe { CloseHandle(handle) };
    // A process that has exited keeps its object while any handle to it is
    // open, so a successful open alone does not mean it runs: only an exit
    // code other than STILL_ACTIVE proves it ended. A query that fails proves
    // nothing, and the object exists, so like access denied it counts as
    // running rather than letting a caller reclaim a live owner's work.
    queried == 0 || code == STILL_ACTIVE as u32
}

#[cfg(not(any(unix, windows)))]
compile_error!("process_alive has no liveness probe for this platform (Issue 339)");

#[cfg(test)]
#[path = "process_liveness_tests.rs"]
mod tests;
