//! A process's identity (pid and start time), the process table, and signals
//! that reach only the process a scan pinned.
//!
//! A pid alone is not an identity: once its process is reaped, the pid can
//! be handed to an unrelated process.
//!
//! - Linux: a member's pidfd is opened when the member is adopted, and its
//!   start time is checked after the open; every later signal goes through
//!   that pidfd, which names that process and no other for its lifetime.
//!   The residual: `/proc` reports start times in clock ticks, so a pid
//!   reused within the same tick as its predecessor's death would pass the
//!   check at adoption. Linux allocates pids sequentially, so that needs the
//!   pid space to wrap within one tick.
//! - macOS: there is no pidfd. The start time is read again immediately
//!   before every `kill`. The residual: the process can exit, and its pid be
//!   reused, between that read and the `kill`. Closing that window needs
//!   kernel support macOS does not offer; it is accepted and documented.

use std::collections::BTreeSet;
use std::io;
use std::time::Instant;

use super::Process;

/// A process as a scan saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pinned {
    pub pid: u32,
    pub start: u64,
}

/// One scan of the process table.
#[derive(Debug, Clone, Default)]
pub struct Table {
    /// Every process the scan could read.
    pub processes: Vec<Process>,
    /// Every pid the kernel listed, read or not: a listed pid that could not
    /// be read may still be alive, so it is never taken for gone.
    pub listed: BTreeSet<u32>,
}

fn timed_out() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "process table scan ran out of time",
    )
}

/// The process table, read within `deadline`: syscalls and `/proc` only, no
/// subprocess. A scan that runs out of time, or that the kernel could not
/// list completely, is an error, never a smaller table.
pub fn snapshot_until(deadline: Instant) -> io::Result<Table> {
    #[cfg(target_os = "linux")]
    {
        snapshot_proc(deadline)
    }
    #[cfg(target_os = "macos")]
    {
        snapshot_libproc(deadline)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = deadline;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "process-tree scans are implemented for Linux and macOS only",
        ))
    }
}

#[cfg(target_os = "linux")]
fn snapshot_proc(deadline: Instant) -> io::Result<Table> {
    let mut table = Table::default();
    for entry in std::fs::read_dir("/proc")? {
        if Instant::now() >= deadline {
            return Err(timed_out());
        }
        let entry = entry?;
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse().ok()) else {
            continue;
        };
        table.listed.insert(pid);
        // A process that exits between the listing and the read is gone.
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        if let Some(process) = super::parse_proc_stat(pid, &stat) {
            table.processes.push(process);
        }
    }
    Ok(table)
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

/// Send `signal` to `pinned` only if it is still the same process (see the
/// module docs for each platform's guarantee). For a tracked member, the
/// tracker signals through the pidfd it opened at adoption instead.
pub fn deliver(pinned: Pinned, signal: i32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pinned.pid) else {
        return false;
    };
    if pid <= 1 {
        return false;
    }
    #[cfg(target_os = "linux")]
    if let Some(fd) = pidfd::open(pinned) {
        return pidfd::send(&fd, signal);
    }
    // macOS (and a Linux kernel without pidfds): the read and the signal
    // are adjacent; the window between them is the accepted residual.
    if start_of(pinned.pid) != Some(pinned.start) {
        return false;
    }
    // SAFETY: kill takes plain integers and touches no memory.
    unsafe { libc::kill(pid, signal) == 0 }
}

/// Whether the child `pid` of this process has exited, without reaping it
/// (`waitid` with `WNOWAIT`): an unreaped child keeps its pid, so its
/// process group and session cannot be reused while teardown runs.
pub fn exited(pid: u32) -> io::Result<bool> {
    let id = libc::id_t::from(pid);
    // SAFETY: an all-zero siginfo is valid output space.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is valid for the call; the flags only query.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            id,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    #[cfg(target_os = "linux")]
    // SAFETY: waitid filled `info` for a child-state change, or left it zero.
    let reported = unsafe { info.si_pid() };
    #[cfg(not(target_os = "linux"))]
    let reported = info.si_pid;
    Ok(reported != 0)
}

#[cfg(target_os = "linux")]
pub(super) mod pidfd {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    use super::{Pinned, start_of};

    /// A pidfd for `pinned`, if it is still that process: the pidfd is opened
    /// first and the start time read after, so the descriptor names the
    /// process whose start time matched. `None` also on a kernel without
    /// pidfds, where the caller falls back to verify-then-kill.
    pub(crate) fn open(pinned: Pinned) -> Option<OwnedFd> {
        let pid = libc::pid_t::try_from(pinned.pid).ok()?;
        // SAFETY: pidfd_open takes integers; ownership of the result is taken.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        let fd = libc::c_int::try_from(fd).ok().filter(|fd| *fd >= 0)?;
        // SAFETY: the descriptor was just returned to this process.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        (start_of(pinned.pid) == Some(pinned.start)).then_some(fd)
    }

    /// Signal the process `fd` names; false once it has exited.
    pub(crate) fn send(fd: &OwnedFd, signal: i32) -> bool {
        // SAFETY: a valid pidfd, a plain signal number, no siginfo.
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                fd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            ) == 0
        }
    }
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

/// Every pid the kernel lists. A listing that fills its buffer may have been
/// cut short, so it is asked again with a larger one; a zero or negative
/// answer is a failed listing, never an empty table.
#[cfg(target_os = "macos")]
fn list_pids() -> io::Result<Vec<libc::pid_t>> {
    // SAFETY: a null buffer asks only for the count.
    let estimate = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    let mut capacity = usize::try_from(estimate)
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| io::Error::other("the kernel listed no processes"))?
        + 256;
    for _ in 0..8 {
        let mut pids: Vec<libc::pid_t> = vec![0; capacity];
        let bytes = libc::c_int::try_from(capacity * std::mem::size_of::<libc::pid_t>())
            .map_err(|_| io::Error::other("process table too large"))?;
        // SAFETY: the buffer and its size in bytes describe `pids`.
        let listed = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
        let listed = usize::try_from(listed)
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| io::Error::other("listing the processes failed"))?;
        if listed < capacity {
            pids.truncate(listed);
            return Ok(pids);
        }
        capacity *= 2;
    }
    Err(io::Error::other(
        "the process table kept outgrowing its buffer",
    ))
}

/// The process table from libproc: one `proc_pidinfo` per pid, no `ps`.
#[cfg(target_os = "macos")]
fn snapshot_libproc(deadline: Instant) -> io::Result<Table> {
    let pids = list_pids()?;
    let mut table = Table::default();
    for pid in pids.into_iter().filter(|pid| *pid > 0) {
        if Instant::now() >= deadline {
            return Err(timed_out());
        }
        table.listed.insert(pid as u32);
        // A process that exited since the listing, or that this user may
        // not inspect, has no entry; it stays listed.
        let Some(info) = bsd_info(pid) else { continue };
        // SAFETY: getsid takes a plain integer.
        let sid = unsafe { libc::getsid(pid) };
        table.processes.push(Process {
            pid: info.pbi_pid,
            ppid: info.pbi_ppid,
            pgid: info.pbi_pgid,
            sid: u32::try_from(sid).ok(),
            start: start_of_info(&info),
            zombie: info.pbi_status == libc::SZOMB,
        });
    }
    Ok(table)
}
