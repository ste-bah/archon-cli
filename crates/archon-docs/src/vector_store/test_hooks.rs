use std::sync::{Mutex, OnceLock, mpsc};

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
