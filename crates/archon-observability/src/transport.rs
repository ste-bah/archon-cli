//! Run-owned transport evidence scope.
use std::path::PathBuf;
/// Context for one workflow call's transport evidence.
pub struct EvidenceScope;
impl EvidenceScope {
    /// Prepare the run-owned evidence destination.
    pub fn new(_path: PathBuf, _call_id: &str) -> std::io::Result<Self> { Ok(Self) }
    /// Execute a call with its evidence destination attached.
    pub async fn run<F: std::future::Future>(&self, future: F) -> F::Output { future.await }
}
