use std::cell::Cell;
use std::sync::{Mutex, MutexGuard, OnceLock, mpsc};

thread_local! {
    static FAIL_NEXT_HNSW_PUBLICATION: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn clear_persisted_hnsw_cache() {
    super::persisted_hnsw::clear();
}

pub(crate) fn persisted_hnsw_load_count() -> usize {
    super::persisted_hnsw::load_count()
}

pub(crate) struct HnswStateGuard {
    _lock: MutexGuard<'static, ()>,
}

impl Drop for HnswStateGuard {
    fn drop(&mut self) {
        super::persisted_hnsw::clear();
    }
}

pub(crate) fn hnsw_state_guard() -> HnswStateGuard {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    super::persisted_hnsw::clear();
    HnswStateGuard { _lock: lock }
}

pub(crate) struct HnswPublicationFailureGuard;

impl Drop for HnswPublicationFailureGuard {
    fn drop(&mut self) {
        FAIL_NEXT_HNSW_PUBLICATION.set(false);
    }
}

struct SnapshotFenceHook {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

fn hook() -> &'static Mutex<Option<SnapshotFenceHook>> {
    static HOOK: OnceLock<Mutex<Option<SnapshotFenceHook>>> = OnceLock::new();
    HOOK.get_or_init(|| Mutex::new(None))
}

pub(super) fn install_snapshot_fence_hook() -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    *hook().lock().unwrap_or_else(|error| error.into_inner()) = Some(SnapshotFenceHook {
        entered: entered_sender,
        release: release_receiver,
    });
    (entered_receiver, release_sender)
}

pub(super) fn wait_at_snapshot_fence() {
    if let Some(hook) = hook()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
    {
        hook.entered.send(()).unwrap();
        hook.release.recv().unwrap();
    }
}

pub(super) fn clear_snapshot_fence_hook() {
    *hook().lock().unwrap_or_else(|error| error.into_inner()) = None;
}

pub(crate) fn fail_next_hnsw_publication() -> HnswPublicationFailureGuard {
    FAIL_NEXT_HNSW_PUBLICATION.set(true);
    HnswPublicationFailureGuard
}

pub(crate) fn should_fail_hnsw_publication() -> bool {
    FAIL_NEXT_HNSW_PUBLICATION.replace(false)
}

impl super::DocVectorStore {
    pub(crate) fn fail_next_hnsw_publication() -> HnswPublicationFailureGuard {
        fail_next_hnsw_publication()
    }
}
