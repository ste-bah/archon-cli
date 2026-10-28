//! Fixture-owned faults also reach host work dispatched on another thread.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, LazyLock, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Point {
    AfterCommit,
    CommitSync,
    Prepare,
    Seal,
    Archive,
    Link,
    Direct,
    Repository,
    ClientBuild,
    LaunchInput,
    ChainRead,
}
type Hook = Box<dyn FnOnce() -> std::io::Result<()> + Send>;
static HOOKS: LazyLock<Mutex<BTreeMap<(Point, PathBuf), Hook>>> = LazyLock::new(Default::default);

fn key(path: &Path) -> PathBuf {
    path.canonicalize()
        .or_else(|_| {
            Ok::<_, std::io::Error>(
                path.parent()
                    .unwrap()
                    .canonicalize()?
                    .join(path.file_name().unwrap()),
            )
        })
        .map(archon_shell::paths::plain)
        .unwrap_or_else(|_| path.to_owned())
}

pub(crate) struct Injection {
    point: Point,
    path: PathBuf,
    fired: Arc<AtomicBool>,
}
impl Injection {
    pub(crate) fn fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }
}
impl Drop for Injection {
    fn drop(&mut self) {
        HOOKS
            .lock()
            .unwrap()
            .remove(&(self.point, self.path.clone()));
    }
}

pub(crate) fn inject(point: Point, path: &Path, hook: Hook) -> Injection {
    let path = key(path);
    let fired = Arc::new(AtomicBool::new(false));
    let flag = fired.clone();
    HOOKS.lock().unwrap().insert(
        (point, path.clone()),
        Box::new(move || {
            flag.store(true, Ordering::SeqCst);
            hook()
        }),
    );
    Injection { point, path, fired }
}

pub(crate) fn fire(point: Point, path: &Path) -> std::io::Result<()> {
    let hook = HOOKS.lock().unwrap().remove(&(point, key(path)));
    hook.map_or(Ok(()), |hook| hook())
}
