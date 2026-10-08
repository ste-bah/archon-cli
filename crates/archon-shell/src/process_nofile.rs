//! Bound Apple descriptor sweeps before the runtime creates threads.
use std::io;
use std::sync::OnceLock;

/// The soft NOFILE limit archon runs under on Apple systems, when the inherited
/// one is higher. Children get the inherited limit back. 10240 keeps headroom
/// for archon's own stores and servers while a sweep stays near a millisecond.
pub const STARTUP_SOFT_LIMIT: u64 = 10_240;

static DEGRADED: OnceLock<String> = OnceLock::new();

/// Lower the parent's soft NOFILE limit, retaining the original for children.
/// Other platforms use atomic CLOEXEC creation and need no startup cap.
///
/// This never stops startup. If the limit cannot be lowered (for example a
/// sandbox denies `/dev/fd`), nothing is changed: every spawn then sweeps up
/// to the full soft limit, which is complete but slower, and the reason is
/// kept for [`startup_degradation`] so that the caller can warn about it once
/// logging is running.
///
/// # Safety
/// Call before creating threads or descriptors concurrently. Lowering a limit
/// does not close inherited high descriptors; their upper bound is captured first.
pub unsafe fn initialize() {
    #[cfg(target_vendor = "apple")]
    // SAFETY: the caller guarantees startup isolation.
    unsafe {
        initialize_from(std::path::Path::new("/dev/fd"));
    }
}

/// [`initialize`], reading the open descriptors from `fd_dir`.
///
/// # Safety
/// As for [`initialize`].
#[cfg(target_vendor = "apple")]
pub(crate) unsafe fn initialize_from(fd_dir: &std::path::Path) {
    if let Err(error) = apple::initialize(fd_dir) {
        let _ = DEGRADED.set(format!(
            "could not lower the soft descriptor limit to {STARTUP_SOFT_LIMIT} \
             (reading {}): {error}; each child spawn sweeps up to the full soft limit",
            fd_dir.display()
        ));
    }
}

/// Why the startup limit was not applied, if it was not.
pub fn startup_degradation() -> Option<&'static str> {
    DEGRADED.get().map(String::as_str)
}

pub(crate) fn ceiling() -> Option<libc::c_int> {
    #[cfg(target_vendor = "apple")]
    return apple::STATE.get().map(|state| state.ceiling);
    #[cfg(not(target_vendor = "apple"))]
    None
}

/// Called after the child's CLOEXEC sweep. Only setrlimit, no locks or allocation.
pub(crate) fn restore() -> io::Result<()> {
    #[cfg(target_vendor = "apple")]
    if let Some(state) = apple::STATE.get() {
        // SAFETY: writes only the calling process's resource limit.
        if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &state.original) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(target_vendor = "apple")]
mod apple {
    use super::*;
    use std::sync::OnceLock;

    pub(super) struct State {
        pub(super) original: libc::rlimit,
        pub(super) ceiling: libc::c_int,
    }
    pub(super) static STATE: OnceLock<State> = OnceLock::new();

    pub(super) fn initialize(fd_dir: &std::path::Path) -> io::Result<()> {
        if STATE.get().is_some() {
            return Ok(());
        }
        let mut original = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: getrlimit writes the struct supplied here.
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut original) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut ceiling = 0;
        // Startup only, before other threads can allocate descriptors. This is
        // an upper bound, never an exact descriptor list used by the child.
        for entry in std::fs::read_dir(fd_dir)? {
            let name = entry?.file_name();
            if let Some(fd) = name
                .to_str()
                .and_then(|name| name.parse::<libc::c_int>().ok())
            {
                ceiling = ceiling.max(fd.saturating_add(1));
            }
        }
        let capped = libc::rlimit {
            rlim_cur: original.rlim_cur.min(STARTUP_SOFT_LIMIT as libc::rlim_t),
            ..original
        };
        ceiling = ceiling.max(capped.rlim_cur as libc::c_int);
        // SAFETY: the caller has guaranteed startup isolation. The hard limit stays intact.
        if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &capped) } != 0 {
            return Err(io::Error::last_os_error());
        }
        STATE.set(State { original, ceiling }).map_err(|_| {
            io::Error::other("descriptor startup must run before concurrent initialization")
        })
    }
}

#[cfg(all(test, target_vendor = "apple"))]
#[path = "process_nofile_tests.rs"]
mod tests;
