//! A process's identity (pid and start time), and signals that reach only
//! the process a scan saw.
//!
//! A pid alone is not an identity: once its process is reaped, the pid can
//! be handed to an unrelated process. Every signal here is preceded by a
//! fresh read of the start time. On Linux the signal then goes through a
//! pidfd opened before that read, so the process that was verified is the
//! one signalled. Elsewhere (macOS) the read and the signal are adjacent
//! syscalls, the closest the platform allows.

use std::io;

#[cfg(target_os = "macos")]
use super::Process;

/// A process as a scan saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pinned {
    pub pid: u32,
    pub start: u64,
}

/// The start time of the live (not zombie) process `pid`, if there is one.
pub fn start_of(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let process = super::parse_proc_stat(pid, &stat)?;
        (!process.zombie).then_some(process.start)
    }
    #[cfg(target_os = "macos")]
    {
        let info = bsd_info(libc::pid_t::try_from(pid).ok()?)?;
        (info.pbi_status != libc::SZOMB).then(|| start_of_info(&info))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

/// Send `signal` to `pinned` only if it is still the same process. Returns
/// whether the signal was delivered.
pub fn deliver(pinned: Pinned, signal: i32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pinned.pid) else {
        return false;
    };
    if pid <= 1 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        // SAFETY: pidfd_open takes integers; the descriptor is closed below.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd >= 0 {
            let fd = fd as libc::c_int;
            let same = start_of(pinned.pid) == Some(pinned.start);
            // SAFETY: a valid pidfd, a plain signal number, no siginfo.
            let sent = same
                && unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        fd,
                        signal,
                        std::ptr::null::<libc::siginfo_t>(),
                        0,
                    )
                } == 0;
            // SAFETY: closes the descriptor opened above, once.
            unsafe { libc::close(fd) };
            return sent;
        }
        // Only a kernel without pidfds (before 5.3) falls back; any other
        // error means the process is gone.
        if io::Error::last_os_error().raw_os_error() != Some(libc::ENOSYS) {
            return false;
        }
    }
    if start_of(pinned.pid) != Some(pinned.start) {
        return false;
    }
    // SAFETY: kill takes plain integers and touches no memory.
    unsafe { libc::kill(pid, signal) == 0 }
}

#[cfg(target_os = "macos")]
fn bsd_info(pid: libc::pid_t) -> Option<libc::proc_bsdinfo> {
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    // SAFETY: an all-zero proc_bsdinfo is valid output space.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    // SAFETY: the buffer and its size describe `info`.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size as libc::c_int,
        )
    };
    (usize::try_from(written).ok() == Some(size)).then_some(info)
}

#[cfg(target_os = "macos")]
fn start_of_info(info: &libc::proc_bsdinfo) -> u64 {
    info.pbi_start_tvsec
        .saturating_mul(1_000_000)
        .saturating_add(info.pbi_start_tvusec)
}

/// The process table from libproc: one `proc_pidinfo` per pid, no `ps`.
#[cfg(target_os = "macos")]
pub(super) fn snapshot_libproc() -> io::Result<Vec<Process>> {
    // SAFETY: a null buffer asks only for the count.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    let count = usize::try_from(count).map_err(|_| io::Error::last_os_error())?;
    // Room for processes started between the two calls.
    let mut pids: Vec<libc::pid_t> = vec![0; count + 256];
    let bytes = libc::c_int::try_from(pids.len() * std::mem::size_of::<libc::pid_t>())
        .map_err(|_| io::Error::other("process table too large"))?;
    // SAFETY: the buffer and its size in bytes describe `pids`.
    let listed = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    let listed = usize::try_from(listed).map_err(|_| io::Error::last_os_error())?;
    let mut processes = Vec::with_capacity(listed);
    for pid in pids.into_iter().take(listed).filter(|pid| *pid > 0) {
        // A process that exited since the listing, or that this user may
        // not inspect, has no entry.
        let Some(info) = bsd_info(pid) else { continue };
        // SAFETY: getsid takes a plain integer.
        let sid = unsafe { libc::getsid(pid) };
        processes.push(Process {
            pid: info.pbi_pid,
            ppid: info.pbi_ppid,
            pgid: info.pbi_pgid,
            sid: u32::try_from(sid).ok(),
            start: start_of_info(&info),
            zombie: info.pbi_status == libc::SZOMB,
        });
    }
    Ok(processes)
}
