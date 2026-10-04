//! A fresh registry per current-thread test, without resetting sibling tests.
use super::SecretValues;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

thread_local! {
    static SCOPED: RefCell<Option<SecretValues>> = const { RefCell::new(None) };
}

/// This guard must stay on its creating thread. Async tests using it must use
/// a current-thread runtime; tasks on that runtime share the scoped registry.
pub struct RegistryScope {
    previous: Option<SecretValues>,
    _thread: PhantomData<Rc<()>>,
}

/// Install an empty registry on this thread until the returned guard drops.
pub fn scoped_registry_for_tests() -> RegistryScope {
    let previous = SCOPED.with(|registry| registry.replace(Some(SecretValues::default())));
    RegistryScope {
        previous,
        _thread: PhantomData,
    }
}

impl Drop for RegistryScope {
    fn drop(&mut self) {
        SCOPED.with(|registry| registry.replace(self.previous.take()));
    }
}

pub(super) fn register(values: &SecretValues) -> bool {
    SCOPED.with(|registry| {
        if let Some(registry) = registry.borrow_mut().as_mut() {
            values.register_in(registry);
            true
        } else {
            false
        }
    })
}

pub(super) fn text(text: &str) -> Option<String> {
    SCOPED.with(|registry| {
        registry
            .borrow()
            .as_ref()
            .map(|registry| registry.text(text))
    })
}
