//! One container per sandbox lifetime, re-entered with `docker exec`.
//!
//! `execute_bash` used to build and destroy a container per command. Measured on
//! the machine this was written on: 214ms for `docker run --rm` against 57ms for
//! `docker exec`. The latency is the small half. The bind mount covers only the
//! workspace, so everything a build leaves *outside* it — `~/.cargo/registry`,
//! `~/.npm`, pip wheels, apt lists, `/tmp` — went with the container. A
//! sandboxed `cargo build` re-downloaded its dependency graph on every call, and
//! would have gone on doing so forever.
//!
//! What is held is decided by [`SandboxScope`], and it is keyed by working
//! directory as well. That is not an optimisation: a worktree-isolated subagent
//! mounts a different tree, and one container shared across two trees would put
//! two agents in a single world while each believed it was isolated.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use archon_permissions::sandbox::{SandboxCommandRequest, SandboxScope};
use tokio::process::Command as TokioCommand;

use super::DockerConfig;
use super::exec::{ContainerKind, docker_exec_args, docker_pool_create_args};

/// Labels every container Archon creates carries, and the only handle teardown
/// has on a container whose creator is gone.
pub(super) const OWNED_LABEL: &str = "archon.sandbox";
pub(super) const OWNER_LABEL: &str = "archon.sandbox.owner";
pub(super) const PID_LABEL: &str = "archon.sandbox.pid";
const KIND_LABEL: &str = "archon.sandbox.kind";

/// The labels that make a container findable. Applied by `isolation_args`, so
/// every container Archon starts carries them — held, per-command and terminal
/// alike. A container without them can be neither reaped nor listed by the
/// `docker ps --filter label=archon.sandbox=1` the docs hand operators, which
/// makes it a leak nothing can even see.
pub(super) fn archon_labels(kind: ContainerKind) -> Vec<(&'static str, String)> {
    vec![
        (OWNED_LABEL, "1".to_string()),
        (OWNER_LABEL, owner_id().to_string()),
        (PID_LABEL, std::process::id().to_string()),
        (KIND_LABEL, kind.as_str().to_string()),
    ]
}

/// Identity of *this* Archon process, for telling our containers from those of
/// another Archon running concurrently — which is the normal case here, not an
/// edge one. Reaping that keyed on "not mine" alone would have two sessions
/// destroying each other's sandboxes.
pub(super) fn owner_id() -> &'static str {
    static OWNER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    OWNER.get_or_init(|| uuid::Uuid::new_v4().simple().to_string()[..12].to_string())
}

/// What a held container is keyed by.
///
/// `working_dir` is a whole `PathBuf` compared by equality rather than a hash
/// folded into the container name, so no digest collision can ever hand one
/// tree's container to another tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LifetimeKey {
    session_id: String,
    /// The instance of the scope: the session for `session`, the turn for
    /// `turn`. `tool` never reaches here.
    instance: String,
    working_dir: PathBuf,
}

#[derive(Debug, Clone)]
struct Held {
    name: String,
    /// Commands currently executing in this container.
    ///
    /// The turn boundary tears containers down with `docker rm --force`, and
    /// force-removing a container with a command still inside it kills that
    /// command — the model gets a bare `Exit code 137` for a container Archon
    /// destroyed underneath it. "Turns are sequential" is true of one agent's
    /// own turns and nothing enforces it across a tree: a subagent inherits its
    /// parent's turn id and may still be running when the parent's next turn
    /// begins. So eviction asks rather than assumes.
    in_flight: Arc<AtomicUsize>,
}

impl Held {
    /// Claim this container for one command. Taken under the pool lock, so a
    /// container can never become a candidate for eviction between the decision
    /// to use it and the count that protects it.
    fn lease(&self) -> ContainerLease {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        ContainerLease {
            name: self.name.clone(),
            in_flight: Arc::clone(&self.in_flight),
        }
    }
}

/// A command's claim on a held container, held for exactly as long as the
/// command runs.
///
/// The count is incremented under the pool lock, before the container can be a
/// candidate for eviction, and decremented by `Drop` — so it is right even when
/// the command panics or is cancelled.
pub(super) struct ContainerLease {
    name: String,
    in_flight: Arc<AtomicUsize>,
}

impl ContainerLease {
    pub(super) fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for ContainerLease {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The containers this process is holding open.
#[derive(Debug)]
pub(super) struct ContainerPool {
    binary: String,
    image: String,
    scope: SandboxScope,
    workspace_access: String,
    config: DockerConfig,
    live: tokio::sync::Mutex<HashMap<LifetimeKey, Held>>,
    reaped: tokio::sync::OnceCell<()>,
}

static NAME_COUNTER: AtomicU64 = AtomicU64::new(0);

impl ContainerPool {
    pub(super) fn new(config: DockerConfig, workspace_access: String, scope: SandboxScope) -> Self {
        Self {
            binary: config.binary.clone(),
            image: config.image.clone(),
            scope,
            workspace_access,
            config,
            live: tokio::sync::Mutex::new(HashMap::new()),
            reaped: tokio::sync::OnceCell::new(),
        }
    }

    /// The lifetime this request belongs to, or `None` when nothing is held.
    ///
    /// `turn` scope with no turn identity resolves to `None` on purpose. A
    /// caller that cannot name its turn has no turn identity to share, and
    /// treating every such caller's `None` as one identity would collapse
    /// unrelated agents into a single container. Per-command is the only
    /// answer that is both safe and true.
    fn key(&self, request: &SandboxCommandRequest) -> Option<LifetimeKey> {
        let instance = match self.scope {
            SandboxScope::Tool => return None,
            SandboxScope::Session => request.session_id.clone(),
            SandboxScope::Turn => request.turn_id.clone()?,
        };
        Some(LifetimeKey {
            session_id: request.session_id.clone(),
            instance,
            working_dir: request.working_dir.clone(),
        })
    }

    /// A lease on the held container for this request.
    ///
    /// `Ok(None)` means this request has no lifetime to hold and the caller
    /// should fall back to the per-command `docker run`.
    pub(super) async fn container_for(
        &self,
        request: &SandboxCommandRequest,
    ) -> Result<Option<ContainerLease>, String> {
        // Before the early return, not after it. Reaping used to sit below the
        // `key()` bail-out, which meant it never ran under `scope = "tool"` —
        // where `key()` is always `None` — nor for a `turn`-scoped caller with
        // no turn id, which is what the TUI's pipeline adapter is. The two
        // configurations that create the *most* uncollectable containers were
        // exactly the two that never collected any.
        self.reaped
            .get_or_init(|| super::reap::reap_orphans(self.binary.clone()))
            .await;
        let Some(key) = self.key(request) else {
            return Ok(None);
        };
        // Held across the `docker run` below, so two commands racing for one
        // key cannot each start a container and leave one of them orphaned with
        // nothing holding its name. The cost is that a concurrent command for a
        // *different* key waits out that creation — a few hundred milliseconds,
        // once per lifetime, against a correctness property.
        let mut live = self.live.lock().await;
        if self.scope == SandboxScope::Turn {
            self.evict_finished_turns(&mut live, &key).await;
        }
        if let Some(held) = live.get(&key) {
            return Ok(Some(held.lease()));
        }
        let name = self.create(&key).await?;
        let held = Held {
            name,
            in_flight: Arc::new(AtomicUsize::new(0)),
        };
        let lease = held.lease();
        live.insert(key, held);
        Ok(Some(lease))
    }

    /// End the turns this session has moved past, sparing any container with a
    /// command still inside it.
    ///
    /// Turns are sequential within *one agent's* own turns, and nothing enforces
    /// that across an agent tree: a subagent inherits its parent's turn id and
    /// may still be running when the parent's next turn begins. Force-removing
    /// its container would kill its command and report a bare `Exit code 137`
    /// for a container Archon destroyed itself. A busy container is therefore
    /// left alone and reconsidered at the next turn boundary; if none comes,
    /// `Drop` and the container's own age bound still end it.
    async fn evict_finished_turns(
        &self,
        live: &mut HashMap<LifetimeKey, Held>,
        current: &LifetimeKey,
    ) {
        for key in finished_turns(live.keys(), current) {
            let Some(held) = live.get(&key) else {
                continue;
            };
            if held.in_flight.load(Ordering::SeqCst) > 0 {
                tracing::debug!(
                    container = %held.name,
                    "sandbox: a command is still running in the previous turn's \
                     container; deferring teardown rather than killing it"
                );
                continue;
            }
            if let Some(held) = live.remove(&key) {
                self.destroy(&held.name).await;
            }
        }
    }

    async fn create(&self, key: &LifetimeKey) -> Result<String, String> {
        let name = format!(
            "archon-sbx-{}-{}",
            owner_id(),
            NAME_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let args = docker_pool_create_args(
            &self.config,
            &self.workspace_access,
            &key.working_dir,
            &name,
            self.config.container_max_age_secs,
        );
        let output = TokioCommand::new(&self.binary)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|error| format!("failed to spawn docker: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "could not start a {} sandbox container from {}: {}",
                self.scope,
                self.image,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(name)
    }

    /// Build the `docker exec` for a command in a held container.
    pub(super) fn exec_args(&self, name: &str, request: &SandboxCommandRequest) -> Vec<String> {
        docker_exec_args(&self.config, name, request)
    }

    /// Forget a container that is no longer running, so the next command
    /// rebuilds it.
    ///
    /// Authoritative: the daemon is asked, never the shape of an error string.
    /// A container can vanish under us for reasons that are nobody's bug — the
    /// `sleep` that is its PID 1 reaches `container_max_age_secs`, an operator
    /// runs `docker rm`, the daemon restarts.
    pub(super) async fn forget_if_gone(&self, request: &SandboxCommandRequest, name: &str) -> bool {
        if self.is_running(name).await {
            return false;
        }
        let Some(key) = self.key(request) else {
            return false;
        };
        let mut live = self.live.lock().await;
        if live.get(&key).is_some_and(|held| held.name == name) {
            live.remove(&key);
        }
        true
    }

    async fn is_running(&self, name: &str) -> bool {
        TokioCommand::new(&self.binary)
            .args(["inspect", "-f", "{{.State.Running}}", name])
            .stdin(Stdio::null())
            .output()
            .await
            .is_ok_and(|output| {
                output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
            })
    }

    async fn destroy(&self, name: &str) {
        let _ = TokioCommand::new(&self.binary)
            .args(["rm", "--force", name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .await;
    }
}

/// The session-scope boundary, and the only teardown this process can run for
/// itself.
///
/// Best-effort by construction and described as nothing more. It does not run
/// when the process is SIGKILLed, panics through an abort, or calls
/// `std::process::exit` — and it does not run when the last `Arc` is held by
/// something that outlives the process either, which the workflow CLI's
/// process-global subagent executor is. That is why it is the third of three
/// mechanisms rather than the only one: `container_max_age_secs` bounds the leak
/// with no host involvement at all, and startup reaping closes it the moment any
/// Archon runs again.
impl Drop for ContainerPool {
    fn drop(&mut self) {
        // `get_mut` rather than a lock: `Drop` holds `&mut self`, so no other
        // reference exists and blocking a runtime thread on an async mutex here
        // would be both unnecessary and deadlock-prone.
        let names: Vec<String> = self
            .live
            .get_mut()
            .drain()
            .map(|(_, held)| held.name)
            .collect();
        for name in names {
            let _ = std::process::Command::new(&self.binary)
                .args(["rm", "--force", &name])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

/// Which held lifetimes a request under `turn` scope ends.
///
/// Pure, and split out from the teardown that acts on it, because the filter is
/// the whole safety argument: it must be restricted to the *same session*. A
/// concurrent session's turns are not ordered against this one's, so evicting on
/// a turn-id mismatch alone would destroy another session's container while a
/// command was running in it.
fn finished_turns<'a>(
    live: impl Iterator<Item = &'a LifetimeKey>,
    current: &LifetimeKey,
) -> Vec<LifetimeKey> {
    live.filter(|key| key.session_id == current.session_id && key.instance != current.instance)
        .cloned()
        .collect()
}

/// How long a held container may live without anyone tearing it down.
pub(super) const DEFAULT_MAX_AGE_SECS: u64 = 4 * 60 * 60;

/// A bound on the timeout a held container's `sleep` must outlast.
pub(super) fn max_age_is_sane(secs: u64) -> Result<(), String> {
    if secs < 60 {
        return Err(format!(
            "sandbox.docker.container_max_age_secs must be at least 60, got {secs}; \
             a shorter bound would destroy containers mid-command"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "pool_tests.rs"]
mod tests;
