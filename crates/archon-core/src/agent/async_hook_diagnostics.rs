use std::sync::Arc;

use super::Agent;

impl Agent {
    /// Set the hook registry for pre/post tool execution hooks.
    pub fn set_hook_registry(&mut self, registry: Arc<crate::hooks::HookRegistry>) {
        let event_tx = self.event_tx.clone();
        registry.set_async_hook_diagnostic_observer(Some(Arc::new(move |diagnostic| {
            let event_name = diagnostic.event.clone();
            let result = event_tx.try_send(crate::agent::TimestampedEvent {
                sent_at: std::time::Instant::now(),
                inner: crate::agent::AgentEvent::AsyncHookDiagnostic(diagnostic),
            });
            if let Err(error) = result {
                tracing::warn!(
                    event = %event_name,
                    error = %error,
                    "async hook diagnostic retained for session-end reporting because the agent event channel is full"
                );
            }
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

#[cfg(all(test, unix))]
#[path = "async_hook_diagnostics_tests.rs"]
mod tests;
