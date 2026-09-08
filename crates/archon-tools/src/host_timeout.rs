//! Host-owned timeout override; never parsed from an agent tool request.
#[derive(Clone, Copy, Debug)]
pub enum HostTimeout { Finite(u64), Unlimited }

tokio::task_local! { static OVERRIDE: HostTimeout; }

pub fn current() -> Option<HostTimeout> { OVERRIDE.try_with(|value| *value).ok() }

pub async fn scope<T>(limit: HostTimeout, work: impl std::future::Future<Output = T>) -> T {
    OVERRIDE.scope(limit, work).await
}

/// Task locals do not cross spawn automatically. Capture on the caller and
/// explicitly restore around the spawned executor's work.
pub async fn inherit<T>(limit: Option<HostTimeout>, work: impl std::future::Future<Output = T>) -> T {
    match limit { Some(limit) => scope(limit, work).await, None => work.await }
}
