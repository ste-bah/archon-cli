//! Registry policy, isolation and schema configuration.
use super::*;

impl ToolRegistry {
    /// Restrict this registry's `Bash` to the agent's isolation tier (#184 M3).
    ///
    /// The registry is built per subagent, which makes it the one place a
    /// per-agent restriction can live: `ToolContext` is inherited verbatim from
    /// the parent, and the admission callback is skipped for `Safe` tools.
    ///
    /// Returns `false` when there is no `Bash` to restrict — a read-only agent
    /// whose allowlist excluded it, which needs no restriction anyway.
    pub fn set_bash_isolation_tier(
        &mut self,
        tier: archon_tools::isolation::IsolationTier,
    ) -> bool {
        // Nothing to enforce, and rebuilding the tool would discard whatever
        // provider-env configuration was attached moments earlier.
        if tier == archon_tools::isolation::IsolationTier::Shared {
            return true;
        }
        let Some(bash) = self.tools.get("Bash") else {
            return false;
        };
        // The same call attaches the shared build-cache pool, because this is
        // the one place that knows the agent is isolated AND allowed to build —
        // which is exactly when a leased cache directory applies. An agent
        // without that tier builds where it always did.
        let pool = matches!(
            tier,
            archon_tools::isolation::IsolationTier::WorktreeWithBuilds
        )
        .then(archon_tools::build_cache_lease::shared_build_cache_pool)
        .flatten();
        let Some(restricted) = bash.with_isolation_tier(tier, pool) else {
            return false;
        };
        self.replace(restricted);
        true
    }

    pub fn attach_provider_env_to_bash(
        &mut self,
        provider_env: archon_tools::provider_env::ProviderEnvSource,
    ) -> bool {
        let Some(bash) = self.tools.get("Bash") else {
            return false;
        };
        let Some(configured) = bash.with_provider_env_source(provider_env) else {
            return false;
        };
        self.replace(configured);
        true
    }

    /// Get all tool names.
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Keep only the tools whose names appear in the whitelist, plus
    /// [`ALWAYS_AVAILABLE_TOOLS`].
    ///
    /// The retention is here rather than at each caller because the caller is
    /// the wrong place to remember it. A whitelist applied at session start
    /// *deletes* tools from the registry, and a subagent's toolset is later
    /// taken from that same registry by name — so a tool dropped here could not
    /// be restored downstream however loudly the spawn asked for it. The
    /// subagent path unions the same names into its request and would silently
    /// get nothing back, because an intersection cannot produce what the source
    /// no longer holds.
    ///
    /// Use [`Self::filter_blacklist`] to refuse one of these deliberately;
    /// a denial still wins.
    pub fn filter_whitelist(&mut self, names: &[&str]) {
        self.tools
            .retain(|k, _| names.contains(&k.as_str()) || is_always_available(k));
    }

    /// Create a new registry containing only the tools whose names appear
    /// in `allowed`. Arc pointers are cloned (cheap ref-count bump).
    /// An empty `allowed` list produces an empty registry.
    pub fn clone_filtered(&self, allowed: &[&str]) -> Self {
        let filtered = self
            .tools
            .iter()
            .filter(|(name, _)| allowed.contains(&name.as_str()))
            .map(|(name, tool)| (name.clone(), Arc::clone(tool)))
            .collect();
        Self { tools: filtered }
    }

    /// Remove tools whose names appear in the blacklist.
    pub fn filter_blacklist(&mut self, names: &[&str]) {
        self.tools.retain(|k, _| !names.contains(&k.as_str()));
    }

    /// Get tool definitions for API request (JSON schemas).
    pub fn tool_definitions(&self) -> Vec<serde_json::Value> {
        let mut tools: Vec<_> = self.tools.iter().collect();
        tools.sort_by_key(|(left, _)| *left);
        tools
            .into_iter()
            .map(|(_, tool)| {
                serde_json::json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "input_schema": tool.input_schema(),
                })
            })
            .collect()
    }

}
