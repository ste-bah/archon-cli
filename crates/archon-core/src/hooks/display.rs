//! How a hook is named wherever it reaches a log line or a failure reason.

use super::types::{HookCommandType, HookConfig};
use crate::url_redact::redact_url;

impl HookConfig {
    /// The hook's command as it may be shown to people and models.
    ///
    /// An HTTP hook's `command` is its URL, which commonly embeds the
    /// webhook credential (userinfo, `?token=`, or the path itself), so only
    /// its origin is shown. Other hook kinds are shown as configured.
    pub fn display_command(&self) -> String {
        match self.hook_type {
            HookCommandType::Http => redact_url(&self.command),
            _ => self.command.clone(),
        }
    }
}
