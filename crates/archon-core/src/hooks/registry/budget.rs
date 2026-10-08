//! Compatibility for the former aggregate timeout key.
use crate::hooks::types::HookConfig;

/// Supply a fresh fallback idle window; never shorten an explicit timeout.
pub(crate) fn with_fallback_timeout(hook: &HookConfig, fallback_ms: u64) -> HookConfig {
    let mut configured = hook.clone();
    if configured.timeout.is_none() {
        configured.timeout = Some(fallback_ms.div_ceil(1000).min(u64::from(u32::MAX)) as u32);
    }
    configured
}
