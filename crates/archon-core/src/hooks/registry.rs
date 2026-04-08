use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::{Mutex, RwLock};

use serde::Deserialize;

use super::callback::HookCallbackEntry;
use super::condition;
use super::context::HookContext;
use super::executor;
use super::toml_loader;
use super::types::{
    AggregatedHookResult, HookConfig, HookError, HookEvent, HookExecutionConfig, HookMatcher,
};

struct HookEntry {
    source: Option<String>,
    matcher: HookMatcher,
}

/// Registry of hook matchers, organized by `HookEvent`.
///
/// Loaded once at startup from `.archon/settings.json` and optionally
/// extended at runtime by plugins via `register_matchers`.
pub struct HookRegistry {
    entries: HashMap<HookEvent, Vec<HookEntry>>,
    /// Tracks `once: true` hooks that have already fired (event:source:cmd).
    once_fired: Mutex<HashSet<String>>,
    /// Aggregate timeout budget and execution configuration.
    config: HookExecutionConfig,
    /// In-process callbacks registered by plugins/extensions.
    callbacks: RwLock<HashMap<HookEvent, Vec<HookCallbackEntry>>>,
    /// Session-scoped temporary hooks: session_id -> (event -> hooks).
    /// Auto-cleared when SessionEnd fires for the session.
    session_hooks: RwLock<HashMap<String, HashMap<HookEvent, Vec<HookConfig>>>>,
}

#[derive(Deserialize, Default)]
struct SettingsJson {
    #[serde(default)]
    hooks: HashMap<HookEvent, Vec<HookMatcher>>,
}

impl HookRegistry {
    /// Create an empty registry with default configuration.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            once_fired: Mutex::new(HashSet::new()),
            config: HookExecutionConfig::default(),
            callbacks: RwLock::new(HashMap::new()),
            session_hooks: RwLock::new(HashMap::new()),
        }
    }

    /// Create an empty registry with custom execution configuration.
    pub fn with_config(config: HookExecutionConfig) -> Self {
        Self {
            entries: HashMap::new(),
            once_fired: Mutex::new(HashSet::new()),
            config,
            callbacks: RwLock::new(HashMap::new()),
            session_hooks: RwLock::new(HashMap::new()),
        }
    }

    /// Parse `.archon/settings.json` and populate from `"hooks"` field.
    /// Returns `Err` only on JSON parse failure.
    pub fn load_from_settings_json(json: &str) -> Result<Self, HookError> {
        let settings: SettingsJson = serde_json::from_str(json)
            .map_err(|e| HookError::JsonError(format!("settings.json parse error: {e}")))?;

        let mut registry = Self::new();
        for (event, matchers) in settings.hooks {
            registry.register_matchers(event, matchers, None);
        }
        Ok(registry)
    }

    /// Register `HookMatcher` entries for `event` with optional `source` tag.
    /// Load order is execution order.
    pub fn register_matchers(
        &mut self,
        event: HookEvent,
        matchers: Vec<HookMatcher>,
        source: Option<&str>,
    ) {
        let entries = self.entries.entry(event).or_default();
        for matcher in matchers {
            entries.push(HookEntry {
                source: source.map(str::to_owned),
                matcher,
            });
        }
    }

    /// Register a session-scoped temporary hook. Uses interior mutability via `RwLock`.
    /// Auto-cleared when `SessionEnd` fires for the `session_id`.
    pub fn register_session_hook(&self, session_id: &str, event: HookEvent, config: HookConfig) {
        let mut hooks = self
            .session_hooks
            .write()
            .unwrap_or_else(|p| p.into_inner());
        hooks
            .entry(session_id.to_string())
            .or_default()
            .entry(event.clone())
            .or_default()
            .push(config);
        tracing::debug!(
            "Registered session hook for session={} event={:?}",
            session_id,
            event
        );
    }

    /// Remove all session-scoped hooks for the given `session_id`.
    pub fn clear_session_hooks(&self, session_id: &str) {
        let mut hooks = self
            .session_hooks
            .write()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(removed) = hooks.remove(session_id) {
            let count: usize = removed.values().map(|v| v.len()).sum();
            tracing::info!("Cleared {} session hooks for session={}", count, session_id);
        }
    }

    /// Execute all hooks registered for `event` against `input`.
    /// Hooks run in registration order with no short-circuit on Block.
    pub async fn execute_hooks(
        &self,
        event: HookEvent,
        input: serde_json::Value,
        cwd: &Path,
        session_id: &str,
    ) -> AggregatedHookResult {
        // Recursion guard: if we are already inside an agent hook, skip
        // all hook execution to prevent infinite recursion.
        if executor::is_in_hook_agent() {
            tracing::debug!(
                "Skipping hook execution -- already inside agent hook (recursion guard)"
            );
            return AggregatedHookResult::new();
        }

        let empty_entries: Vec<HookEntry> = Vec::new();
        let entries = self.entries.get(&event).unwrap_or(&empty_entries);

        let event_name = event.to_string();
        let mut aggregated = AggregatedHookResult::new();
        let mut skipped: u32 = 0;

        // Aggregate timeout budget tracking.
        let budget_start = std::time::Instant::now();
        let budget = std::time::Duration::from_millis(self.config.aggregate_timeout_ms);

        for entry in entries {
            // Apply HookMatcher.matcher filter against tool_name in input.
            if let Some(ref matcher_str) = entry.matcher.matcher {
                let tool_name = input
                    .get("tool_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !matcher_matches(matcher_str, tool_name) {
                    continue;
                }
            }

            for hook in &entry.matcher.hooks {
                // Check aggregate timeout budget before executing.
                if budget_start.elapsed() >= budget {
                    tracing::warn!(
                        hook = %hook.command,
                        event = %event_name,
                        "aggregate timeout budget exhausted; skipping hook (fail-open)"
                    );
                    skipped += 1;
                    continue;
                }

                // Evaluate if_condition filter.
                if let Some(ref cond) = hook.if_condition
                    && !condition::evaluate(cond, &input)
                {
                    continue;
                }

                // Check once: skip if this hook has already fired.
                let once_key = make_once_key(&event_name, &entry.source, &hook.command);
                if hook.once == Some(true) {
                    let already_fired = self
                        .once_fired
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .contains(&once_key);
                    if already_fired {
                        continue;
                    }
                }

                // Clamp per-hook timeout to remaining budget.
                let remaining = budget.saturating_sub(budget_start.elapsed());
                let remaining_secs = remaining.as_secs().max(1) as u32;
                let hook_timeout = hook.timeout.unwrap_or(60);
                let clamped_timeout = hook_timeout.min(remaining_secs);

                let clamped_hook = if clamped_timeout != hook_timeout {
                    let mut cloned = hook.clone();
                    cloned.timeout = Some(clamped_timeout);
                    cloned
                } else {
                    hook.clone()
                };

                // Execute the hook command with clamped timeout.
                let result =
                    executor::execute_hook(&clamped_hook, &input, cwd, session_id, &event_name)
                        .await;

                // Mark once-hooks as fired after execution.
                if hook.once == Some(true)
                    && let Ok(mut fired) = self.once_fired.lock()
                {
                    fired.insert(once_key);
                }

                // Override source_authority from registry source tag (prevents self-attestation).
                let mut result = result;
                result.source_authority = match entry.source.as_deref() {
                    Some("policy") => Some(crate::hooks::SourceAuthority::Policy),
                    Some("user") => Some(crate::hooks::SourceAuthority::User),
                    Some("project") => Some(crate::hooks::SourceAuthority::Project),
                    Some("local") => Some(crate::hooks::SourceAuthority::Local),
                    _ => None,
                };

                // Accumulate into aggregate (no short-circuit).
                aggregated.merge(result);
            }
        }

        aggregated.skipped_count = skipped;

        // Execute session-scoped hooks for this session_id.
        // Collect under lock, then release before async execution.
        let session_hook_configs: Vec<HookConfig> = {
            let session_hooks = self.session_hooks.read().unwrap_or_else(|p| p.into_inner());
            session_hooks
                .get(session_id)
                .and_then(|m| m.get(&event))
                .cloned()
                .unwrap_or_default()
        };
        for config in &session_hook_configs {
            if budget_start.elapsed() >= budget {
                tracing::warn!(
                    hook = %config.command,
                    event = %event_name,
                    "aggregate timeout budget exhausted; skipping session hook"
                );
                aggregated.skipped_count += 1;
                continue;
            }

            // Clamp per-hook timeout to remaining budget (same as persistent hooks).
            let remaining = budget.saturating_sub(budget_start.elapsed());
            let remaining_secs = remaining.as_secs().max(1) as u32;
            let hook_timeout = config.timeout.unwrap_or(60);
            let clamped_timeout = hook_timeout.min(remaining_secs);

            let clamped_config = if clamped_timeout != hook_timeout {
                let mut cloned = config.clone();
                cloned.timeout = Some(clamped_timeout);
                cloned
            } else {
                config.clone()
            };

            let result =
                executor::execute_hook(&clamped_config, &input, cwd, session_id, &event_name).await;

            // Session hooks have no persistent source tag; strip any
            // self-reported source_authority to prevent privilege escalation.
            let mut result = result;
            result.source_authority = None;

            aggregated.merge(result);
        }

        // Auto-clear session hooks on SessionEnd.
        if event == HookEvent::SessionEnd {
            self.clear_session_hooks(session_id);
        }

        // Execute registered in-process callbacks for this event.
        let tool_name = input
            .get("tool_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let mut ctx_builder = HookContext::builder(event.clone())
            .session_id(session_id.to_string())
            .cwd(cwd.to_string_lossy().to_string());
        if let Some(name) = tool_name {
            ctx_builder = ctx_builder.tool_name(name);
        }
        if let Some(tool_input) = input.get("tool_input") {
            ctx_builder = ctx_builder.tool_input(tool_input.clone());
        }
        if let Some(tool_output) = input.get("tool_output") {
            ctx_builder = ctx_builder.tool_output(tool_output.clone());
        }
        let ctx = ctx_builder.build();
        self.execute_callbacks(&event, &ctx, &mut aggregated, budget_start, budget)
            .await;

        aggregated
    }

    /// Load hooks from all 5 sources in order, with deduplication and authority tagging.
    /// Deduplication: by `(event, hook_type, command)` -- later source wins.
    pub fn load_all(project_root: &Path, home_dir: &Path) -> Self {
        let mut registry = Self::new();

        // 1. settings.json (backward compat, with .claude fallback)
        let new_settings = project_root.join(".archon/settings.json");
        let old_settings = project_root.join(".claude/settings.json");
        let settings_path = if new_settings.exists() {
            new_settings
        } else if old_settings.exists() {
            tracing::warn!(
                "Loading from deprecated path {}. Rename to {} to suppress this warning.",
                old_settings.display(),
                new_settings.display()
            );
            old_settings
        } else {
            new_settings // Will fail to read, which is fine (handled by if let Ok)
        };
        if let Ok(json_str) = std::fs::read_to_string(&settings_path) {
            if let Ok(settings) = serde_json::from_str::<SettingsJson>(&json_str) {
                for (event, matchers) in settings.hooks {
                    registry.register_matchers(event, matchers, Some("project"));
                }
            }
        }

        // 2-5. TOML sources (with .claude fallback for backward compat)
        let sources: [(std::path::PathBuf, std::path::PathBuf, &str); 4] = [
            (
                home_dir.join(".archon/hooks.toml"),
                home_dir.join(".claude/hooks.toml"),
                "user",
            ),
            (
                project_root.join(".archon/hooks.toml"),
                project_root.join(".claude/hooks.toml"),
                "project",
            ),
            (
                project_root.join(".archon/hooks.local.toml"),
                project_root.join(".claude/hooks.local.toml"),
                "local",
            ),
            (
                home_dir.join(".archon/policy/hooks.toml"),
                home_dir.join(".claude/policy/hooks.toml"),
                "policy",
            ),
        ];

        for (new_path, old_path, source_tag) in &sources {
            let effective_path = if new_path.exists() {
                new_path
            } else if old_path.exists() {
                tracing::warn!(
                    "Loading from deprecated path {}. Rename to {} to suppress this warning.",
                    old_path.display(),
                    new_path.display()
                );
                old_path
            } else {
                new_path
            };
            if let Ok(settings) = toml_loader::load_hooks_from_toml(effective_path) {
                for (event, matchers) in settings {
                    registry.register_matchers(event, matchers, Some(source_tag));
                }
            }
        }

        // Deduplicate by (event, hook_type, command) -- keep last
        registry.deduplicate();

        registry
    }

    /// Deduplicate by `(hook_type, command)` per event -- keep last.
    fn deduplicate(&mut self) {
        for entries in self.entries.values_mut() {
            let mut seen: HashSet<(String, String)> = HashSet::new();
            let mut deduped: Vec<HookEntry> = Vec::new();

            // Walk from end so that later sources (higher priority) are seen first.
            for entry in entries.drain(..).rev() {
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
            *entries = deduped;
        }
    }

    /// Return the total number of individual hooks registered (for testing).
    pub fn hook_count(&self) -> usize {
        self.entries
            .values()
            .flat_map(|v| v.iter())
            .map(|e| e.matcher.hooks.len())
            .sum()
    }

    /// Register an in-process callback for the given event.
    pub fn register_callback(&self, event: HookEvent, entry: HookCallbackEntry) {
        let mut map = self.callbacks.write().unwrap_or_else(|p| p.into_inner());
        map.entry(event).or_default().push(entry);
    }

    /// Remove a previously registered callback by name.
    pub fn unregister_callback(&self, event: &HookEvent, name: &str) {
        let mut map = self.callbacks.write().unwrap_or_else(|p| p.into_inner());
        if let Some(entries) = map.get_mut(event) {
            entries.retain(|e| e.name != name);
        }
    }

    /// Execute registered callbacks for `event`, participating in the aggregate
    /// timeout budget. Each runs in `spawn_blocking` with `catch_unwind` + timeout.
    async fn execute_callbacks(
        &self,
        event: &HookEvent,
        ctx: &HookContext,
        aggregated: &mut AggregatedHookResult,
        budget_start: std::time::Instant,
        budget: std::time::Duration,
    ) {
        // Collect callback info under a short read-lock.
        let callback_snapshot: Vec<(String, super::callback::HookCallback, u32)> = {
            let map = self.callbacks.read().unwrap_or_else(|p| p.into_inner());
            match map.get(event) {
                Some(entries) => entries
                    .iter()
                    .map(|e| (e.name.clone(), e.callback.clone(), e.timeout_secs))
                    .collect(),
                None => return,
            }
        };

        for (name, cb, timeout_secs) in callback_snapshot {
            // Check aggregate budget before each callback.
            if budget_start.elapsed() >= budget {
                tracing::warn!(callback = %name, "aggregate timeout budget exhausted; skipping callback");
                aggregated.skipped_count += 1;
                continue;
            }

            // Clamp per-callback timeout to remaining budget.
            let remaining = budget.saturating_sub(budget_start.elapsed());
            let effective_timeout = std::cmp::min(
                std::time::Duration::from_secs(timeout_secs as u64),
                remaining,
            );

            let ctx_clone = ctx.clone();

            let task_result = tokio::time::timeout(
                effective_timeout,
                tokio::task::spawn_blocking(move || {
                    std::panic::catch_unwind(AssertUnwindSafe(|| cb(&ctx_clone)))
                }),
            )
            .await;

            match task_result {
                Ok(Ok(Ok(result))) => {
                    aggregated.merge(result);
                }
                Ok(Ok(Err(_panic))) => {
                    tracing::warn!(callback = %name, "callback panicked; treating as success");
                }
                Ok(Err(join_err)) => {
                    tracing::warn!(callback = %name, error = %join_err, "callback join error");
                }
                Err(_timeout) => {
                    tracing::warn!(
                        callback = %name,
                        timeout_secs,
                        "callback timed out; treating as success"
                    );
                }
            }
        }
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a HookMatcher.matcher string matches a tool name.
/// Simple equality check; "*" matches everything.
fn matcher_matches(matcher: &str, tool_name: &str) -> bool {
    matcher == "*" || matcher == tool_name
}

fn make_once_key(event_name: &str, source: &Option<String>, command: &str) -> String {
    format!("{event_name}:{}:{command}", source.as_deref().unwrap_or(""))
}
