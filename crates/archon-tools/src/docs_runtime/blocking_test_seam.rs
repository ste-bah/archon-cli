use std::sync::{Mutex, OnceLock, mpsc};

use super::docs_runtime_tests::WaitHookControl;

enum WaitHook {
    Block {
        operation: String,
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    },
}

static WAIT_HOOK: OnceLock<Mutex<Option<WaitHook>>> = OnceLock::new();
static PANIC_HOOK: OnceLock<Mutex<Option<String>>> = OnceLock::new();

pub(super) fn install_docs_db_wait_hook(operation: &str) -> WaitHookControl {
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    *wait_hook()
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(WaitHook::Block {
        operation: operation.into(),
        entered: entered_tx,
        release: release_rx,
    });
    WaitHookControl { entered, release }
}

pub(super) fn install_blocking_panic(operation: &str) {
    *panic_hook()
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(operation.into());
}

pub(super) fn run_wait_hook(operation: &str) {
    let mut hook = wait_hook()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    match hook.take() {
        Some(WaitHook::Block {
            operation: expected,
            entered,
            release,
        }) if expected == operation => {
            let _ = entered.send(());
            let _ = release.recv_timeout(std::time::Duration::from_secs(1));
        }
        Some(other) => *hook = Some(other),
        None => {}
    }
}

pub(super) fn run_panic_hook(operation: &str) {
    let mut hook = panic_hook()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if hook.as_deref() == Some(operation) {
        hook.take();
        panic!("test blocking task panic");
    }
}

fn wait_hook() -> &'static Mutex<Option<WaitHook>> {
    WAIT_HOOK.get_or_init(|| Mutex::new(None))
}

fn panic_hook() -> &'static Mutex<Option<String>> {
    PANIC_HOOK.get_or_init(|| Mutex::new(None))
}
