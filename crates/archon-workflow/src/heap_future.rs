//! Build a large future on the heap from a small frame (#246).
//!
//! An `async` body that awaits `f(..)` keeps the child future inside its own
//! state, so every caller's future grows by the child's size. Boxing inline,
//! `Box::pin(f(..)).await`, keeps the state small but not the stack: the
//! child is first built in a stack slot of the poll function, and in a debug
//! build every such slot is a separate part of the poll frame. That frame is
//! on the stack for every poll of the whole call tree below it, so a few
//! large children per level overflow a 2 MiB thread stack in a deep chain.
//!
//! [`on_heap`] builds the child inside its own short frame, which is popped
//! before the first poll: the awaiting frame holds one pointer.
use std::{future::Future, pin::Pin};

/// A sendable future on the heap. The concrete type is erased, which also
/// ends the auto-trait (`Send`) proof at this boundary instead of nesting it
/// through every layer of the call tree.
pub type HeapFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The future `make` returns, built in this frame and moved to the heap.
#[inline(never)]
pub fn on_heap<'a, F>(make: impl FnOnce() -> F) -> HeapFuture<'a, F::Output>
where
    F: Future + Send + 'a,
{
    Box::pin(make())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_built_future_runs_to_its_output() {
        assert_eq!(on_heap(|| async { 7 }).await, 7);
    }

    #[tokio::test]
    async fn the_closure_runs_once_and_only_when_called() {
        let mut calls = 0;
        let future = on_heap(|| {
            calls += 1;
            async { "built" }
        });
        assert_eq!(calls, 1, "built once, before the first poll");
        assert_eq!(future.await, "built");
    }

    #[test]
    fn the_awaiting_side_holds_a_pointer_not_the_future() {
        let big = || async {
            let buffer = [0u8; 64 * 1024];
            std::future::ready(()).await;
            buffer.len()
        };
        assert!(std::mem::size_of_val(&big()) >= 64 * 1024);
        assert_eq!(
            std::mem::size_of_val(&on_heap(big)),
            2 * std::mem::size_of::<usize>(),
            "a pointer and its vtable"
        );
    }
}
