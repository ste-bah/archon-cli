use crate::cli_args::Cli;
use crate::setup::strip_cache_control_if_disabled;
use archon_consciousness::assembler::{AssemblyInput, BudgetConfig, SystemPromptAssembler};
use archon_core::reasoning::build_environment_section;
use archon_llm::identity::IdentityProvider;
use archon_memory::MemoryTrait;

pub(super) fn build_system_prompt(
    config: &archon_core::config::ArchonConfig,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    cli: &Cli,
    working_dir: &std::path::Path,
    identity: &IdentityProvider,
    agent_def: Option<&archon_core::agents::definition::CustomAgentDefinition>,
    inject_output_style: bool,
) -> Vec<serde_json::Value> {
    let archon_md = if resolved_flags.bare_mode {
        String::new()
    } else {
        archon_core::archonmd::load_hierarchical_archon_md_with_limit(
            working_dir,
            config.context.archonmd_max_tokens as usize,
        )
    };
    let git_info = archon_core::git::detect_git_info(working_dir);
    let git_branch = git_info.as_ref().map(|g| g.branch.as_str());
    let env_section = build_environment_section(working_dir, git_branch);
    let mut identity_blocks = identity.system_prompt_blocks("", &archon_md, &env_section);
    strip_cache_control_if_disabled(&mut identity_blocks, config.context.prompt_cache);

    let mut prompt = if let Some(def) = agent_def {
        vec![serde_json::json!({
            "type": "text",
            "text": agent_prompt_text(def),
        })]
    } else {
        identity_blocks
    };

    if inject_output_style
        && let Some(text) = output_style_prompt(cli, config)
    {
        prompt.push(serde_json::json!({ "type": "text", "text": text }));
    }
    prompt
}

pub(super) fn build_interactive_system_prompt(
    config: &archon_core::config::ArchonConfig,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    cli: &Cli,
    working_dir: &std::path::Path,
    session_id: &str,
    identity: &IdentityProvider,
    agent_def: Option<&archon_core::agents::definition::CustomAgentDefinition>,
    memory: &dyn MemoryTrait,
) -> Vec<serde_json::Value> {
    let archon_md = if resolved_flags.bare_mode {
        tracing::info!("bare mode: skipping ARCHON.md loading");
        String::new()
    } else {
        archon_core::archonmd::load_hierarchical_archon_md_with_limit(
            working_dir,
            config.context.archonmd_max_tokens as usize,
        )
    };
    let git_info = archon_core::git::detect_git_info(working_dir);
    let git_branch = git_info.as_ref().map(|g| g.branch.as_str());
    let env_section = build_environment_section(working_dir, git_branch);

    let identity_blocks = identity.system_prompt_blocks("", &archon_md, &env_section);
    let identity_text = identity_blocks
        .iter()
        .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join("\n\n");

    let personality_text = config.personality.to_prompt_text();
    let rules_text = archon_consciousness::rules::RulesEngine::new(memory)
        .format_for_prompt()
        .unwrap_or_default();

    let assembler = SystemPromptAssembler::new(BudgetConfig::default());
    let input = AssemblyInput {
        identity: if identity_text.is_empty() {
            None
        } else {
            Some(identity_text)
        },
        personality: if agent_def.is_some() || resolved_flags.bare_mode {
            None
        } else {
            Some(personality_text)
        },
        rules: if rules_text.is_empty() {
            None
        } else {
            Some(rules_text)
        },
        memories: None,
        user_prompt: None,
        project_instructions: if archon_md.is_empty() {
            None
        } else {
            Some(archon_md.clone())
        },
        environment: if env_section.is_empty() {
            None
        } else {
            Some(env_section.clone())
        },
        inner_voice: None,
        personality_briefing: agent_def.map(agent_prompt_text),
        memory_briefing: None,
        dynamic: Some(format!(
            "Date: {}\nSession: {}\n\n\
            ## Memory System\n\
            You have a persistent memory graph backed by CozoDB. Use it proactively:\n\
            - `memory_store`: Save facts, decisions, preferences, and behavioral rules for future recall. \
            Always store things the user asks you to remember.\n\
            - `memory_recall`: Search past memories by keyword. Use this when the user asks what you \
            remember, or when context from past sessions would be useful.\n\
            Memories persist across sessions. Store important decisions and preferences immediately.",
            chrono::Utc::now().format("%Y-%m-%d"),
            session_id
        )),
    };

    let sections = assembler.assemble(&input);
    if let Some(ref override_text) = resolved_flags.system_prompt_override {
        return vec![serde_json::json!({ "type": "text", "text": override_text })];
    }

    let mut blocks: Vec<serde_json::Value> = sections
        .into_iter()
        .map(|section| {
            let mut block = serde_json::json!({
                "type": "text",
                "text": section.content,
            });
            if config.context.prompt_cache
                && let Some(ref cc) = section.cache_control
            {
                block["cache_control"] = serde_json::json!({ "type": cc });
            }
            block
        })
        .collect();
    if let Some(ref append_text) = resolved_flags.system_prompt_append {
        blocks.push(serde_json::json!({ "type": "text", "text": append_text }));
    }
    if let Some(ref text) = output_style_prompt(cli, config) {
        tracing::info!("injecting output style into system prompt");
        blocks.push(serde_json::json!({ "type": "text", "text": text }));
    }
    blocks
}

fn agent_prompt_text(def: &archon_core::agents::definition::CustomAgentDefinition) -> String {
    let mut prompt = def.system_prompt.clone();
    if !def.tool_guidance.is_empty() {
        prompt = format!("{prompt}\n\n<tool-guidance>\n{}\n</tool-guidance>", def.tool_guidance);
    }
    if let Some(ref skills) = def.skills
        && !skills.is_empty()
    {
        prompt = format!(
            "{prompt}\n\n<available-skills>\nThe following skills are available to you: {}\nInvoke them by name when relevant to the task.\n</available-skills>",
            skills.join(", ")
        );
    }
    if !def.leann_queries.is_empty() {
        prompt = format!(
            "{prompt}\n\n<leann-queries>\nRelevant code search queries for your task: {}\nUse these with the LEANN semantic search tool when exploring the codebase.\n</leann-queries>",
            def.leann_queries.join(", ")
        );
    }
    if !def.tags.is_empty() {
        prompt = format!(
            "{prompt}\n\n<agent-tags>\nYour memory tags: {}\nUse these tags when storing or recalling memories relevant to your role.\n</agent-tags>",
            def.tags.join(", ")
        );
    }
    prompt
}

fn output_style_prompt(
    cli: &Cli,
    config: &archon_core::config::ArchonConfig,
) -> Option<String> {
    use archon_core::output_style::OutputStyleRegistry;
    use archon_core::output_style_loader::load_styles_from_dir;

    let mut reg = OutputStyleRegistry::new();
    if let Some(home) = dirs::home_dir() {
        let new_dir = home.join(".archon").join("output-styles");
        if new_dir.is_dir() {
            for style in load_styles_from_dir(&new_dir) {
                reg.register(style);
            }
        } else {
            let old_dir = home.join(".claude").join("output-styles");
            if old_dir.is_dir() {
                tracing::warn!(
                    "Loading from deprecated path {}. Rename to {} to suppress this warning.",
                    old_dir.display(),
                    new_dir.display()
                );
                for style in load_styles_from_dir(&old_dir) {
                    reg.register(style);
                }
            }
        }
    }
    if let Some(name) = cli.output_style.as_deref().or(config.output_style.as_deref()) {
        reg.get_or_default(name).prompt.clone()
    } else {
        reg.forced_plugin_style().and_then(|s| s.prompt.clone())
    }
}
