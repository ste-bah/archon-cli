pub mod agent_skills;
pub mod builtin;
pub mod discovery;
pub mod expanded;
pub mod parser;
pub mod skill_tool;
pub mod templates;
pub mod to_prd;
pub mod prd_to_spec;

use std::collections::HashMap;

/// Output produced by a skill execution.
#[derive(Debug, Clone)]
pub enum SkillOutput {
    /// Display text directly in the TUI output pane — NOT sent to the agent.
    Text(String),
    /// Display markdown directly in the TUI output pane — NOT sent to the agent.
    Markdown(String),
    /// Inject this string into the conversation as a user message and send to the agent.
    /// Equivalent to Claude Code's `PromptCommand` / `getPromptForCommand()`.
    Prompt(String),
    Error(String),
}

/// Contextual information passed to skill execution.
pub struct SkillContext {
    pub session_id: String,
    pub working_dir: std::path::PathBuf,
    pub model: String,
    /// Agent registry for agent management skills (/create-agent, /run-agent, etc.).
    pub agent_registry: Option<std::sync::Arc<std::sync::RwLock<crate::agents::AgentRegistry>>>,
}

/// A skill that can be invoked via a slash command.
pub trait Skill: Send + Sync {
    /// Canonical name of the skill (without leading `/`).
    fn name(&self) -> &str;

    /// Short human-readable description.
    fn description(&self) -> &str;

    /// Optional aliases that also resolve to this skill.
    fn aliases(&self) -> Vec<&str> {
        vec![]
    }

    /// Execute the skill with the given arguments and context.
    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput;
}

/// Registry that maps command names and aliases to [`Skill`] implementations.
pub struct SkillRegistry {
    skills: HashMap<String, Box<dyn Skill>>,
    aliases: HashMap<String, String>, // alias -> canonical name
}

impl SkillRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
            aliases: HashMap::new(),
        }
    }

    /// Register a skill. Also registers any aliases declared by the skill.
    pub fn register(&mut self, skill: Box<dyn Skill>) {
        let name = skill.name().to_string();
        for alias in skill.aliases() {
            self.aliases.insert(alias.to_string(), name.clone());
        }
        self.skills.insert(name, skill);
    }

    /// Register an additional alias that maps to an existing skill name.
    pub fn register_alias(&mut self, alias: &str, name: &str) {
        self.aliases.insert(alias.to_string(), name.to_string());
    }

    /// Look up a skill by its canonical name only.
    pub fn get(&self, name: &str) -> Option<&dyn Skill> {
        self.skills.get(name).map(|b| b.as_ref())
    }

    /// Resolve a name or alias to the corresponding skill.
    pub fn resolve(&self, name_or_alias: &str) -> Option<&dyn Skill> {
        if let Some(skill) = self.skills.get(name_or_alias) {
            return Some(skill.as_ref());
        }
        if let Some(canonical) = self.aliases.get(name_or_alias) {
            return self.skills.get(canonical).map(|b| b.as_ref());
        }
        None
    }

    /// Return a list of `(name, description)` pairs for all registered skills,
    /// sorted alphabetically by name.
    pub fn list_all(&self) -> Vec<(&str, &str)> {
        let mut list: Vec<(&str, &str)> = self
            .skills
            .values()
            .map(|s| (s.name(), s.description()))
            .collect();
        list.sort_by_key(|(name, _)| *name);
        list
    }

    /// Format a help summary listing all registered skills.
    pub fn format_help(&self) -> String {
        let entries = self.list_all();
        let mut out = String::from("Available commands:\n\n");
        for (name, desc) in &entries {
            out.push_str(&format!("  /{name:<16} {desc}\n"));
        }
        out
    }

    /// Format detailed help for a single skill, or `None` if not found.
    pub fn format_skill_help(&self, name: &str) -> Option<String> {
        let skill = self.resolve(name)?;
        let mut out = format!("/{}\n\n{}\n", skill.name(), skill.description());
        let aliases = skill.aliases();
        if !aliases.is_empty() {
            out.push_str(&format!(
                "\nAliases: {}\n",
                aliases
                    .iter()
                    .map(|a| format!("/{a}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Some(out)
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}
