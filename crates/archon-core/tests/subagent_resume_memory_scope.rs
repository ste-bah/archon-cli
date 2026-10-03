//! A repair runs under its stored context and, on top of it, the caller's
//! supervision. It never shares a sandbox that can change: it gets a frozen
//! copy of the sandbox as it was at spawn, or it is refused.
#[path = "support/boundary_harness.rs"]
mod harness;
#[path = "support/resume_memory_harness.rs"]
mod memory_harness;
use archon_permissions::{SandboxBackend, SandboxSnapshot, ToolCapability, WorldReach};
use archon_tools::tool::ToolContext;
use harness::*;
use memory_harness::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[tokio::test]
async fn the_repair_callers_interrupt_still_stops_the_repaired_agent() {
    let (_t, root) = temp();
    let workspace = dir(&root, "workspace");
    let host = Host::new(&root, "memory-cancel", vec![STOP, STOP]);
    let spawn = request(&workspace, None, vec![]);
    host.spawn("child", spawn.clone(), parent(&root, &[]))
        .await
        .unwrap();
    let interrupted = tokio_util::sync::CancellationToken::new();
    interrupted.cancel();
    let result = host
        .repair(
            "child",
            spawn,
            ToolContext {
                cancel_parent: Some(interrupted),
                ..parent(&root, &[])
            },
        )
        .await;
    assert!(
        result.is_err(),
        "the caller's interrupt did not reach the repaired agent: {result:?}"
    );
}

/// Read-only while on, like the session's `/sandbox` toggle.
fn decide(read_only: &AtomicBool, tool: &str, capability: ToolCapability) -> Result<(), String> {
    match capability {
        _ if !read_only.load(Ordering::SeqCst) => Ok(()),
        ToolCapability::WorldBound(WorldReach::FileRead) | ToolCapability::HostLocal => Ok(()),
        _ => Err(format!("sandbox: {tool} is blocked")),
    }
}

/// A toggle that can give a frozen copy of itself.
#[derive(Debug)]
struct Toggle(Arc<AtomicBool>);
/// A toggle that cannot.
#[derive(Debug)]
struct Unfreezable(Arc<AtomicBool>);

macro_rules! toggle_backend {
    ($name:ident) => {
        impl SandboxBackend for $name {
            fn check(
                &self,
                tool: &str,
                capability: ToolCapability,
                _: &serde_json::Value,
            ) -> Result<(), String> {
                decide(&self.0, tool, capability)
            }
            fn terminal(
                &self,
                _: &archon_permissions::SandboxTerminalRequest,
            ) -> archon_permissions::SandboxTerminal {
                archon_permissions::SandboxTerminal::Host
            }
            fn scope_support(
                &self,
                _: archon_permissions::SandboxScope,
            ) -> archon_permissions::SandboxScopeSupport {
                archon_permissions::SandboxScopeSupport::Durable
            }
            fn snapshot(&self) -> SandboxSnapshot {
                $name::snapshot(self)
            }
        }
    };
}
impl Toggle {
    fn snapshot(&self) -> SandboxSnapshot {
        SandboxSnapshot::Frozen(Arc::new(Toggle(Arc::new(AtomicBool::new(
            self.0.load(Ordering::SeqCst),
        )))))
    }
}
impl Unfreezable {
    fn snapshot(&self) -> SandboxSnapshot {
        SandboxSnapshot::Unavailable
    }
}
toggle_backend!(Toggle);
toggle_backend!(Unfreezable);

#[tokio::test]
async fn a_repair_keeps_the_sandbox_it_was_spawned_under_after_it_is_switched_off() {
    let (_t, root) = temp();
    let workspace = dir(&root, "workspace");
    let target = workspace.join("out.txt");
    let read_only = Arc::new(AtomicBool::new(true));
    let live: Arc<dyn SandboxBackend> = Arc::new(Toggle(read_only.clone()));
    let host = Host::new(
        &root,
        "memory-toggle",
        vec![
            write(&target, "first"),
            STOP,
            ("ContextProbe", serde_json::json!({})),
            write(&target, "second"),
            STOP,
        ],
    );
    let mut spawn = request(&workspace, None, vec![]);
    spawn.allowed_tools.push("ContextProbe".into());
    let sandboxed = ToolContext {
        sandbox: Some(live.clone()),
        ..parent(&root, &[])
    };
    host.spawn("child", spawn.clone(), sandboxed.clone())
        .await
        .unwrap();
    assert!(host.outcome(0).is_error, "the spawn was not read-only");
    read_only.store(false, Ordering::SeqCst);
    host.repair("child", spawn, sandboxed).await.unwrap();
    assert!(host.outcome(3).is_error, "{:?}", host.outcome(3));
    assert!(
        !target.exists(),
        "the repaired agent wrote outside its sandbox"
    );
    let contexts = host.contexts.lock().unwrap();
    let held = contexts[0]
        .sandbox
        .as_ref()
        .expect("the repair had no sandbox");
    assert!(
        !Arc::ptr_eq(held, &live),
        "the repair shares the live toggle, so a later flip would change it"
    );
}

#[tokio::test]
async fn a_repair_whose_sandbox_cannot_be_frozen_is_refused() {
    let (_t, root) = temp();
    let workspace = dir(&root, "workspace");
    let host = Host::new(&root, "memory-unfreezable", vec![STOP, STOP]);
    let spawn = request(&workspace, None, vec![]);
    let sandboxed = ToolContext {
        sandbox: Some(Arc::new(Unfreezable(Arc::new(AtomicBool::new(true))))),
        ..parent(&root, &[])
    };
    host.spawn("child", spawn.clone(), sandboxed.clone())
        .await
        .unwrap();
    let refusal = host
        .repair("child", spawn, sandboxed)
        .await
        .expect_err("a repair shared a sandbox that can change")
        .to_string();
    assert!(
        refusal.contains("'child'") && refusal.contains("cannot be frozen"),
        "{refusal}"
    );
    assert_eq!(host.turns(), 1, "the refused repair still ran");
}
