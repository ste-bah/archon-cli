//! A resume runs under its stored context and, on top of it, the caller's
//! supervision; state that a stored object can change after spawn refuses it.
#[path = "support/boundary_harness.rs"]
mod harness;
#[path = "support/resume_memory_harness.rs"]
mod memory_harness;
use archon_tools::tool::ToolContext;
use harness::*;
use memory_harness::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[tokio::test]
async fn the_resume_callers_cancellation_still_stops_the_resumed_agent() {
    let (_t, root) = real_temp();
    let workspace = dir(&root, "workspace");
    let store = store(&root);
    let host = Host::new(&root, "memory-cancel", vec![STOP, STOP]);
    host.spawn("child", request(&workspace, None, vec![]), parent(&root, &[]))
        .await
        .unwrap();
    history(&store, "child");
    let plan = host.plan(&store, "child").await.unwrap();
    let interrupted = tokio_util::sync::CancellationToken::new();
    interrupted.cancel();
    let result = host
        .resume(
            "child",
            plan,
            ToolContext {
                cancel_parent: Some(interrupted),
                ..parent(&root, &[])
            },
        )
        .await;
    assert!(
        result.is_err(),
        "the caller's interrupt did not reach the resumed agent: {result:?}"
    );
}

/// Read-only while on, like the session's `/sandbox` toggle.
#[derive(Debug)]
struct Toggle(Arc<AtomicBool>);
impl archon_permissions::SandboxBackend for Toggle {
    fn check(
        &self,
        tool: &str,
        capability: archon_permissions::ToolCapability,
        _: &serde_json::Value,
    ) -> Result<(), String> {
        match capability {
            _ if !self.0.load(Ordering::SeqCst) => Ok(()),
            archon_permissions::ToolCapability::WorldBound(
                archon_permissions::WorldReach::FileRead,
            )
            | archon_permissions::ToolCapability::HostLocal => Ok(()),
            _ => Err(format!("sandbox: {tool} is blocked")),
        }
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
    fn live_state(&self) -> Option<String> {
        Some(format!("read-only={}", self.0.load(Ordering::SeqCst)))
    }
}

#[tokio::test]
async fn a_sandbox_switched_off_after_spawn_refuses_the_resume() {
    let (_t, root) = real_temp();
    let workspace = dir(&root, "workspace");
    let target = workspace.join("out.txt");
    let store = store(&root);
    let read_only = Arc::new(AtomicBool::new(true));
    let host = Host::new(
        &root,
        "memory-toggle",
        vec![write(&target, "first"), STOP, write(&target, "second"), STOP],
    );
    host.spawn(
        "child",
        request(&workspace, None, vec![]),
        ToolContext {
            sandbox: Some(Arc::new(Toggle(read_only.clone()))),
            ..parent(&root, &[])
        },
    )
    .await
    .unwrap();
    assert!(host.outcome(0).is_error, "the spawn was not read-only");
    history(&store, "child");
    read_only.store(false, Ordering::SeqCst);
    let plan = host.plan(&store, "child").await.unwrap();
    let result = host.resume("child", plan, parent(&root, &[])).await;
    assert!(!target.exists(), "the resumed agent wrote outside its sandbox");
    let refusal = result
        .expect_err("a resume with a weaker sandbox was accepted")
        .to_string();
    assert!(
        refusal.contains("'child'") && refusal.contains("sandbox"),
        "{refusal}"
    );
}
