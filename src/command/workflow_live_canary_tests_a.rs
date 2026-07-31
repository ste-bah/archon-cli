#[path = "workflow_live_canary_usage_tests.rs"]
mod usage_tests;

const CANARY_TASK_ID: &str = "TASK-TDL-001";
const CANARY_ARTIFACT_REL: &str = ".archon/artifacts/TASK-TDL-001/gap-audit.md";

// tokio's Mutex, not std's: the guard is held for the whole of an async test
// (it serialises ARCHON_SCRIPT_LIFECYCLE mutation), and a std guard held across
// an await point is a deadlock risk clippy rightly rejects.
static LIFECYCLE_ENV_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

struct DecomposedLifecycleEnvGuard {
    previous: Option<String>,
}

impl DecomposedLifecycleEnvGuard {
    async fn set() -> (tokio::sync::MutexGuard<'static, ()>, Self) {
        let guard = LIFECYCLE_ENV_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        let previous = std::env::var("ARCHON_SCRIPT_LIFECYCLE").ok();
        unsafe {
            std::env::set_var("ARCHON_SCRIPT_LIFECYCLE", "0");
        }
        (guard, Self { previous })
    }
}

impl Drop for DecomposedLifecycleEnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var("ARCHON_SCRIPT_LIFECYCLE", value),
                None => std::env::remove_var("ARCHON_SCRIPT_LIFECYCLE"),
            }
        }
    }
}

/// Scripted stand-in for every agent role in the decomposed-PRD scaffold.
///
/// Responses are keyed on prompt content (the scaffold's `task:` strings), not
/// call order, so the client survives lifecycle refactors. Implementation and
/// remediation agents obey instructions literally: the artifact file is
/// written only when the prompt contains its path. Verification agents check
/// the filesystem like a real focused-verification agent would.
struct CanaryAgentClient {
    project_root: PathBuf,
    prompts: CanaryMutex<Vec<String>>,
}

