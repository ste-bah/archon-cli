use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Permission mode — controls how the agent handles tool approvals
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    Plan,
    Auto,
    DontAsk,
    BypassPermissions,
    AcceptEdits,
    Bubble,
}

impl PermissionMode {
    /// Returns the camelCase string representation used on the wire and in
    /// the shared `AgentConfig::permission_mode` mutex.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Plan => "plan",
            Self::Auto => "auto",
            Self::DontAsk => "dontAsk",
            Self::BypassPermissions => "bypassPermissions",
            Self::AcceptEdits => "acceptEdits",
            Self::Bubble => "bubble",
        }
    }

    /// Parse a permission mode string (case-sensitive, camelCase).
    /// Returns `None` for unrecognised values.
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "default" => Some(Self::Default),
            "plan" => Some(Self::Plan),
            "auto" => Some(Self::Auto),
            "dontAsk" => Some(Self::DontAsk),
            "bypassPermissions" => Some(Self::BypassPermissions),
            "acceptEdits" => Some(Self::AcceptEdits),
            "bubble" => Some(Self::Bubble),
            _ => None,
        }
    }
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Agent memory scope — where persistent memory is stored
// ---------------------------------------------------------------------------

/// Memory scope for an agent. `None` means NO persistent memory (not a
/// default to user scope). Memory must be explicitly set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentMemoryScope {
    /// ~/.archon/agent-memory/<agent_type>/
    User,
    /// .archon/agent-memory/<agent_type>/
    Project,
    /// .archon/agent-memory-local/<agent_type>/
    Local,
}

// ---------------------------------------------------------------------------
// Agent source — where the definition came from
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum AgentSource {
    /// User-level: ~/.archon/agents/custom/
    User,
    /// Project-level: .archon/agents/custom/
    Project,
    /// Compiled into the binary
    BuiltIn,
}

// ---------------------------------------------------------------------------
// Evolution tracking
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionEntry {
    pub version: String,
    pub timestamp: DateTime<Utc>,
    pub change_type: String,
    pub description: String,
}

// ---------------------------------------------------------------------------
// Quality metrics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentQuality {
    pub applied_rate: f64,
    pub completion_rate: f64,
}

impl Default for AgentQuality {
    fn default() -> Self {
        Self {
            applied_rate: 0.0,
            completion_rate: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Agent metadata (persisted in meta.json)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMeta {
    pub version: String,
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub invocation_count: u64,
    #[serde(default)]
    pub quality: AgentQuality,
    #[serde(default)]
    pub evolution_history: Vec<EvolutionEntry>,
    #[serde(default)]
    pub archived: bool,
}

impl Default for AgentMeta {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            version: "1.0".to_string(),
            created_at: now,
            updated_at: now,
            invocation_count: 0,
            quality: AgentQuality::default(),
            evolution_history: Vec::new(),
            archived: false,
        }
    }
}

// ---------------------------------------------------------------------------
// CustomAgentDefinition — 28 fields
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CustomAgentDefinition {
    // --- Core identity (fields 1-3) ---
    pub agent_type: String,
    pub system_prompt: String,
    pub description: String,

    // --- Tool control (fields 4-6) ---
    pub allowed_tools: Option<Vec<String>>,
    pub disallowed_tools: Option<Vec<String>>,
    pub tool_guidance: String,

    // --- Session overrides (fields 7-11) ---
    pub model: Option<String>,
    pub effort: Option<String>,
    pub max_turns: Option<u32>,
    pub permission_mode: Option<PermissionMode>,
    pub background: bool,

    // --- Interaction (fields 12-13) ---
    pub initial_prompt: Option<String>,
    pub color: Option<String>,

    // --- Memory + discovery (fields 14-17) ---
    pub memory_scope: Option<AgentMemoryScope>,
    pub recall_queries: Vec<String>,
    pub leann_queries: Vec<String>,
    pub tags: Vec<String>,

    // --- Provenance (fields 18-20) ---
    pub source: AgentSource,
    pub meta: AgentMeta,
    pub filename: Option<String>,
    pub base_dir: Option<String>,

    // --- Phase G: full Claude Code parity (fields 21-28) ---
    pub isolation: Option<String>,
    pub mcp_servers: Option<Vec<String>>,
    pub required_mcp_servers: Option<Vec<String>>,
    pub hooks: Option<serde_json::Value>,
    pub skills: Option<Vec<String>>,
    pub omit_claude_md: bool,
    pub critical_system_reminder: Option<String>,
}

impl CustomAgentDefinition {
    /// Check if all required MCP servers are available.
    ///
    /// Uses case-insensitive substring matching: a requirement of "github"
    /// matches an available server named "mcp__github__api".
    pub fn has_required_mcp_servers(&self, available: &[String]) -> bool {
        match &self.required_mcp_servers {
            None => true,
            Some(required) => required.iter().all(|pattern| {
                let lower = pattern.to_lowercase();
                available.iter().any(|s| s.to_lowercase().contains(&lower))
            }),
        }
    }
}

impl Default for CustomAgentDefinition {
    fn default() -> Self {
        Self {
            agent_type: String::new(),
            system_prompt: String::new(),
            description: String::new(),
            allowed_tools: None,
            disallowed_tools: None,
            tool_guidance: String::new(),
            model: None,
            effort: None,
            max_turns: None,
            permission_mode: None,
            background: false,
            initial_prompt: None,
            color: None,
            memory_scope: None,
            recall_queries: Vec::new(),
            leann_queries: Vec::new(),
            tags: Vec::new(),
            source: AgentSource::BuiltIn,
            meta: AgentMeta::default(),
            filename: None,
            base_dir: None,
            isolation: None,
            mcp_servers: None,
            required_mcp_servers: None,
            hooks: None,
            skills: None,
            omit_claude_md: false,
            critical_system_reminder: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_correct_field_values() {
        let def = CustomAgentDefinition::default();

        // Core identity
        assert!(def.agent_type.is_empty());
        assert!(def.system_prompt.is_empty());
        assert!(def.description.is_empty());

        // Tool control
        assert!(def.allowed_tools.is_none());
        assert!(def.disallowed_tools.is_none());
        assert!(def.tool_guidance.is_empty());

        // Session overrides
        assert!(def.model.is_none());
        assert!(def.effort.is_none());
        assert!(def.max_turns.is_none());
        assert!(def.permission_mode.is_none());
        assert!(!def.background);

        // Interaction
        assert!(def.initial_prompt.is_none());
        assert!(def.color.is_none());

        // Memory + discovery
        assert!(def.memory_scope.is_none()); // None = no memory, NOT default to user
        assert!(def.recall_queries.is_empty());
        assert!(def.leann_queries.is_empty());
        assert!(def.tags.is_empty());

        // Provenance
        assert_eq!(def.source, AgentSource::BuiltIn);
        assert!(def.filename.is_none());
        assert!(def.base_dir.is_none());

        // Phase G
        assert!(def.isolation.is_none());
        assert!(def.mcp_servers.is_none());
        assert!(def.required_mcp_servers.is_none());
        assert!(def.hooks.is_none());
        assert!(def.skills.is_none());
        assert!(!def.omit_claude_md);
        assert!(def.critical_system_reminder.is_none());
    }

    #[test]
    fn default_has_exactly_28_fields() {
        // Compile-time field count check via destructuring.
        // If a field is added or removed, this will fail to compile.
        let CustomAgentDefinition {
            agent_type: _,
            system_prompt: _,
            description: _,
            allowed_tools: _,
            disallowed_tools: _,
            tool_guidance: _,
            model: _,
            effort: _,
            max_turns: _,
            permission_mode: _,
            background: _,
            initial_prompt: _,
            color: _,
            memory_scope: _,
            recall_queries: _,
            leann_queries: _,
            tags: _,
            source: _,
            meta: _,
            filename: _,
            base_dir: _,
            isolation: _,
            mcp_servers: _,
            required_mcp_servers: _,
            hooks: _,
            skills: _,
            omit_claude_md: _,
            critical_system_reminder: _,
        } = CustomAgentDefinition::default();
    }

    #[test]
    fn agent_source_equality() {
        assert_eq!(AgentSource::User, AgentSource::User);
        assert_eq!(AgentSource::Project, AgentSource::Project);
        assert_eq!(AgentSource::BuiltIn, AgentSource::BuiltIn);
        assert_ne!(AgentSource::User, AgentSource::Project);
        assert_ne!(AgentSource::User, AgentSource::BuiltIn);
        assert_ne!(AgentSource::Project, AgentSource::BuiltIn);
    }

    #[test]
    fn agent_meta_serde_roundtrip() {
        let meta = AgentMeta::default();
        let json = serde_json::to_string(&meta).expect("serialize AgentMeta");
        let restored: AgentMeta = serde_json::from_str(&json).expect("deserialize AgentMeta");
        assert_eq!(restored.version, "1.0");
        assert_eq!(restored.invocation_count, 0);
        assert!(!restored.archived);
        assert!(restored.evolution_history.is_empty());
    }

    #[test]
    fn agent_quality_serde_roundtrip() {
        let q = AgentQuality {
            applied_rate: 0.85,
            completion_rate: 0.92,
        };
        let json = serde_json::to_string(&q).expect("serialize AgentQuality");
        let restored: AgentQuality = serde_json::from_str(&json).expect("deserialize AgentQuality");
        assert!((restored.applied_rate - 0.85).abs() < f64::EPSILON);
        assert!((restored.completion_rate - 0.92).abs() < f64::EPSILON);
    }

    #[test]
    fn evolution_entry_serde_roundtrip() {
        let entry = EvolutionEntry {
            version: "1.1".to_string(),
            timestamp: Utc::now(),
            change_type: "FIX".to_string(),
            description: "Fixed prompt clarity".to_string(),
        };
        let json = serde_json::to_string(&entry).expect("serialize EvolutionEntry");
        let restored: EvolutionEntry =
            serde_json::from_str(&json).expect("deserialize EvolutionEntry");
        assert_eq!(restored.version, "1.1");
        assert_eq!(restored.change_type, "FIX");
        assert_eq!(restored.description, "Fixed prompt clarity");
    }

    #[test]
    fn default_meta_values() {
        let meta = AgentMeta::default();
        assert_eq!(meta.version, "1.0");
        assert!(!meta.archived);
        assert_eq!(meta.invocation_count, 0);
        assert!(meta.evolution_history.is_empty());
        assert!((meta.quality.applied_rate - 0.0).abs() < f64::EPSILON);
        assert!((meta.quality.completion_rate - 0.0).abs() < f64::EPSILON);
    }

    // -----------------------------------------------------------------------
    // MCP scoping tests (AGT-018)
    // -----------------------------------------------------------------------

    #[test]
    fn has_required_mcp_no_requirements_always_true() {
        let def = CustomAgentDefinition::default();
        assert!(def.has_required_mcp_servers(&[]));
        assert!(def.has_required_mcp_servers(&["github".into()]));
    }

    #[test]
    fn has_required_mcp_satisfied() {
        let mut def = CustomAgentDefinition::default();
        def.required_mcp_servers = Some(vec!["github".into()]);
        assert!(def.has_required_mcp_servers(&["mcp__github__api".into()]));
    }

    #[test]
    fn has_required_mcp_not_satisfied() {
        let mut def = CustomAgentDefinition::default();
        def.required_mcp_servers = Some(vec!["github".into()]);
        assert!(!def.has_required_mcp_servers(&["slack".into()]));
    }

    #[test]
    fn has_required_mcp_case_insensitive() {
        let mut def = CustomAgentDefinition::default();
        def.required_mcp_servers = Some(vec!["GitHub".into()]);
        assert!(def.has_required_mcp_servers(&["mcp__github__api".into()]));
    }

    #[test]
    fn has_required_mcp_multiple_requirements() {
        let mut def = CustomAgentDefinition::default();
        def.required_mcp_servers = Some(vec!["github".into(), "slack".into()]);
        // Only github available, not slack
        assert!(!def.has_required_mcp_servers(&["mcp__github__api".into()]));
        // Both available
        assert!(def.has_required_mcp_servers(&["mcp__github__api".into(), "slack-server".into()]));
    }

    #[test]
    fn has_required_mcp_empty_requirements_always_true() {
        let mut def = CustomAgentDefinition::default();
        def.required_mcp_servers = Some(vec![]);
        assert!(def.has_required_mcp_servers(&[]));
    }

    // -----------------------------------------------------------------------
    // PermissionMode tests (AGT-001)
    // -----------------------------------------------------------------------

    #[test]
    fn permission_mode_serde_roundtrip_all_variants() {
        let variants = vec![
            (PermissionMode::Default, "\"default\""),
            (PermissionMode::Plan, "\"plan\""),
            (PermissionMode::Auto, "\"auto\""),
            (PermissionMode::DontAsk, "\"dontAsk\""),
            (PermissionMode::BypassPermissions, "\"bypassPermissions\""),
            (PermissionMode::AcceptEdits, "\"acceptEdits\""),
            (PermissionMode::Bubble, "\"bubble\""),
        ];
        for (variant, expected_json) in variants {
            let json = serde_json::to_string(&variant).expect("serialize PermissionMode");
            assert_eq!(json, expected_json, "serialized {:?}", variant);
            let restored: PermissionMode =
                serde_json::from_str(&json).expect("deserialize PermissionMode");
            assert_eq!(restored, variant);
        }
    }

    #[test]
    fn permission_mode_has_exactly_7_variants() {
        // Exhaustive match ensures compile-time check for variant count
        let mode = PermissionMode::Default;
        match mode {
            PermissionMode::Default => {}
            PermissionMode::Plan => {}
            PermissionMode::Auto => {}
            PermissionMode::DontAsk => {}
            PermissionMode::BypassPermissions => {}
            PermissionMode::AcceptEdits => {}
            PermissionMode::Bubble => {}
        }
    }

    #[test]
    fn permission_mode_equality() {
        assert_eq!(PermissionMode::Default, PermissionMode::Default);
        assert_ne!(PermissionMode::Default, PermissionMode::Auto);
        assert_ne!(PermissionMode::DontAsk, PermissionMode::BypassPermissions);
    }

    // -----------------------------------------------------------------------
    // AgentMemoryScope tests (AGT-001)
    // -----------------------------------------------------------------------

    #[test]
    fn memory_scope_serde_roundtrip_all_variants() {
        let variants = vec![
            (AgentMemoryScope::User, "\"user\""),
            (AgentMemoryScope::Project, "\"project\""),
            (AgentMemoryScope::Local, "\"local\""),
        ];
        for (variant, expected_json) in variants {
            let json = serde_json::to_string(&variant).expect("serialize AgentMemoryScope");
            assert_eq!(json, expected_json, "serialized {:?}", variant);
            let restored: AgentMemoryScope =
                serde_json::from_str(&json).expect("deserialize AgentMemoryScope");
            assert_eq!(restored, variant);
        }
    }

    #[test]
    fn memory_scope_none_means_no_memory() {
        // AC: memory_scope: None = no persistent memory (NOT a default to user scope)
        let def = CustomAgentDefinition::default();
        assert!(
            def.memory_scope.is_none(),
            "Default memory_scope must be None (no memory), not Some(User)"
        );
    }

    #[test]
    fn memory_scope_equality() {
        assert_eq!(AgentMemoryScope::User, AgentMemoryScope::User);
        assert_ne!(AgentMemoryScope::User, AgentMemoryScope::Project);
        assert_ne!(AgentMemoryScope::Project, AgentMemoryScope::Local);
    }

    // -----------------------------------------------------------------------
    // Typed permission_mode field test (AGT-001)
    // -----------------------------------------------------------------------

    #[test]
    fn definition_permission_mode_accepts_typed_enum() {
        let mut def = CustomAgentDefinition::default();
        def.permission_mode = Some(PermissionMode::Auto);
        assert_eq!(def.permission_mode, Some(PermissionMode::Auto));
        def.permission_mode = Some(PermissionMode::Bubble);
        assert_eq!(def.permission_mode, Some(PermissionMode::Bubble));
    }

    #[test]
    fn agent_meta_deserialize_from_meta_json_format() {
        // Simulate what meta.json looks like on disk (PRD Section 3.3)
        let json = r#"{
            "version": "2.0",
            "created_at": "2026-04-01T12:00:00Z",
            "updated_at": "2026-04-08T09:30:00Z",
            "invocation_count": 42,
            "quality": { "applied_rate": 0.95, "completion_rate": 0.88 },
            "evolution_history": [
                {
                    "version": "1.0",
                    "timestamp": "2026-04-01T12:00:00Z",
                    "change_type": "CAPTURED",
                    "description": "Initial creation"
                }
            ],
            "archived": false
        }"#;
        let meta: AgentMeta = serde_json::from_str(json).expect("deserialize meta.json");
        assert_eq!(meta.version, "2.0");
        assert_eq!(meta.invocation_count, 42);
        assert!((meta.quality.applied_rate - 0.95).abs() < f64::EPSILON);
        assert_eq!(meta.evolution_history.len(), 1);
        assert_eq!(meta.evolution_history[0].change_type, "CAPTURED");
    }
}
