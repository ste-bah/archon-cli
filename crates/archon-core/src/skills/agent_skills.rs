use super::{Skill, SkillContext, SkillOutput};

/// Validate that an agent name is safe (no path traversal).
fn is_valid_agent_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("..")
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

/// /create-agent — Generate a custom agent from a natural language description.
///
/// Returns a SkillOutput::Prompt that instructs the parent agent to create
/// the 6-file agent definition directory and hot-reload the registry.
pub struct CreateAgentSkill;

impl Skill for CreateAgentSkill {
    fn name(&self) -> &str {
        "create-agent"
    }

    fn description(&self) -> &str {
        "Create a custom agent from a natural language description"
    }

    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
        let description = args.join(" ");
        if description.trim().is_empty() {
            return SkillOutput::Error(
                "Usage: /create-agent <description>\n\nExample: /create-agent A code review specialist that checks for security issues".into(),
            );
        }

        // Derive a kebab-case agent name from the first few words
        let agent_name = derive_agent_name(&description);
        let agent_dir = ctx
            .working_dir
            .join(".archon")
            .join("agents")
            .join("custom")
            .join(&agent_name);

        // Return a prompt that instructs the agent to create the files
        let prompt = format!(
            r#"Create a custom agent definition based on this description: "{description}"

The agent name is: {agent_name}
The agent directory is: {agent_dir}

Create ALL 6 files in that directory:

1. `agent.md` — YAML frontmatter with name, description, model (default: sonnet). Body is the system prompt.
2. `behavior.md` — Behavioral rules for the agent.
3. `context.md` — Additional context the agent should know.
4. `tools.md` — YAML frontmatter with allowed/disallowed tool lists.
5. `memory-keys.json` — JSON array of memory key strings the agent uses (can be empty `[]`).
6. `meta.json` — JSON with version, created_at, updated_at, invocation_count, quality, evolution_history, archived fields.

Use these exact formats:

agent.md:
```
---
name: {agent_name}
description: <one-line description>
model: sonnet
---
<system prompt for the agent>
```

meta.json:
```json
{{
  "version": "1.0",
  "created_at": "<ISO-8601 now>",
  "updated_at": "<ISO-8601 now>",
  "invocation_count": 0,
  "quality": {{"applied_rate": 0.0, "completion_rate": 0.0}},
  "evolution_history": [],
  "archived": false
}}
```

tools.md:
```
---
allowed: [Read, Grep, Glob, Bash, Write, Edit]
---
```

After creating all 6 files, confirm the agent was created successfully.
The agent will be available immediately after creation (hot-reloaded on next registry access)."#,
            agent_dir = agent_dir.display(),
        );

        SkillOutput::Prompt(prompt)
    }
}

/// Derive a kebab-case agent name from a description.
fn derive_agent_name(description: &str) -> String {
    let words: Vec<&str> = description
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .take(3)
        .collect();

    if words.is_empty() {
        return "custom-agent".into();
    }

    let segments: Vec<String> = words
        .iter()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|s| !s.is_empty())
        .collect();

    if segments.is_empty() {
        return "custom-agent".into();
    }

    segments.join("-")
}

// ---------------------------------------------------------------------------
// /list-agents — Display all loaded agents from the registry
// ---------------------------------------------------------------------------

pub struct ListAgentsSkill;

impl Skill for ListAgentsSkill {
    fn name(&self) -> &str {
        "list-agents"
    }

    fn description(&self) -> &str {
        "List all available agents with name, description, source, and model"
    }

    fn aliases(&self) -> Vec<&str> {
        vec!["agents"]
    }

    fn execute(&self, _args: &[String], ctx: &SkillContext) -> SkillOutput {
        let registry = match ctx.agent_registry.as_ref() {
            Some(r) => r,
            None => return SkillOutput::Error("Agent registry not available.".into()),
        };

        let reg = match registry.read() {
            Ok(r) => r,
            Err(_) => return SkillOutput::Error("Failed to read agent registry.".into()),
        };

        let agents = reg.list();
        if agents.is_empty() {
            return SkillOutput::Text("No agents loaded.".into());
        }

        let mut out = format!("{} agents loaded:\n\n", agents.len());
        for def in &agents {
            let model = def.model.as_deref().unwrap_or("default");
            let source = match def.source {
                crate::agents::AgentSource::BuiltIn => "built-in",
                crate::agents::AgentSource::Project => "project",
                crate::agents::AgentSource::User => "user",
            };
            let path = def.base_dir.as_deref().unwrap_or("built-in");
            out.push_str(&format!(
                "  {:<24} {:<40} [{source}, {model}]\n                         {path}\n",
                def.agent_type, def.description
            ));
        }
        SkillOutput::Text(out)
    }
}

// ---------------------------------------------------------------------------
// /run-agent — Spawn a subagent with a specific agent type
// ---------------------------------------------------------------------------

pub struct RunAgentSkill;

impl Skill for RunAgentSkill {
    fn name(&self) -> &str {
        "run-agent"
    }

    fn description(&self) -> &str {
        "Invoke a custom agent with a task description"
    }

    fn execute(&self, args: &[String], _ctx: &SkillContext) -> SkillOutput {
        if args.is_empty() {
            return SkillOutput::Error(
                "Usage: /run-agent <agent-name> <task description>\n\nExample: /run-agent code-reviewer Review the auth module for security issues".into(),
            );
        }

        let agent_name = &args[0];
        if !is_valid_agent_name(agent_name) {
            return SkillOutput::Error(format!("Invalid agent name: {agent_name}"));
        }
        let task = if args.len() > 1 {
            args[1..].join(" ")
        } else {
            return SkillOutput::Error(
                "Usage: /run-agent <agent-name> <task description>\n\nPlease provide a task for the agent.".into(),
            );
        };

        let prompt = format!(
            r#"Use the Agent tool to spawn a subagent with the following parameters:
- subagent_type: "{agent_name}"
- prompt: "{task}"

Spawn the agent now and report its results."#
        );

        SkillOutput::Prompt(prompt)
    }
}

// ---------------------------------------------------------------------------
// /adjust-behavior — Modify an agent's behavior.md
// ---------------------------------------------------------------------------

pub struct AdjustBehaviorSkill;

impl Skill for AdjustBehaviorSkill {
    fn name(&self) -> &str {
        "adjust-behavior"
    }

    fn description(&self) -> &str {
        "Modify behavioral rules for a custom agent"
    }

    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
        if args.is_empty() {
            return SkillOutput::Error(
                "Usage: /adjust-behavior <agent-name> <change description>\n\nExample: /adjust-behavior code-reviewer Add a rule to always check for SQL injection".into(),
            );
        }

        let agent_name = &args[0];
        if !is_valid_agent_name(agent_name) {
            return SkillOutput::Error(format!("Invalid agent name: {agent_name}"));
        }
        let change = if args.len() > 1 {
            args[1..].join(" ")
        } else {
            return SkillOutput::Error("Please describe the behavioral change to apply.".into());
        };

        let behavior_path = ctx
            .working_dir
            .join(".archon/agents/custom")
            .join(agent_name)
            .join("behavior.md");

        let prompt = format!(
            r#"Read the agent's behavior file at: {path}

Apply this change: "{change}"

Merge the change into the existing behavior rules. Do NOT replace the entire file — add to or modify only the relevant rules. Write the updated file back.

After updating, confirm what changed."#,
            path = behavior_path.display()
        );

        SkillOutput::Prompt(prompt)
    }
}

// ---------------------------------------------------------------------------
// /evolve-agent — Apply FIX/DERIVED/CAPTURED evolution
// ---------------------------------------------------------------------------

pub struct EvolveAgentSkill;

impl Skill for EvolveAgentSkill {
    fn name(&self) -> &str {
        "evolve-agent"
    }

    fn description(&self) -> &str {
        "Apply evolution suggestions (FIX, DERIVED, CAPTURED) to an agent"
    }

    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
        if args.len() < 2 {
            return SkillOutput::Error(
                "Usage: /evolve-agent <agent-name> <FIX|DERIVED|CAPTURED> [description]\n\nFIX: In-place repair\nDERIVED: Create variant\nCAPTURED: Extract from success".into(),
            );
        }

        let agent_name = &args[0];
        if !is_valid_agent_name(agent_name) {
            return SkillOutput::Error(format!("Invalid agent name: {agent_name}"));
        }
        let evolution_type = args[1].to_uppercase();
        let description = if args.len() > 2 {
            args[2..].join(" ")
        } else {
            String::new()
        };

        if !["FIX", "DERIVED", "CAPTURED"].contains(&evolution_type.as_str()) {
            return SkillOutput::Error(format!(
                "Unknown evolution type: {evolution_type}. Use FIX, DERIVED, or CAPTURED."
            ));
        }

        let agent_dir = ctx
            .working_dir
            .join(".archon/agents/custom")
            .join(agent_name);

        let prompt = format!(
            r#"Evolve agent "{agent_name}" with type: {evolution_type}
Description: {description}

Agent directory: {agent_dir}

Steps:
1. Read meta.json from the agent directory
2. Read the current agent.md and behavior.md
3. Apply the evolution:
   - FIX: Repair the agent definition in-place based on the description
   - DERIVED: Create a new variant agent (copy to a new directory with modified name)
   - CAPTURED: Extract successful patterns into the agent definition
4. Update meta.json:
   - Increment version (e.g., "1.0" -> "1.1" for FIX, "2.0" for DERIVED)
   - Update updated_at to current ISO-8601 timestamp
   - Add entry to evolution_history: {{"type": "{evolution_type}", "description": "{description}", "timestamp": "<now>"}}
5. Write all modified files back

Confirm the evolution was applied."#,
            agent_dir = agent_dir.display()
        );

        SkillOutput::Prompt(prompt)
    }
}

// ---------------------------------------------------------------------------
// /archive-agent — Move an agent to _archived/
// ---------------------------------------------------------------------------

pub struct ArchiveAgentSkill;

impl Skill for ArchiveAgentSkill {
    fn name(&self) -> &str {
        "archive-agent"
    }

    fn description(&self) -> &str {
        "Archive or restore a custom agent"
    }

    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
        if args.is_empty() {
            return SkillOutput::Error(
                "Usage: /archive-agent <agent-name> [--restore]\n\nArchives an agent (moves to _archived/) or restores it.".into(),
            );
        }

        let agent_name = &args[0];
        if !is_valid_agent_name(agent_name) {
            return SkillOutput::Error(format!("Invalid agent name: {agent_name}"));
        }
        let restore = args.iter().any(|a| a == "--restore");

        let custom_dir = ctx.working_dir.join(".archon/agents/custom");
        let archived_dir = ctx.working_dir.join(".archon/agents/_archived");

        if restore {
            let prompt = format!(
                r#"Restore archived agent "{agent_name}":
1. Move directory from {archived}/{agent_name} to {custom}/{agent_name}
2. Read meta.json and set "archived": false
3. Write updated meta.json
4. Confirm the agent was restored."#,
                archived = archived_dir.display(),
                custom = custom_dir.display()
            );
            SkillOutput::Prompt(prompt)
        } else {
            let prompt = format!(
                r#"Archive agent "{agent_name}":
1. Create directory {archived} if it doesn't exist
2. Move directory from {custom}/{agent_name} to {archived}/{agent_name}
3. Read meta.json and set "archived": true
4. Write updated meta.json
5. Confirm the agent was archived."#,
                archived = archived_dir.display(),
                custom = custom_dir.display()
            );
            SkillOutput::Prompt(prompt)
        }
    }
}

// ---------------------------------------------------------------------------
// /agent-history — Display evolution history from meta.json
// ---------------------------------------------------------------------------

pub struct AgentHistorySkill;

impl Skill for AgentHistorySkill {
    fn name(&self) -> &str {
        "agent-history"
    }

    fn description(&self) -> &str {
        "Show version history and evolution lineage for a custom agent"
    }

    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
        if args.is_empty() {
            return SkillOutput::Error("Usage: /agent-history <agent-name>".into());
        }

        let agent_name = &args[0];
        if !is_valid_agent_name(agent_name) {
            return SkillOutput::Error(format!("Invalid agent name: {agent_name}"));
        }
        let meta_path = ctx
            .working_dir
            .join(".archon/agents/custom")
            .join(agent_name)
            .join("meta.json");

        // Read meta.json directly — this is a read-only operation
        let content = match std::fs::read_to_string(&meta_path) {
            Ok(c) => c,
            Err(_) => {
                return SkillOutput::Error(format!(
                    "Agent '{agent_name}' not found or meta.json missing at {}",
                    meta_path.display()
                ));
            }
        };

        let meta: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                return SkillOutput::Error(format!("Failed to parse meta.json: {e}"));
            }
        };

        let version = meta.get("version").and_then(|v| v.as_str()).unwrap_or("?");
        let created = meta
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let updated = meta
            .get("updated_at")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let invocations = meta
            .get("invocation_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let archived = meta
            .get("archived")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut out = format!(
            "Agent: {agent_name}\nVersion: {version}\nCreated: {created}\nUpdated: {updated}\nInvocations: {invocations}\nArchived: {archived}\n"
        );

        if let Some(history) = meta.get("evolution_history").and_then(|v| v.as_array()) {
            if history.is_empty() {
                out.push_str("\nNo evolution history.\n");
            } else {
                out.push_str(&format!(
                    "\nEvolution history ({} entries):\n",
                    history.len()
                ));
                for (i, entry) in history.iter().enumerate() {
                    let etype = entry.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                    let desc = entry
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let ts = entry
                        .get("timestamp")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    out.push_str(&format!("  {}. [{etype}] {desc} ({ts})\n", i + 1));
                }
            }
        }

        SkillOutput::Text(out)
    }
}

// ---------------------------------------------------------------------------
// /rollback-behavior — Restore behavior.md from .versions/
// ---------------------------------------------------------------------------

pub struct RollbackBehaviorSkill;

impl Skill for RollbackBehaviorSkill {
    fn name(&self) -> &str {
        "rollback-behavior"
    }

    fn description(&self) -> &str {
        "Rollback agent behavioral rules to a previous version"
    }

    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
        if args.is_empty() {
            return SkillOutput::Error(
                "Usage: /rollback-behavior <agent-name> [version]\n\nRolls back behavior.md to a previous version from .versions/ directory.".into(),
            );
        }

        let agent_name = &args[0];
        if !is_valid_agent_name(agent_name) {
            return SkillOutput::Error(format!("Invalid agent name: {agent_name}"));
        }
        let version = args.get(1).map(|s| s.as_str());
        let agent_dir = ctx
            .working_dir
            .join(".archon/agents/custom")
            .join(agent_name);

        let prompt = if let Some(ver) = version {
            format!(
                r#"Rollback behavior for agent "{agent_name}" to version {ver}:

Agent directory: {agent_dir}

1. Read {agent_dir}/.versions/behavior-v{ver}.md
2. Back up current behavior.md to .versions/behavior-v<current>.md
3. Replace behavior.md with the version {ver} content
4. Update meta.json: add evolution_history entry with type "ROLLBACK"
5. Confirm the rollback."#,
                agent_dir = agent_dir.display()
            )
        } else {
            format!(
                r#"Rollback behavior for agent "{agent_name}" to the most recent previous version:

Agent directory: {agent_dir}

1. List files in {agent_dir}/.versions/ to find behavior versions
2. If no versions exist, report that no rollback is possible
3. Back up current behavior.md to .versions/behavior-v<current>.md
4. Replace behavior.md with the most recent previous version
5. Update meta.json: add evolution_history entry with type "ROLLBACK"
6. Confirm the rollback."#,
                agent_dir = agent_dir.display()
            )
        };

        SkillOutput::Prompt(prompt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_name_from_description() {
        assert_eq!(
            derive_agent_name("A code review specialist"),
            "code-review-specialist"
        );
    }

    #[test]
    fn derive_name_skips_short_words() {
        assert_eq!(derive_agent_name("a bug fix helper"), "bug-fix-helper");
    }

    #[test]
    fn derive_name_limits_to_3_words() {
        assert_eq!(
            derive_agent_name("security vulnerability scanner for web apps"),
            "security-vulnerability-scanner"
        );
    }

    #[test]
    fn derive_name_empty_returns_default() {
        assert_eq!(derive_agent_name(""), "custom-agent");
        assert_eq!(derive_agent_name("a b c"), "custom-agent");
    }

    #[test]
    fn derive_name_strips_special_chars() {
        assert_eq!(
            derive_agent_name("code-review! testing#agent"),
            "codereview-testingagent"
        );
    }

    #[test]
    fn create_agent_empty_args_returns_error() {
        let skill = CreateAgentSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&[], &ctx);
        assert!(matches!(output, SkillOutput::Error(_)));
    }

    #[test]
    fn create_agent_returns_prompt() {
        let skill = CreateAgentSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp/project"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&["code".into(), "reviewer".into()], &ctx);
        match output {
            SkillOutput::Prompt(p) => {
                assert!(p.contains("code-reviewer"));
                assert!(p.contains("agent.md"));
                assert!(p.contains("meta.json"));
                assert!(p.contains("behavior.md"));
                assert!(p.contains("context.md"));
                assert!(p.contains("tools.md"));
                assert!(p.contains("memory-keys.json"));
            }
            other => panic!("expected Prompt, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Agent name validation tests
    // -----------------------------------------------------------------------

    #[test]
    fn valid_agent_names() {
        assert!(is_valid_agent_name("code-reviewer"));
        assert!(is_valid_agent_name("my-agent-v2"));
        assert!(is_valid_agent_name("test"));
    }

    #[test]
    fn invalid_agent_names() {
        assert!(!is_valid_agent_name(""));
        assert!(!is_valid_agent_name("../etc"));
        assert!(!is_valid_agent_name("foo/bar"));
        assert!(!is_valid_agent_name("foo\\bar"));
        assert!(!is_valid_agent_name(".."));
    }

    // -----------------------------------------------------------------------
    // /list-agents tests
    // -----------------------------------------------------------------------

    #[test]
    fn list_agents_no_registry_returns_error() {
        let skill = ListAgentsSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&[], &ctx);
        assert!(matches!(output, SkillOutput::Error(_)));
    }

    #[test]
    fn list_agents_empty_registry_returns_text() {
        use std::sync::{Arc, RwLock};
        let reg = Arc::new(RwLock::new(crate::agents::AgentRegistry::empty()));
        let skill = ListAgentsSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: Some(reg),
        };
        let output = skill.execute(&[], &ctx);
        match output {
            SkillOutput::Text(t) => assert!(t.contains("No agents")),
            other => panic!("expected Text, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // /run-agent tests
    // -----------------------------------------------------------------------

    #[test]
    fn run_agent_no_args_returns_error() {
        let skill = RunAgentSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&[], &ctx);
        assert!(matches!(output, SkillOutput::Error(_)));
    }

    #[test]
    fn run_agent_no_task_returns_error() {
        let skill = RunAgentSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&["code-reviewer".into()], &ctx);
        assert!(matches!(output, SkillOutput::Error(_)));
    }

    #[test]
    fn run_agent_returns_prompt_with_agent_name() {
        let skill = RunAgentSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(
            &[
                "code-reviewer".into(),
                "Review".into(),
                "auth".into(),
                "module".into(),
            ],
            &ctx,
        );
        match output {
            SkillOutput::Prompt(p) => {
                assert!(p.contains("code-reviewer"));
                assert!(p.contains("Review auth module"));
            }
            other => panic!("expected Prompt, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // /adjust-behavior tests
    // -----------------------------------------------------------------------

    #[test]
    fn adjust_behavior_no_args_returns_error() {
        let skill = AdjustBehaviorSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&[], &ctx);
        assert!(matches!(output, SkillOutput::Error(_)));
    }

    #[test]
    fn adjust_behavior_returns_prompt() {
        let skill = AdjustBehaviorSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/home/user/proj"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(
            &[
                "my-agent".into(),
                "Add".into(),
                "SQL".into(),
                "injection".into(),
                "check".into(),
            ],
            &ctx,
        );
        match output {
            SkillOutput::Prompt(p) => {
                assert!(p.contains("behavior.md"));
                assert!(p.contains("my-agent"));
                assert!(p.contains("Add SQL injection check"));
            }
            other => panic!("expected Prompt, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // /evolve-agent tests
    // -----------------------------------------------------------------------

    #[test]
    fn evolve_agent_too_few_args_returns_error() {
        let skill = EvolveAgentSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&["my-agent".into()], &ctx);
        assert!(matches!(output, SkillOutput::Error(_)));
    }

    #[test]
    fn evolve_agent_invalid_type_returns_error() {
        let skill = EvolveAgentSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&["my-agent".into(), "INVALID".into()], &ctx);
        match output {
            SkillOutput::Error(e) => assert!(e.contains("INVALID")),
            other => panic!("expected Error, got: {:?}", other),
        }
    }

    #[test]
    fn evolve_agent_fix_returns_prompt() {
        let skill = EvolveAgentSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(
            &[
                "my-agent".into(),
                "fix".into(),
                "improve".into(),
                "accuracy".into(),
            ],
            &ctx,
        );
        match output {
            SkillOutput::Prompt(p) => {
                assert!(p.contains("FIX"));
                assert!(p.contains("my-agent"));
                assert!(p.contains("meta.json"));
            }
            other => panic!("expected Prompt, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // /archive-agent tests
    // -----------------------------------------------------------------------

    #[test]
    fn archive_agent_no_args_returns_error() {
        let skill = ArchiveAgentSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&[], &ctx);
        assert!(matches!(output, SkillOutput::Error(_)));
    }

    #[test]
    fn archive_agent_returns_archive_prompt() {
        let skill = ArchiveAgentSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&["old-agent".into()], &ctx);
        match output {
            SkillOutput::Prompt(p) => {
                assert!(p.contains("Archive"));
                assert!(p.contains("old-agent"));
                assert!(p.contains("_archived"));
            }
            other => panic!("expected Prompt, got: {:?}", other),
        }
    }

    #[test]
    fn archive_agent_restore_returns_restore_prompt() {
        let skill = ArchiveAgentSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&["old-agent".into(), "--restore".into()], &ctx);
        match output {
            SkillOutput::Prompt(p) => {
                assert!(p.contains("Restore"));
                assert!(p.contains("old-agent"));
            }
            other => panic!("expected Prompt, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // /agent-history tests
    // -----------------------------------------------------------------------

    #[test]
    fn agent_history_no_args_returns_error() {
        let skill = AgentHistorySkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&[], &ctx);
        assert!(matches!(output, SkillOutput::Error(_)));
    }

    #[test]
    fn agent_history_missing_agent_returns_error() {
        let skill = AgentHistorySkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&["nonexistent-agent".into()], &ctx);
        assert!(matches!(output, SkillOutput::Error(_)));
    }

    // -----------------------------------------------------------------------
    // /rollback-behavior tests
    // -----------------------------------------------------------------------

    #[test]
    fn rollback_behavior_no_args_returns_error() {
        let skill = RollbackBehaviorSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&[], &ctx);
        assert!(matches!(output, SkillOutput::Error(_)));
    }

    #[test]
    fn rollback_behavior_with_version_returns_prompt() {
        let skill = RollbackBehaviorSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&["my-agent".into(), "1.0".into()], &ctx);
        match output {
            SkillOutput::Prompt(p) => {
                assert!(p.contains("my-agent"));
                assert!(p.contains("version 1.0"));
                assert!(p.contains(".versions"));
            }
            other => panic!("expected Prompt, got: {:?}", other),
        }
    }

    #[test]
    fn rollback_behavior_no_version_returns_prompt() {
        let skill = RollbackBehaviorSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/tmp"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&["my-agent".into()], &ctx);
        match output {
            SkillOutput::Prompt(p) => {
                assert!(p.contains("my-agent"));
                assert!(p.contains("most recent"));
            }
            other => panic!("expected Prompt, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // /create-agent tests (existing)
    // -----------------------------------------------------------------------

    #[test]
    fn create_agent_includes_correct_dir() {
        let skill = CreateAgentSkill;
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::path::PathBuf::from("/home/user/project"),
            model: "sonnet".into(),
            agent_registry: None,
        };
        let output = skill.execute(&["security".into(), "scanner".into()], &ctx);
        match output {
            SkillOutput::Prompt(p) => {
                assert!(p.contains("/home/user/project/.archon/agents/custom/security-scanner"));
            }
            other => panic!("expected Prompt, got: {:?}", other),
        }
    }
}
