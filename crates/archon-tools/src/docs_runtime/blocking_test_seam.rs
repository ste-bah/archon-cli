use std::sync::{Mutex, OnceLock, mpsc};

use super::docs_runtime_tests::BlockingHookControl;

enum BlockingHook {
    Block {
        operation: String,
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    },
    Panic {
        operation: String,
    },
}

static HOOK: OnceLock<Mutex<Option<BlockingHook>>> = OnceLock::new();

pub(super) fn install_blocking_hook(operation: &str) -> BlockingHookControl {
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    *hook().lock().unwrap_or_else(|error| error.into_inner()) = Some(BlockingHook::Block {
        operation: operation.into(),
        entered: entered_tx,
        release: release_rx,
    });
    BlockingHookControl { entered, release }
}

pub(super) fn install_blocking_panic(operation: &str) {
    *hook().lock().unwrap_or_else(|error| error.into_inner()) = Some(BlockingHook::Panic {
        operation: operation.into(),
    });
}

pub(super) fn run_hook(operation: &str) {
    let mut hook = hook().lock().unwrap_or_else(|error| error.into_inner());
    match hook.take() {
        Some(BlockingHook::Block {
            operation: expected,
            entered,
            release,
        }) if expected == operation => {
            entered.send(()).unwrap();
            release.recv().unwrap();
        }
        Some(BlockingHook::Panic {
            operation: expected,
        }) if expected == operation => {
            panic!("test blocking task panic");
        }
        Some(other) => *hook = Some(other),
        None => {}
    }
}

fn hook() -> &'static Mutex<Option<BlockingHook>> {
    HOOK.get_or_init(|| Mutex::new(None))
}
