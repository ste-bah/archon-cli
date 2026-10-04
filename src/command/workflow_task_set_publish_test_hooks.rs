//! Thread-local observers keep fault tests independent of concurrent tests.
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};
/// A per-thread observer of each named publish step.
pub(crate) type StepHook = Option<Box<dyn Fn(&str)>>;
thread_local! {
    pub(crate) static STEP: RefCell<StepHook> = RefCell::new(None);
    pub(crate) static SYNCS: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}
pub(crate) fn step(value: &str) {
    STEP.with(|hook| {
        if let Some(hook) = hook.borrow().as_ref() {
            hook(value);
        }
    });
}
pub(crate) fn synced(path: &Path) {
    SYNCS.with(|paths| paths.borrow_mut().push(path.to_path_buf()));
}

/// Windows requires the named termination code as well as the step marker;
/// a panic after writing a marker still cannot impersonate this termination.
#[cfg(windows)]
pub(super) fn terminate() -> ! {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn TerminateProcess(process: *mut std::ffi::c_void, exit_code: u32) -> i32;
    }
    loop {
        // SAFETY: our own process pseudo-handle and an integer exit code have
        // no memory-safety preconditions. No Rust destructors run on this path.
        unsafe {
            TerminateProcess(GetCurrentProcess(), super::NAMED_CRASH_EXIT_CODE as u32);
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
