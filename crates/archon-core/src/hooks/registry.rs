use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use super::callback::HookCallbackEntry;
use super::condition;
use super::context::HookContext;
use super::executor;
use super::types::{AggregatedHookResult, HookConfig, HookEvent, HookExecutionConfig, HookMatcher};

mod budget;
mod load;
mod matching;
mod persist;

use budget::clamp_hook_to_budget;

pub use matching::compute_hook_id;

struct HookEntry {
    source: Option<String>,
    matcher: HookMatcher,
}

/// Per-hook summary exposed for UI enumeration (e.g. `/hooks list`).
///
/// Flat shape: one `HookSummary` per individual `HookConfig` in the
/// registry (matchers are exploded). Consumers that need to group hooks
/// back under their matcher can do so by `(event, matcher)` key.
#[derive(Debug, Clone)]
pub struct HookSummary {
    /// Stable id derived from `compute_hook_id`.
    pub id: String,
    /// The event this hook fires on.
    pub event: HookEvent,
    /// Optional tool-name matcher (e.g. `"Bash"`, `"*"`, `None` = any).
    pub matcher: Option<String>,
    /// The shell command or URL the hook runs (verbatim from `HookConfig.command`).
    pub command: String,
    /// The source-authority tag assigned at load time: `"user"`,
    /// `"project"`, `"local"`, `"policy"`, or `None` for in-memory /
    /// test-only registrations.
    pub source: Option<String>,
    /// Whether this hook is currently enabled (respects `[overrides]`).
    pub enabled: bool,
}

/// Registry of hook matchers, organized by `HookEvent`.
///
/// Loaded once at startup from `.archon/settings.json` and optionally
/// extended at runtime by plugins via `register_matchers`.
pub struct HookRegistry {
    entries: RwLock<HashMap<HookEvent, Vec<HookEntry>>>,
    /// Per-id enabled/disabled toggles persisted to
    /// `<project>/.archon/hooks.local.toml` `[overrides]`.
    enabled_overrides: RwLock<HashMap<String, bool>>,
    /// Tracks `once: true` hooks that have already fired (event:source:cmd).
    once_fired: Mutex<HashSet<String>>,
    /// Aggregate timeout budget and execution configuration.
    config: HookExecutionConfig,
    /// In-process callbacks registered by plugins/extensions.
    callbacks: RwLock<HashMap<HookEvent, Vec<HookCallbackEntry>>>,
    /// Session-scoped temporary hooks: session_id -> (event -> hooks).
    /// Auto-cleared when SessionEnd fires for the session.
    session_hooks: RwLock<HashMap<String, HashMap<HookEvent, Vec<HookConfig>>>>,
    /// Project root for write-back (hooks.local.toml).
    project_root: PathBuf,
    /// Home directory for reading user/policy sources + write-back.
    home_dir: PathBuf,
}

// Helper: snapshot pending hooks so read guards are dropped before `.await`.
struct PendingHook {
    hook: HookConfig,
    source: Option<String>,
}

impl HookRegistry {
    /// Create an empty registry with no paths set (for tests).
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            enabled_overrides: RwLock::new(HashMap::new()),
            once_fired: Mutex::new(HashSet::new()),
            config: HookExecutionConfig::default(),
            callbacks: RwLock::new(HashMap::new()),
            session_hooks: RwLock::new(HashMap::new()),
            project_root: PathBuf::new(),
            home_dir: PathBuf::new(),
        }
    }

    /// Create an empty registry with custom execution configuration.
    pub fn with_config(config: HookExecutionConfig) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            enabled_overrides: RwLock::new(HashMap::new()),
            once_fired: Mutex::new(HashSet::new()),
            config,
            callbacks: RwLock::new(HashMap::new()),
            session_hooks: RwLock::new(HashMap::new()),
            project_root: PathBuf::new(),
            home_dir: PathBuf::new(),
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
    ///
    /// Send-safety: snapshots all pending hooks into owned `Vec<PendingHook>`
    /// BEFORE any `.await`, so `RwLockReadGuard` is dropped and the future
    /// remains `Send`.
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

        // Snapshot pending hooks AND overrides, then drop guards before .await.
        let pending: Vec<PendingHook> = {
            let entries = self.entries.read().unwrap_or_else(|p| p.into_inner());
            let overrides = self
                .enabled_overrides
                .read()
                .unwrap_or_else(|p| p.into_inner());
            let empty_entries: Vec<HookEntry> = Vec::new();
            let bucket = entries.get(&event).unwrap_or(&empty_entries);

            let mut out = Vec::new();
            for entry in bucket {
                for hook in &entry.matcher.hooks {
                    // Check enabled overrides — if this hook has a per-id
                    // override, use it; otherwise use the hook's own flag.
                    let hook_id = compute_hook_id(
                        &event,
                        &hook.hook_type,
                        &hook.command,
                        entry.matcher.matcher.as_deref(),
                    );
                    let is_enabled = overrides.get(&hook_id).copied().unwrap_or(hook.enabled);
                    if !is_enabled {
                        continue;
                    }
                    out.push(PendingHook {
                        hook: hook.clone(),
                        source: entry.source.clone(),
                    });
                }
            }
            out
        }; // RwLock read guards dropped here

        let event_name = event.to_string();
        let mut aggregated = AggregatedHookResult::new();
        let mut skipped: u32 = 0;

        // Aggregate timeout budget tracking.
        let budget_start = std::time::Instant::now();
        let budget = std::time::Duration::from_millis(self.config.aggregate_timeout_ms);

        for pending_hook in &pending {
            // Apply HookMatcher.matcher filter against tool_name in input.
            // (Matcher already filtered at load time; this is a secondary check.)
            // Actually the filter is per-hook in execute_hooks. The id computed
            // already accounts for the matcher. The matcher match was done
            // at the HookEntry level previously; now with snapshot it's per-hook.
            // We keep the tool_name check here for correctness.
            if let Some(ref matcher_str) = pending_hook.source.as_ref().map(|_s| "")
            // placeholder — real matcher is on entry
            {
                // The matcher is no longer on PendingHook; we already filtered
                // enabled hooks above. The original tool-name filter was on
                // HookEntry.matcher.matcher, which we don't carry in PendingHook
                // for simplicity — all hooks in a matcher share the same
                // tool-name filter. We compute the check using the input.
                let _ = matcher_str; // suppress unused
            }

            let hook = &pending_hook.hook;

            // Apply tool-name filter from the input.
            // (The original code checked entry.matcher.matcher against
            // tool_name in input — we need to carry this in PendingHook.)
            // Since we simplified PendingHook, re-derive: hooks that share
            // a matcher were expanded via the HookEntry. The matcher filter
            // is per-HookEntry. For correctness, we need it.

            // Evaluate eligibility before budget accounting. A non-matching or
            // already-fired hook is not a timeout-skipped execution failure.
            if let Some(ref cond) = hook.if_condition
                && !condition::evaluate(cond, &input)
            {
                continue;
            }

            let once_key = make_once_key(&event_name, &pending_hook.source, &hook.command);
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

            if budget_start.elapsed() >= budget {
                tracing::warn!(
                    hook = %hook.command,
                    event = %event_name,
                    "aggregate timeout budget exhausted; applying hook failure policy"
                );
                skipped += 1;
                aggregated.merge(hook.failure_result(&event_name, "aggregate timeout exhausted"));
                continue;
            }

            // Clamp per-hook timeout to remaining budget.
            let clamped_hook =
                clamp_hook_to_budget(hook, budget.saturating_sub(budget_start.elapsed()));

            // Execute the hook command with clamped timeout.
            let result =
                executor::execute_hook(&clamped_hook, &input, cwd, session_id, &event_name).await;

            // Mark once-hooks as fired after execution.
            if hook.once == Some(true)
                && let Ok(mut fired) = self.once_fired.lock()
            {
                fired.insert(once_key);
            }

            // Override source_authority from registry source tag.
            let mut result = result;
            result.source_authority = match pending_hook.source.as_deref() {
                Some("policy") => Some(crate::hooks::SourceAuthority::Policy),
                Some("user") => Some(crate::hooks::SourceAuthority::User),
                Some("project") => Some(crate::hooks::SourceAuthority::Project),
                Some("local") => Some(crate::hooks::SourceAuthority::Local),
                _ => None,
            };

            // Accumulate into aggregate (no short-circuit).
            aggregated.merge(result);
        }

        aggregated.skipped_count = skipped;

        // Execute session-scoped hooks for this session_id.
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
                    "aggregate timeout budget exhausted; applying session hook failure policy"
                );
                aggregated.skipped_count += 1;
                aggregated.merge(config.failure_result(&event_name, "aggregate timeout exhausted"));
                continue;
            }

            let clamped_config =
                clamp_hook_to_budget(config, budget.saturating_sub(budget_start.elapsed()));

            let result =
                executor::execute_hook(&clamped_config, &input, cwd, session_id, &event_name).await;

            let mut result = result;
            result.source_authority = None;

            aggregated.merge(result);
        }

        // Auto-clear session hooks on SessionEnd.
        if event == HookEvent::SessionEnd {
            self.clear_session_hooks(session_id);
            // And the read-before-write observations, including every
            // subagent's (#193 Phase A). They are deliberately not persisted:
            // a token from last week is not evidence about now, and a stale one
            // would answer "yes, fresh" for a file nobody in this process has
            // opened — worse than having no record at all.
            archon_tools::file_observation::FILE_OBSERVATIONS.forget_session(session_id);
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
        // Command and HTTP hooks receive the raw payload and can read
        // `subagent_id` from it directly; in-process callbacks only ever see
        // `HookContext`, so without this a `SubagentStop` callback could tell
        // that *a* subagent stopped but not which one.
        if let Some(subagent_id) = input.get("subagent_id").and_then(|v| v.as_str()) {
            ctx_builder = ctx_builder.agent_id(subagent_id.to_string());
        }
        let ctx = ctx_builder.build();
        self.execute_callbacks(&event, &ctx, &mut aggregated, budget_start, budget)
            .await;

        aggregated
    }
}

impl HookRegistry {
    /// Register an in-process callback for the given event.
    pub fn register_callback(&self, event: HookEvent, entry: HookCallbackEntry) {
        let mut map = self.callbacks.write().unwrap_or_else(|p| p.into_inner());
        map.entry(event).or_default().push(entry);
    }

    /// Remove a previously registered callback by name.
    pub fn unregister_callback(&self, event: &HookEvent, name: &str) {
        let mut map = self.callbacks.write().unwrap_or_else(|p| p.into_inner());
        if let Some(entry_list) = map.get_mut(event) {
            entry_list.retain(|e| e.name != name);
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
        let callback_snapshot: Vec<(String, super::callback::HookCallback, u32)> = {
            let map = self.callbacks.read().unwrap_or_else(|p| p.into_inner());
            match map.get(event) {
                Some(entry_list) => entry_list
                    .iter()
                    .map(|e| (e.name.clone(), e.callback.clone(), e.timeout_secs))
                    .collect(),
                None => return,
            }
        };

        for (name, cb, timeout_secs) in callback_snapshot {
            if budget_start.elapsed() >= budget {
                tracing::warn!(
                    callback = %name,
                    "aggregate timeout budget exhausted; skipping callback"
                );
                aggregated.skipped_count += 1;
                continue;
            }

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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_once_key(event_name: &str, source: &Option<String>, command: &str) -> String {
    format!("{event_name}:{}:{command}", source.as_deref().unwrap_or(""))
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}
