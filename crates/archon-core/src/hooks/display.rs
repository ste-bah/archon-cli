//! How a hook is named wherever it reaches a log line or a failure reason.

use super::types::{HookCommandType, HookConfig};
use crate::url_redact::redact_url;

impl HookConfig {
    /// The hook's command as it may be shown to people and models.
    ///
    /// HTTP credentials, command arguments, and shell expressions may contain
    /// secrets, so user and model facing displays use a redacted name.
    pub fn display_command(&self) -> String {
        self.redacted_command()
    }

    /// The hook's command reduced to what a model may see: an HTTP hook's
    /// URL origin, or the program name of any other hook without its
    /// arguments, which can carry tokens.
    pub(crate) fn redacted_command(&self) -> String {
        match self.hook_type {
            HookCommandType::Http => redact_url(&self.command),
            _ => redacted_program(&self.command),
        }
    }

    /// Command value for operator logs; HTTP credentials stay redacted.
    pub(crate) fn operator_command(&self) -> String {
        match self.hook_type {
            HookCommandType::Http => redact_url(&self.command),
            _ => self.command.clone(),
        }
    }

    /// The model-visible reason for an allowing no-progress stop.
    pub(crate) fn no_progress_reason(
        &self,
        hook_id: Option<&str>,
        event_name: &str,
        error: &str,
    ) -> String {
        let id = hook_id.map(|id| format!(" {id}")).unwrap_or_default();
        format!(
            "hook{id} on {event_name} (`{}`) stopped: {error}",
            self.redacted_command()
        )
    }
}

/// The first word's file name when it is a plain program name, else a fixed
/// placeholder (an environment assignment or quoted word could carry a value).
fn redacted_program(command: &str) -> String {
    let mut words = command.split_whitespace();
    let Some(first) = words.next() else {
        return "<empty command>".to_owned();
    };
    let program = first.rsplit(['/', '\\']).next().unwrap_or(first);
    let plain = !program.is_empty()
        && program
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'));
    let shown = if plain { program } else { "<shell command>" };
    if words.next().is_some() {
        format!("{shown} ...")
    } else {
        shown.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::redacted_program;

    #[test]
    fn arguments_are_never_shown() {
        assert_eq!(redacted_program("curl -H 'Bearer sk-live-1' x"), "curl ...");
        assert_eq!(
            redacted_program("/opt/hooks/check.sh --token t"),
            "check.sh ..."
        );
        assert_eq!(redacted_program("lint"), "lint");
    }

    #[test]
    fn values_in_the_first_word_are_never_shown() {
        assert_eq!(
            redacted_program("TOKEN=sk-live-2 run"),
            "<shell command> ..."
        );
        assert_eq!(redacted_program("'my hook' a"), "<shell command> ...");
        assert_eq!(redacted_program("  "), "<empty command>");
    }
}
