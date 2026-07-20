use std::collections::HashSet;

use sha2::{Digest, Sha256};

use super::{HookEntry, HookRegistry, HookSummary};
use crate::hooks::types::{HookCommandType, HookEvent, HookMatcher};

/// Deterministic SHA-256-based id for a hook (8 hex chars, prefixed `h`).
///
/// Hash inputs (in order): event JSON, hook-type discriminant string,
/// command, matcher (empty string for None). The 5 discriminant variants
/// are enumerated explicitly — a future 6th variant will require a manual
/// extension here (no wildcard arm).
pub fn compute_hook_id(
    event: &HookEvent,
    hook_type: &HookCommandType,
    command: &str,
    matcher: Option<&str>,
) -> String {
    let event_json = serde_json::to_string(event).unwrap_or_default();
    let type_str = hook_command_type_discriminant(hook_type);
    let matcher_str = matcher.unwrap_or("");

    let mut hasher = Sha256::new();
    hasher.update(event_json.as_bytes());
    hasher.update(type_str.as_bytes());
    hasher.update(command.as_bytes());
    hasher.update(matcher_str.as_bytes());
    let digest = hasher.finalize();
    format!(
        "h{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3]
    )
}

/// Return a stable discriminant string for each `HookCommandType` variant.
/// ALL FIVE arms are enumerated; do NOT add a wildcard — a future 6th
/// variant must explicitly extend the hash scheme.
pub fn hook_command_type_discriminant(t: &HookCommandType) -> &'static str {
    match t {
        HookCommandType::Command => "command",
        HookCommandType::Prompt => "prompt",
        HookCommandType::Agent => "agent",
        HookCommandType::Http => "http",
        HookCommandType::Function => "function",
    }
}

impl HookRegistry {
    /// Register `HookMatcher` entries for `event` with optional `source` tag.
    /// Load order is execution order. Computes stable ids for each hook.
    pub fn register_matchers(
        &self,
        event: HookEvent,
        matchers: Vec<HookMatcher>,
        source: Option<&str>,
    ) {
        let mut entries = self.entries.write().unwrap_or_else(|p| p.into_inner());
        let bucket = entries.entry(event.clone()).or_default();
        for matcher in matchers {
            // Compute ids for each hook config in the matcher (done eagerly
            // so id is stable even before dedup runs).
            let mut matcher_with_ids = matcher.clone();
            for hook in &mut matcher_with_ids.hooks {
                let _id = compute_hook_id(
                    &event,
                    &hook.hook_type,
                    &hook.command,
                    matcher_with_ids.matcher.as_deref(),
                );
            }
            bucket.push(HookEntry {
                source: source.map(str::to_owned),
                matcher: matcher_with_ids,
            });
        }
    }

    /// Deduplicate by `(hook_type, command)` per event -- keep last.
    pub(super) fn deduplicate(&self) {
        let mut entries = self.entries.write().unwrap_or_else(|p| p.into_inner());
        for bucket in entries.values_mut() {
            let mut seen: HashSet<(String, String)> = HashSet::new();
            let mut deduped: Vec<HookEntry> = Vec::new();

            for entry in bucket.drain(..).rev() {
                let mut kept_hooks = Vec::new();
                for hook in entry.matcher.hooks.iter().rev() {
                    let key = (format!("{:?}", hook.hook_type), hook.command.clone());
                    if seen.insert(key) {
                        kept_hooks.push(hook.clone());
                    }
                }
                if !kept_hooks.is_empty() {
                    kept_hooks.reverse();
                    deduped.push(HookEntry {
                        source: entry.source,
                        matcher: HookMatcher {
                            matcher: entry.matcher.matcher,
                            hooks: kept_hooks,
                        },
                    });
                }
            }

            deduped.reverse();
            *bucket = deduped;
        }
    }

    /// Return the total number of individual hooks registered (for testing).
    pub fn hook_count(&self) -> usize {
        self.entries
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .flat_map(|v| v.iter())
            .map(|e| e.matcher.hooks.len())
            .sum()
    }

    /// Iterate every registered hook as a flat `HookSummary` vector in a
    /// stable order (events sorted by `Debug` name, matchers in
    /// registration order, hooks in declaration order).
    pub fn summaries(&self) -> Vec<HookSummary> {
        let entries = self.entries.read().unwrap_or_else(|p| p.into_inner());
        let overrides = self
            .enabled_overrides
            .read()
            .unwrap_or_else(|p| p.into_inner());

        let mut events: Vec<HookEvent> = entries.keys().cloned().collect();
        events.sort_by_key(|e| format!("{e:?}"));

        let mut out: Vec<HookSummary> = Vec::new();
        for event in events {
            let Some(bucket) = entries.get(&event) else {
                continue;
            };
            for entry in bucket {
                for hook in &entry.matcher.hooks {
                    let hook_id = compute_hook_id(
                        &event,
                        &hook.hook_type,
                        &hook.command,
                        entry.matcher.matcher.as_deref(),
                    );
                    let enabled = overrides.get(&hook_id).copied().unwrap_or(hook.enabled);
                    out.push(HookSummary {
                        id: hook_id,
                        event: event.clone(),
                        matcher: entry.matcher.matcher.clone(),
                        command: hook.command.clone(),
                        source: entry.source.clone(),
                        enabled,
                    });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::types::{HookConfig, HookMatcher};

    fn make_hook(cmd: &str) -> HookConfig {
        HookConfig {
            hook_type: HookCommandType::Command,
            command: cmd.to_string(),
            if_condition: None,
            timeout: None,
            once: None,
            r#async: None,
            async_rewake: None,
            status_message: None,
            headers: std::collections::HashMap::new(),
            allowed_env_vars: Vec::new(),
            on_failure: None,
            enabled: true,
        }
    }

    #[test]
    fn compute_hook_id_is_stable() {
        let id1 = compute_hook_id(
            &HookEvent::PreToolUse,
            &HookCommandType::Command,
            "echo hello",
            Some("Bash"),
        );
        let id2 = compute_hook_id(
            &HookEvent::PreToolUse,
            &HookCommandType::Command,
            "echo hello",
            Some("Bash"),
        );
        assert_eq!(id1, id2, "hook id must be deterministic");
        assert!(id1.starts_with('h'), "id must start with 'h'");
        assert_eq!(id1.len(), 9, "id must be 9 chars (h + 8 hex)");
    }

    #[test]
    fn hook_command_type_discriminant_covers_all_five_variants() {
        // Must compile — no wildcard arm.
        let variants = [
            HookCommandType::Command,
            HookCommandType::Prompt,
            HookCommandType::Agent,
            HookCommandType::Http,
            HookCommandType::Function,
        ];
        for v in &variants {
            let s = hook_command_type_discriminant(v);
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn compute_hook_id_different_inputs_produce_different_ids() {
        let id1 = compute_hook_id(
            &HookEvent::PreToolUse,
            &HookCommandType::Command,
            "echo hello",
            None,
        );
        let id2 = compute_hook_id(
            &HookEvent::PostToolUse,
            &HookCommandType::Command,
            "echo hello",
            None,
        );
        assert_ne!(id1, id2, "different events must produce different ids");

        let id3 = compute_hook_id(
            &HookEvent::PreToolUse,
            &HookCommandType::Prompt,
            "echo hello",
            None,
        );
        assert_ne!(id1, id3, "different hook types must produce different ids");

        let id4 = compute_hook_id(
            &HookEvent::PreToolUse,
            &HookCommandType::Command,
            "different",
            None,
        );
        assert_ne!(id1, id4, "different commands must produce different ids");

        let id5 = compute_hook_id(
            &HookEvent::PreToolUse,
            &HookCommandType::Command,
            "echo hello",
            Some("Bash"),
        );
        assert_ne!(id1, id5, "different matchers must produce different ids");
    }

    #[test]
    fn summaries_empty_registry_is_empty() {
        let reg = HookRegistry::new();
        assert_eq!(reg.summaries().len(), 0);
    }

    #[test]
    fn summaries_exposes_every_hook_with_source_and_matcher_and_ids() {
        let reg = HookRegistry::new();
        reg.register_matchers(
            HookEvent::PreToolUse,
            vec![HookMatcher {
                matcher: Some("Bash".to_string()),
                hooks: vec![make_hook("guard-secrets"), make_hook("audit-log")],
            }],
            Some("project"),
        );
        reg.register_matchers(
            HookEvent::SessionStart,
            vec![HookMatcher {
                matcher: None,
                hooks: vec![make_hook("welcome.sh")],
            }],
            Some("user"),
        );

        let summaries = reg.summaries();
        assert_eq!(
            summaries.len(),
            3,
            "summaries() must produce one entry per HookConfig (2 + 1 = 3)"
        );

        // All summaries must have stable ids.
        for s in &summaries {
            assert!(s.id.starts_with('h'), "id must start with 'h': {:?}", s.id);
            assert_eq!(s.id.len(), 9, "id must be 9 chars");
            assert!(s.enabled, "hooks default to enabled");
        }

        assert_eq!(summaries[0].event, HookEvent::PreToolUse);
        assert_eq!(summaries[0].matcher.as_deref(), Some("Bash"));
        assert_eq!(summaries[0].command, "guard-secrets");
        assert_eq!(summaries[0].source.as_deref(), Some("project"));

        assert_eq!(summaries[1].event, HookEvent::PreToolUse);
        assert_eq!(summaries[1].matcher.as_deref(), Some("Bash"));
        assert_eq!(summaries[1].command, "audit-log");
        assert_eq!(summaries[1].source.as_deref(), Some("project"));

        assert_eq!(summaries[2].event, HookEvent::SessionStart);
        assert!(summaries[2].matcher.is_none());
        assert_eq!(summaries[2].command, "welcome.sh");
        assert_eq!(summaries[2].source.as_deref(), Some("user"));

        // Ids must differ for different hooks.
        assert_ne!(summaries[0].id, summaries[1].id);
        assert_ne!(summaries[1].id, summaries[2].id);
    }
}
