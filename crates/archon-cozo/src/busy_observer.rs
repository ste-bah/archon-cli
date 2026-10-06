//! Thread-local test seam: observe an actual classified busy result before retry/return.
use std::cell::RefCell;

type Observer = Box<dyn FnMut(&str, &str)>;
thread_local! { static OBSERVER: RefCell<Option<Observer>> = RefCell::new(None); }

pub fn with_busy_observer<T>(
    observer: impl FnMut(&str, &str) + 'static,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<Observer>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OBSERVER.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let _restore = Restore(OBSERVER.with(|slot| slot.replace(Some(Box::new(observer)))));
    run()
}

pub(crate) fn notify(context: &str, error: &str) {
    OBSERVER.with(|slot| {
        if let Some(observer) = slot.borrow_mut().as_mut() {
            observer(context, error);
        }
    });
}
