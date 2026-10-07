//! Per-run hooks at the unlocked capture and receipt gaps, never in a lock.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
};
type Hook = Box<dyn FnOnce() + Send>;
static HOOKS: Mutex<BTreeMap<(PathBuf, &'static str), Hook>> = Mutex::new(BTreeMap::new());
pub(super) fn install(root: PathBuf, gap: &'static str, hook: Hook) {
    HOOKS.lock().unwrap().insert((root, gap), hook);
}
pub(super) fn run(root: &Path, gap: &'static str) {
    let hook = HOOKS.lock().unwrap().remove(&(root.to_path_buf(), gap));
    if let Some(hook) = hook {
        hook();
    }
}
