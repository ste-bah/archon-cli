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
///
/// Only an answer that no process has `pid` (Unix ESRCH, Windows
/// ERROR_INVALID_PARAMETER) or that it has exited means not running. Any
/// other probe error proves nothing, so the process counts as running.
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
    // SAFETY: signal 0 only probes whether the process exists. Only ESRCH
    // means no such process; EPERM means it exists and belongs to someone
    // else.
    let probed = unsafe { libc::kill(pid, 0) };
    probed == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(windows)]
fn platform_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_INVALID_PARAMETER, GetLastError, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, WaitForSingleObject,
    };

    // SAFETY: OpenProcess takes plain values and returns a null handle on
    // failure; nothing is borrowed.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        // SAFETY: reads this thread's last error, set by the failed call.
        // As on Unix, where only ESRCH means no process: only
        // ERROR_INVALID_PARAMETER says no process has this pid. Every other
        // failure (access denied for another user's process, a resource or
        // quota error) proves nothing about the process, so it counts as
        // running rather than letting a caller reclaim a live owner's work.
        return unsafe { GetLastError() } != ERROR_INVALID_PARAMETER;
    }
    // SAFETY: `handle` is a valid process handle opened above with
    // SYNCHRONIZE; a zero timeout only polls its state.
    let waited = unsafe { WaitForSingleObject(handle, 0) };
    // SAFETY: `handle` was opened above and is closed exactly once here.
    unsafe { CloseHandle(handle) };
    // A process that has exited keeps its object while any handle to it is
    // open, so a successful open alone does not mean it runs. Its handle is
    // signaled exactly when it has exited, which, unlike an exit code
    // compared with STILL_ACTIVE (259), no exit status can imitate. A timeout
    // means it runs; a failed wait proves nothing and counts as running.
    waited != WAIT_OBJECT_0
}

#[cfg(not(any(unix, windows)))]
compile_error!("process_alive has no liveness probe for this platform (Issue 339)");

#[cfg(test)]
#[path = "process_liveness_tests.rs"]
mod tests;
