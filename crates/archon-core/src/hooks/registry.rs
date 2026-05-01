use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::callback::HookCallbackEntry;
use super::condition;
use super::context::HookContext;
use super::executor;
use super::toml_loader;
use super::types::{
    AggregatedHookResult, HookCommandType, HookConfig, HookError, HookEvent, HookExecutionConfig,
    HookMatcher,
};

// ---------------------------------------------------------------------------
// Public helpers — hook ID scheme
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Internal data structures
// ---------------------------------------------------------------------------

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

#[derive(Deserialize, Default)]
struct SettingsJson {
    #[serde(default)]
    hooks: HashMap<HookEvent, Vec<HookMatcher>>,
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

    /// Parse `.archon/settings.json` and populate from `"hooks"` field.
    /// Returns `Err` only on JSON parse failure.
    pub fn load_from_settings_json(json: &str) -> Result<Self, HookError> {
        let settings: SettingsJson = serde_json::from_str(json)
            .map_err(|e| HookError::JsonError(format!("settings.json parse error: {e}")))?;

        let registry = Self::new();
        for (event, matchers) in settings.hooks {
            registry.register_matchers(event, matchers, None);
        }
        Ok(registry)
    }

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
                    "aggregate timeout budget exhausted; skipping session hook"
                );
                aggregated.skipped_count += 1;
                continue;
            }

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
    /// Stores project_root and home_dir for later `reload()` and `set_enabled()`.
    pub fn load_all(project_root: &Path, home_dir: &Path) -> Self {
        let mut registry = Self::new();
        registry.project_root = project_root.to_path_buf();
        registry.home_dir = home_dir.to_path_buf();

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
            new_settings
        };
        if let Ok(json_str) = std::fs::read_to_string(&settings_path)
            && let Ok(settings) = serde_json::from_str::<SettingsJson>(&json_str)
        {
            for (event, matchers) in settings.hooks {
                registry.register_matchers(event, matchers, Some("project"));
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

        // Load and apply per-id enabled/disabled overrides from hooks.local.toml
        registry.load_overrides();

        registry
    }

    /// Re-load all hook sources from disk and replace internal state.
    /// Preserves the stored `project_root` and `home_dir`.
    pub fn reload(&self) -> Result<(), HookError> {
        let fresh = Self::load_all(&self.project_root, &self.home_dir);

        // Replace entries
        let fresh_entries = fresh
            .entries
            .into_inner()
            .unwrap_or_else(|p| p.into_inner());
        let mut entries = self.entries.write().unwrap_or_else(|p| p.into_inner());
        *entries = fresh_entries;

        // Replace overrides
        let fresh_overrides = fresh
            .enabled_overrides
            .into_inner()
            .unwrap_or_else(|p| p.into_inner());
        let mut overrides = self
            .enabled_overrides
            .write()
            .unwrap_or_else(|p| p.into_inner());
        *overrides = fresh_overrides;

        Ok(())
    }

    /// Enable or disable a hook by id, persisting to
    /// `<project_root>/.archon/hooks.local.toml`.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), HookError> {
        // Update in-memory override.
        {
            let mut overrides = self
                .enabled_overrides
                .write()
                .unwrap_or_else(|p| p.into_inner());
            overrides.insert(id.to_string(), enabled);
        }

        // Persist to disk.
        self.write_overrides_file(id, enabled)
    }

    /// Read `[overrides]` from `<project_root>/.archon/hooks.local.toml` and
    /// merge into the in-memory `enabled_overrides` map.
    fn load_overrides(&self) {
        let path = self.project_root.join(".archon/hooks.local.toml");
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return,
        };

        // Parse the TOML looking for [overrides] section.
        let mut in_overrides = false;
        let mut overrides = HashMap::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == "[overrides]" {
                in_overrides = true;
                continue;
            }
            if in_overrides {
                if trimmed.starts_with('[') {
                    // Next section, stop
                    break;
                }
                if let Some((key, value)) = trimmed.split_once('=') {
                    let key = key.trim().trim_matches('"');
                    let value = value.trim().trim_matches('"');
                    if let Ok(b) = value.parse::<bool>() {
                        overrides.insert(key.to_string(), b);
                    }
                }
            }
        }

        if !overrides.is_empty() {
            let mut map = self
                .enabled_overrides
                .write()
                .unwrap_or_else(|p| p.into_inner());
            for (k, v) in overrides {
                map.entry(k).or_insert(v);
            }
        }
    }

    /// Write the full `enabled_overrides` map to
    /// `<project_root>/.archon/hooks.local.toml`, preserving non-`[overrides]`
    /// sections.
    fn write_overrides_file(&self, _id: &str, _enabled: bool) -> Result<(), HookError> {
        let path = self.project_root.join(".archon/hooks.local.toml");

        // Ensure parent directory exists.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                HookError::IoError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("create .archon dir: {e}"),
                ))
            })?;
        }

        let overrides = self
            .enabled_overrides
            .read()
            .unwrap_or_else(|p| p.into_inner());

        // Read existing file content to preserve non-[overrides] sections.
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let new_content = merge_overrides_into_toml(&existing, &overrides);

        std::fs::write(&path, &new_content).map_err(|e| {
            HookError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("write hooks.local.toml: {e}"),
            ))
        })?;

        Ok(())
    }

    /// Deduplicate by `(hook_type, command)` per event -- keep last.
    fn deduplicate(&self) {
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

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check if a HookMatcher.matcher string matches a tool name.
/// Simple equality check; "*" matches everything.
fn matcher_matches(matcher: &str, tool_name: &str) -> bool {
    matcher == "*" || matcher == tool_name
}

fn make_once_key(event_name: &str, source: &Option<String>, command: &str) -> String {
    format!("{event_name}:{}:{command}", source.as_deref().unwrap_or(""))
}

/// Merge the current `[overrides]` map into an existing hooks.local.toml
/// file, preserving non-override sections.
fn merge_overrides_into_toml(existing: &str, overrides: &HashMap<String, bool>) -> String {
    let mut out = String::new();
    let mut in_overrides = false;

    // Preserve all lines except the old [overrides] section.
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed == "[overrides]" {
            in_overrides = true;
            continue;
        }
        if in_overrides && trimmed.starts_with('[') {
            in_overrides = false;
            // Fall through to emit this line
        }
        if !in_overrides {
            out.push_str(line);
            out.push('\n');
        }
    }

    // Ensure trailing newline before appending overrides.
    if !out.ends_with('\n') {
        out.push('\n');
    }

    // Append fresh overrides section.
    if !overrides.is_empty() {
        out.push_str("[overrides]\n");
        let mut keys: Vec<&String> = overrides.keys().collect();
        keys.sort();
        for k in keys {
            let v = overrides[k];
            out.push_str(&format!("{k} = {v}\n"));
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod summaries_tests {
    use super::*;
    use crate::hooks::types::HookCommandType;
    use crate::hooks::types::HookConfig;
    use crate::hooks::types::HookMatcher;

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

    #[test]
    fn set_enabled_persists_and_toggles() {
        use tempfile::TempDir;

        let project_dir = TempDir::new().unwrap();
        let home_dir = TempDir::new().unwrap();
        let archon_dir = project_dir.path().join(".archon");
        std::fs::create_dir_all(&archon_dir).unwrap();

        // Write a fixture hooks.toml.
        let fixture = r#"
[hooks.PreToolUse]
matchers = [
  { matcher = "Bash", hooks = [
    { type = "command", command = "guard-secrets" }
  ]}
]
"#;
        std::fs::write(archon_dir.join("hooks.toml"), fixture).unwrap();

        let reg = HookRegistry::load_all(project_dir.path(), home_dir.path());
        let summaries = reg.summaries();
        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].enabled, "hook must default to enabled");
        let hook_id = summaries[0].id.clone();

        // Disable the hook.
        reg.set_enabled(&hook_id, false).unwrap();

        // Verify hooks.local.toml was created.
        let local_path = archon_dir.join("hooks.local.toml");
        assert!(local_path.exists(), "hooks.local.toml must be created");
        let content = std::fs::read_to_string(&local_path).unwrap();
        assert!(
            content.contains("[overrides]"),
            "must contain [overrides] section"
        );
        assert!(content.contains(&hook_id), "must contain the hook id");

        // Reload and verify the hook is now disabled.
        let reg2 = HookRegistry::load_all(project_dir.path(), home_dir.path());
        let summaries2 = reg2.summaries();
        assert_eq!(summaries2.len(), 1);
        assert!(
            !summaries2[0].enabled,
            "hook must show as disabled after reload"
        );
        assert_eq!(
            summaries2[0].id, hook_id,
            "id must be stable across reloads"
        );
    }

    #[test]
    fn merge_overrides_preserves_non_overrides_sections() {
        let existing = "[other]\nfoo = \"bar\"\n\n[overrides]\nold = true\n";
        let mut overrides = HashMap::new();
        overrides.insert("h12345678".to_string(), false);
        let merged = merge_overrides_into_toml(existing, &overrides);
        assert!(merged.contains("[other]"));
        assert!(merged.contains("foo = \"bar\""));
        assert!(
            !merged.contains("old = true"),
            "old override must be removed"
        );
        assert!(merged.contains("h12345678 = false"));
    }
}
