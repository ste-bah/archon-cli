use std::sync::Arc;

use super::Agent;

impl Agent {
    /// Set the hook registry for pre/post tool execution hooks.
    pub fn set_hook_registry(&mut self, registry: Arc<crate::hooks::HookRegistry>) {
        let event_tx = self.event_tx.clone();
        registry.set_async_hook_diagnostic_observer(Some(Arc::new(move |diagnostic| {
            let _ = event_tx.try_send(crate::agent::TimestampedEvent {
                sent_at: std::time::Instant::now(),
                inner: crate::agent::AgentEvent::AsyncHookDiagnostic(diagnostic),
            });
        })));
        self.hook_registry = Some(registry);
    }

    pub fn drain_async_hook_diagnostics(&self) -> crate::hooks::AsyncHookDiagnosticBatch {
        self.hook_registry.as_ref().map_or(
            crate::hooks::AsyncHookDiagnosticBatch {
                diagnostics: Vec::new(),
                dropped: 0,
            },
            |registry| registry.drain_async_hook_diagnostics(),
        )
    }

    pub fn close_async_hook_diagnostics(&self) -> crate::hooks::AsyncHookDiagnosticBatch {
        self.hook_registry.as_ref().map_or(
            crate::hooks::AsyncHookDiagnosticBatch {
                diagnostics: Vec::new(),
                dropped: 0,
            },
            |registry| registry.close_async_hook_diagnostics(),
        )
    }
}
