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
