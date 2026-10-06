//! Bound Apple descriptor sweeps before the runtime creates threads.
use std::io;

/// Lower the parent's soft NOFILE limit, retaining the original for children.
/// Other platforms use atomic CLOEXEC creation and need no startup cap.
///
/// # Safety
/// Call before creating threads or descriptors concurrently. Lowering a limit
/// does not close inherited high descriptors; their upper bound is captured first.
pub unsafe fn initialize() -> io::Result<()> {
    #[cfg(target_vendor = "apple")]
    apple::initialize()?;
    Ok(())
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

    pub(super) fn initialize() -> io::Result<()> {
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
        for entry in std::fs::read_dir("/dev/fd")? {
            let name = entry?.file_name();
            if let Some(fd) = name
                .to_str()
                .and_then(|name| name.parse::<libc::c_int>().ok())
            {
                ceiling = ceiling.max(fd.saturating_add(1));
            }
        }
        let capped = libc::rlimit {
            rlim_cur: original.rlim_cur.min(4_096),
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
