use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;

use anyhow::{Result, anyhow};

static COZO_PANIC_HOOK: OnceLock<()> = OnceLock::new();

thread_local! {
    static IN_GUARDED_COZO_OPERATION: Cell<usize> = const { Cell::new(0) };
}

pub(crate) fn catch_guarded_operation<T>(
    context: &str,
    run: &mut impl FnMut() -> Result<T>,
) -> Result<T> {
    install_cozo_panic_hook();
    let _guard = GuardedCozoOperation::enter();
    let result = catch_unwind(AssertUnwindSafe(run));

    match result {
        Ok(result) => result,
        Err(payload) => Err(anyhow!(
            "{context}: Cozo operation panicked: {}",
            panic_payload_message(payload)
        )),
    }
}

pub(crate) fn in_guarded_operation() -> bool {
    IN_GUARDED_COZO_OPERATION.with(|depth| depth.get() > 0)
}

fn install_cozo_panic_hook() {
    COZO_PANIC_HOOK.get_or_init(|| {
        let delegate = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            if !in_guarded_operation() {
                delegate(panic_info);
            }
        }));
    });
}

struct GuardedCozoOperation;

impl GuardedCozoOperation {
    fn enter() -> Self {
        IN_GUARDED_COZO_OPERATION.with(|depth| depth.set(depth.get() + 1));
        Self
    }
}

impl Drop for GuardedCozoOperation {
    fn drop(&mut self) {
        IN_GUARDED_COZO_OPERATION.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

fn panic_payload_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else {
        "unknown panic payload".to_owned()
    }
}
