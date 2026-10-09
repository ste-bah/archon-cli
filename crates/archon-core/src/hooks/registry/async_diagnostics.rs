use super::HookRegistry;

impl HookRegistry {
    pub fn drain_async_hook_diagnostics(&self) -> super::super::AsyncHookDiagnosticBatch {
        self.async_diagnostics.drain()
    }

    pub fn close_async_hook_diagnostics(&self) -> super::super::AsyncHookDiagnosticBatch {
        self.async_diagnostics.close_and_drain()
    }

    pub fn set_async_hook_diagnostic_observer(
        &self,
        observer: Option<std::sync::Arc<dyn Fn(super::super::AsyncHookDiagnostic) + Send + Sync>>,
    ) {
        self.async_diagnostics.set_observer(observer);
    }
}
