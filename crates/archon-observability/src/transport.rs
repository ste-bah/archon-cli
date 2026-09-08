//! Run-owned transport diagnostics, explicitly inherited across named tasks.
use std::{future::Future, fs::OpenOptions, io::Write, path::PathBuf, sync::{Arc, Mutex}};
use serde_json::{Value, json};

tokio::task_local! { static SCOPE: EvidenceScope; }

/// One host call's append-only evidence destination. Never serializes secrets.
#[derive(Clone)]
pub struct EvidenceScope {
    state: Arc<Mutex<State>>,
    call_id: String,
}
struct State { file: std::fs::File, error: Option<String> }
impl EvidenceScope {
    /// Open evidence before dispatch; an unwritable destination blocks dispatch.
    pub fn new(path: PathBuf, call_id: &str) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
        Ok(Self { state: Arc::new(Mutex::new(State { file: options.open(path)?, error: None })), call_id: call_id.into() })
    }
    /// Attach this destination for the lifetime of a future.
    pub async fn run<F: Future>(&self, future: F) -> F::Output { SCOPE.scope(self.clone(), future).await }
    /// Append already-redacted structured evidence; retain write errors for the host.
    pub fn record(&self, mut record: Value) {
        record["call_id"] = json!(self.call_id);
        record["recorded_at"] = json!(chrono::Utc::now().to_rfc3339());
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = writeln!(state.file, "{record}").and_then(|_| state.file.flush()) {
            state.error = Some(error.to_string());
        }
    }
    /// Fail the call if durable evidence could not be written.
    pub fn check(&self) -> std::io::Result<()> {
        match &self.state.lock().unwrap_or_else(|e| e.into_inner()).error {
            Some(e) => Err(std::io::Error::other(e.clone())), None => Ok(()),
        }
    }
}
/// Capture the current call's context before spawning a child task.
pub fn current() -> Option<EvidenceScope> { SCOPE.try_with(Clone::clone).ok() }
/// Preserve call attribution across the named-task boundary.
pub fn inherit<F: Future>(future: F) -> impl Future<Output = F::Output> {
    let scope = current();
    async move { match scope { Some(scope) => scope.run(future).await, None => future.await } }
}
