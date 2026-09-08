//! Host-owned timeout override; never parsed from an agent tool request.
#[derive(Clone, Copy, Debug)]
pub enum HostTimeout { Finite(u64), Unlimited }

pub async fn scope<T>(_: HostTimeout, work: impl std::future::Future<Output = T>) -> T {
    work.await
}
