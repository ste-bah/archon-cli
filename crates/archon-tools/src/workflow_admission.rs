//! Captured host admission, carried through spawned agent/tool contexts.
use std::{future::Future, sync::Arc, task::Poll};

type Check = dyn Fn(&mut dyn FnMut()) -> Result<(), String> + Send + Sync;
#[derive(Clone)]
pub struct AdmissionFence(Arc<Check>);
impl std::fmt::Debug for AdmissionFence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AdmissionFence(..)")
    }
}
impl AdmissionFence {
    /// The host checks its captured owner and invokes `work` under its lock.
    pub fn new(
        check: impl Fn(&mut dyn FnMut()) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(check))
    }
    pub async fn execute<T>(&self, work: impl Future<Output = T>) -> Result<T, String> {
        let mut work = Box::pin(work);
        std::future::poll_fn(|cx| {
            let mut polled = None;
            match (self.0)(&mut || polled = Some(work.as_mut().poll(cx))) {
                Ok(()) => match polled {
                    Some(Poll::Ready(value)) => Poll::Ready(Ok(value)),
                    Some(Poll::Pending) => Poll::Pending,
                    None => Poll::Ready(Err("admission fence did not admit its operation".into())),
                },
                Err(reason) => Poll::Ready(Err(reason)),
            }
        })
        .await
    }
}
