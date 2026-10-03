//! A resumed agent's sandbox, pinned to the state it was spawned under (#241).
//!
//! The stored context keeps the session's own backend object, and a session
//! toggle such as `/sandbox on/off` changes that object's decisions while a
//! run is in flight. Checking the state once, when the resume starts, leaves
//! the rest of the run to whatever the toggle says later. This wrapper asks
//! again around every decision and refuses each one taken while the state
//! differs from the recorded one, in either direction.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use archon_permissions::sandbox::{SandboxCommandRequest, SandboxCommandResult};
use archon_permissions::{
    SandboxBackend, SandboxScope, SandboxScopeSupport, SandboxTerminal, SandboxTerminalRequest,
    ToolCapability,
};

#[derive(Debug)]
pub(crate) struct PinnedSandbox {
    inner: Arc<dyn SandboxBackend>,
    recorded: Option<String>,
}

impl PinnedSandbox {
    pub(crate) fn new(inner: Arc<dyn SandboxBackend>, recorded: Option<String>) -> Self {
        Self { inner, recorded }
    }

    /// Why a decision taken now would not be the recorded sandbox's.
    fn drifted(&self) -> Option<String> {
        let now = self.inner.live_state();
        (now != self.recorded).then(|| {
            format!(
                "sandbox: this resumed agent may act only under the sandbox it was spawned under \
                 ({} then, {} now); restore it or start a new agent",
                self.recorded.as_deref().unwrap_or("fixed"),
                now.as_deref().unwrap_or("fixed"),
            )
        })
    }
}

impl SandboxBackend for PinnedSandbox {
    /// Asked before and after, so a toggle flipped while the inner backend
    /// decided cannot pass a decision taken under the other state.
    fn check(
        &self,
        tool: &str,
        capability: ToolCapability,
        input: &serde_json::Value,
    ) -> Result<(), String> {
        if let Some(reason) = self.drifted() {
            return Err(reason);
        }
        let decision = self.inner.check(tool, capability, input);
        match self.drifted() {
            Some(reason) => Err(reason),
            None => decision,
        }
    }

    fn terminal(&self, request: &SandboxTerminalRequest) -> SandboxTerminal {
        if let Some(reason) = self.drifted() {
            return SandboxTerminal::Refused(reason);
        }
        let decision = self.inner.terminal(request);
        match self.drifted() {
            Some(reason) => SandboxTerminal::Refused(reason),
            None => decision,
        }
    }

    fn scope_support(&self, scope: SandboxScope) -> SandboxScopeSupport {
        self.inner.scope_support(scope)
    }

    fn execute_bash<'a>(
        &'a self,
        request: SandboxCommandRequest,
    ) -> Pin<Box<dyn Future<Output = Option<SandboxCommandResult>> + Send + 'a>> {
        match self.drifted() {
            Some(reason) => Box::pin(async move {
                Some(SandboxCommandResult {
                    content: reason,
                    is_error: true,
                    exit_code: None,
                })
            }),
            None => self.inner.execute_bash(request),
        }
    }

    fn live_state(&self) -> Option<String> {
        self.inner.live_state()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Debug)]
    struct Flag(Arc<AtomicBool>);
    impl SandboxBackend for Flag {
        fn check(&self, _: &str, _: ToolCapability, _: &serde_json::Value) -> Result<(), String> {
            Ok(())
        }
        fn terminal(&self, _: &SandboxTerminalRequest) -> SandboxTerminal {
            SandboxTerminal::Host
        }
        fn scope_support(&self, _: SandboxScope) -> SandboxScopeSupport {
            SandboxScopeSupport::Durable
        }
        fn live_state(&self) -> Option<String> {
            Some(self.0.load(Ordering::SeqCst).to_string())
        }
    }

    #[tokio::test]
    async fn every_decision_refuses_while_the_state_differs_from_the_recorded_one() {
        let flag = Arc::new(AtomicBool::new(true));
        let pinned = PinnedSandbox::new(Arc::new(Flag(flag.clone())), Some("true".into()));
        let input = serde_json::json!({});
        assert!(pinned.check("Write", ToolCapability::HostLocal, &input).is_ok());
        flag.store(false, Ordering::SeqCst);
        assert!(pinned.check("Write", ToolCapability::HostLocal, &input).is_err());
        let request = SandboxTerminalRequest {
            shell: None,
            workspace: "/".into(),
            cwd: "/".into(),
        };
        assert!(matches!(pinned.terminal(&request), SandboxTerminal::Refused(_)));
        let bash = pinned.execute_bash(SandboxCommandRequest::default()).await;
        assert!(bash.is_some_and(|result| result.is_error));
        flag.store(true, Ordering::SeqCst);
        assert!(pinned.check("Write", ToolCapability::HostLocal, &input).is_ok());
    }
}
