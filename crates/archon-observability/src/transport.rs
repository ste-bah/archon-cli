//! Run-owned transport diagnostics, explicitly inherited across named tasks.
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    future::Future,
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
};

tokio::task_local! { static SCOPE: EvidenceScope; }
static APPEND_LOCK: Mutex<()> = Mutex::new(());

/// One host call's append-only evidence destination. Never serializes secrets.
#[derive(Clone)]
pub struct EvidenceScope {
    state: Arc<Mutex<State>>,
    call_id: String,
}
struct State {
    file: std::fs::File,
    error: Option<String>,
    bytes: u64,
    capped: bool,
}

/// Stop appending past this much evidence. Diagnostics must not become the
/// reason a long run fills the volume it is diagnosing.
const MAX_EVIDENCE_BYTES: u64 = 64 * 1024 * 1024;
impl EvidenceScope {
    /// Open evidence before dispatch; an unwritable destination blocks dispatch.
    pub fn new(path: PathBuf, call_id: &str) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        let bytes = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                file,
                error: None,
                bytes,
                capped: false,
            })),
            call_id: call_id.into(),
        })
    }
    /// Attach this destination for the lifetime of a future.
    pub async fn run<F: Future>(&self, future: F) -> F::Output {
        SCOPE.scope(self.clone(), future).await
    }
    /// Append already-redacted structured evidence; retain write errors for the host.
    pub fn record(&self, mut record: Value) {
        record["call_id"] = json!(self.call_id);
        record["recorded_at"] = json!(chrono::Utc::now().to_rfc3339());
        let line = format!("{record}\n");
        let _append = APPEND_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.capped {
            return;
        }
        let line = if state.bytes.saturating_add(line.len() as u64) > MAX_EVIDENCE_BYTES {
            state.capped = true;
            format!(
                "{}\n",
                json!({"kind":"evidence_capped","call_id":self.call_id,
                "recorded_at":chrono::Utc::now().to_rfc3339(),"limit_bytes":MAX_EVIDENCE_BYTES})
            )
        } else {
            line
        };
        state.bytes = state.bytes.saturating_add(line.len() as u64);
        // Deliberately no fsync here. `record` runs on a tokio worker — the
        // capture's Drop sits in the async stream path — and every response
        // writes twice, so syncing per record put a blocking disk wait under a
        // process-global lock on the hot path of the very stalls this evidence
        // exists to explain. The case that matters still survives: a killed or
        // panicking run is flushed by the kernel on process exit. `check`
        // forces durability at the end of the call for anything harder.
        if let Err(error) = state.file.write_all(line.as_bytes()) {
            state.error = Some(error.to_string());
        }
    }
    /// Fail the call if durable evidence could not be written.
    pub fn check(&self) -> std::io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = state.file.sync_data() {
            state.error.get_or_insert(error.to_string());
        }
        match &state.error {
            Some(e) => Err(std::io::Error::other(e.clone())),
            None => Ok(()),
        }
    }
}
/// Capture the current call's context before spawning a child task.
pub fn current() -> Option<EvidenceScope> {
    SCOPE.try_with(Clone::clone).ok()
}
/// Preserve call attribution across the named-task boundary.
pub fn inherit<F: Future>(future: F) -> impl Future<Output = F::Output> {
    let scope = current();
    async move {
        match scope {
            Some(scope) => scope.run(future).await,
            None => future.await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_calls_keep_complete_jsonl_records() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("transport.jsonl");
        std::thread::scope(|threads| {
            let barrier = Arc::new(std::sync::Barrier::new(8));
            for n in 0..8 {
                let scope = EvidenceScope::new(path.clone(), &n.to_string()).unwrap();
                let barrier = barrier.clone();
                threads.spawn(move || {
                    barrier.wait();
                    for _ in 0..20 {
                        scope.record(json!({"kind":"test","sample":"x".repeat(1000)}));
                    }
                    scope.check().unwrap();
                });
            }
        });
        let raw = std::fs::read_to_string(path).unwrap();
        assert_eq!(raw.lines().count(), 160);
        for line in raw.lines() {
            serde_json::from_str::<Value>(line).expect("parallel calls corrupted JSONL");
        }
    }

    #[test]
    fn evidence_stops_growing_at_the_cap_and_says_so_once() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("transport.jsonl");
        let scope = EvidenceScope::new(path.clone(), "call").unwrap();
        // One megabyte per record reaches the cap without writing 64 MiB of
        // real samples, which is the point: the guard is on bytes, not records.
        let sample = "x".repeat(1_000_000);
        for _ in 0..80 {
            scope.record(json!({"kind":"test","sample":sample}));
        }
        scope.check().unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            raw.len() as u64 <= MAX_EVIDENCE_BYTES + 1_000_200,
            "cap overshot: {}",
            raw.len()
        );
        assert_eq!(
            raw.lines()
                .filter(|l| l.contains("evidence_capped"))
                .count(),
            1
        );
        assert!(raw.lines().last().unwrap().contains("evidence_capped"));
        for line in raw.lines() {
            serde_json::from_str::<Value>(line).expect("cap corrupted JSONL");
        }
    }
}
