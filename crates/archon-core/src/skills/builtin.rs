use super::{Skill, SkillContext, SkillOutput, SkillRegistry};

// ---------------------------------------------------------------------------
// Macro to reduce boilerplate for simple descriptor-only skills
// ---------------------------------------------------------------------------

macro_rules! define_skill {
    ($struct_name:ident, $name:expr, $desc:expr) => {
        pub struct $struct_name;

        impl Skill for $struct_name {
            fn name(&self) -> &str {
                $name
            }

            fn description(&self) -> &str {
                $desc
            }

            fn execute(&self, _args: &[String], _ctx: &SkillContext) -> SkillOutput {
                SkillOutput::Text(format!("/{}: {}", self.name(), self.description()))
            }
        }
    };
}

define_skill!(HelpSkill, "help", "Show all commands");
define_skill!(
    CompactSkill,
    "compact",
    "Compact context (micro | snip N-M | auto)"
);
define_skill!(PlanSkill, "plan", "Show or update the current plan");
define_skill!(FastSkill, "fast", "Toggle fast mode");
define_skill!(
    EffortSkill,
    "effort",
    "Set effort level (high, medium, low)"
);
define_skill!(CostSkill, "cost", "Show session cost");
define_skill!(StatusSkill, "status", "Show session status");
define_skill!(DoctorSkill, "doctor", "Run diagnostics");
define_skill!(
    GardenSkill,
    "garden",
    "Run memory consolidation (/garden) or show stats (/garden stats)"
);

// ---------------------------------------------------------------------------
// Git skills
// ---------------------------------------------------------------------------

/// Show git status for the current working directory.
pub struct GitStatusSkill;

impl Skill for GitStatusSkill {
    fn name(&self) -> &str {
        "git-status"
    }

    fn description(&self) -> &str {
        "Show git repository status (modified, staged, untracked files)"
    }

    fn aliases(&self) -> Vec<&str> {
        vec!["gs"]
    }

    fn execute(&self, _args: &[String], ctx: &SkillContext) -> SkillOutput {
        let repo = match archon_tools::git::open_repo(&ctx.working_dir) {
            Ok(r) => r,
            Err(e) => return SkillOutput::Error(e),
        };
        match archon_tools::git::status::git_status(&repo) {
            Ok(info) => SkillOutput::Markdown(archon_tools::git::status::format_status(&info)),
            Err(e) => SkillOutput::Error(e),
        }
    }
}

/// Show git diff output.
pub struct DiffSkill;

impl Skill for DiffSkill {
    fn name(&self) -> &str {
        "diff"
    }

    fn description(&self) -> &str {
        "Show git diff (use --staged for staged changes)"
    }

    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
        let staged = args.iter().any(|a| a == "--staged" || a == "-s");
        let repo = match archon_tools::git::open_repo(&ctx.working_dir) {
            Ok(r) => r,
            Err(e) => return SkillOutput::Error(e),
        };
        match archon_tools::git::diff::git_diff(&repo, staged) {
            Ok(d) if d.is_empty() => SkillOutput::Text("No changes.".to_string()),
            Ok(d) => SkillOutput::Markdown(format!("```diff\n{d}\n```")),
            Err(e) => SkillOutput::Error(e),
        }
    }
}

/// List, create, or switch branches.
pub struct BranchSkill;

impl Skill for BranchSkill {
    fn name(&self) -> &str {
        "branch"
    }

    fn description(&self) -> &str {
        "List branches, create (--create NAME), or switch (--switch NAME)"
    }

    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
        let repo = match archon_tools::git::open_repo(&ctx.working_dir) {
            Ok(r) => r,
            Err(e) => return SkillOutput::Error(e),
        };

        // Parse sub-commands
        if let Some(pos) = args.iter().position(|a| a == "--create" || a == "-c") {
            if let Some(name) = args.get(pos + 1) {
                return match archon_tools::git::branch::create_branch(&repo, name) {
                    Ok(()) => SkillOutput::Text(format!("Branch '{name}' created.")),
                    Err(e) => SkillOutput::Error(e),
                };
            }
            return SkillOutput::Error("--create requires a branch name".to_string());
        }

        if let Some(pos) = args.iter().position(|a| a == "--switch" || a == "-s") {
            if let Some(name) = args.get(pos + 1) {
                return match archon_tools::git::branch::switch_branch(&repo, name) {
                    Ok(()) => SkillOutput::Text(format!("Switched to branch '{name}'.")),
                    Err(e) => SkillOutput::Error(e),
                };
            }
            return SkillOutput::Error("--switch requires a branch name".to_string());
        }

        // Default: list branches
        match archon_tools::git::branch::list_branches(&repo) {
            Ok(branches) => {
                let mut out = String::from("Branches:\n");
                for b in &branches {
                    let marker = if b.is_current { "* " } else { "  " };
                    let remote = if b.is_remote { " (remote)" } else { "" };
                    out.push_str(&format!("{marker}{}{remote}\n", b.name));
                }
                SkillOutput::Text(out)
            }
            Err(e) => SkillOutput::Error(e),
        }
    }
}

/// Create a git commit.
pub struct CommitSkill;

impl Skill for CommitSkill {
    fn name(&self) -> &str {
        "commit"
    }

    fn description(&self) -> &str {
        "Stage all and commit with message (-m MSG) or generate prompt for LLM"
    }

    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
        let repo = match archon_tools::git::open_repo(&ctx.working_dir) {
            Ok(r) => r,
            Err(e) => return SkillOutput::Error(e),
        };

        // Extract message from -m flag
        let message = args
            .iter()
            .position(|a| a == "-m")
            .and_then(|pos| args.get(pos + 1))
            .map(|s| s.as_str());

        if let Err(e) = archon_tools::git::commit::stage_all(&repo) {
            return SkillOutput::Error(format!("Failed to stage: {e}"));
        }

        match message {
            Some(msg) => match archon_tools::git::commit::commit(&repo, msg) {
                Ok(hash) => SkillOutput::Text(format!("Committed: {hash}")),
                Err(e) => SkillOutput::Error(e),
            },
            None => {
                // Generate a prompt for the LLM to produce a commit message
                let diff = archon_tools::git::diff::git_diff(&repo, true).unwrap_or_default();
                if diff.is_empty() {
                    return SkillOutput::Error("Nothing staged to commit.".to_string());
                }
                let prompt = archon_tools::git::commit::generate_commit_message_prompt(&diff);
                SkillOutput::Markdown(prompt)
            }
        }
    }
}

/// Create a pull request via `gh` CLI, or show PR info if no title given.
pub struct PrSkill;

impl Skill for PrSkill {
    fn name(&self) -> &str {
        "pr"
    }

    fn description(&self) -> &str {
        "Create pull request via gh CLI (/pr \"Title\" --body \"desc\") or show PR info"
    }

    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
        let repo = match archon_tools::git::open_repo(&ctx.working_dir) {
            Ok(r) => r,
            Err(e) => return SkillOutput::Error(e),
        };

        // If no args, show PR info (branch + diff stats)
        if args.is_empty() {
            let branch =
                archon_tools::git::current_branch(&repo).unwrap_or_else(|_| "unknown".into());
            let stats = archon_tools::git::diff::git_diff_stats(&repo);

            let mut out = format!("PR preparation for branch: {branch}\n");
            match stats {
                Ok(s) => {
                    out.push_str(&format!(
                        "Files changed: {}, Insertions: +{}, Deletions: -{}\n\n\
                         Usage: /pr \"Title\" --body \"Description\"\n",
                        s.files_changed, s.insertions, s.deletions
                    ));
                }
                Err(e) => {
                    out.push_str(&format!("Could not compute diff stats: {e}\n"));
                }
            }
            return SkillOutput::Text(out);
        }

        // First arg is the title
        let title = &args[0];

        // Extract --body flag
        let body = args
            .iter()
            .position(|a| a == "--body")
            .and_then(|pos| args.get(pos + 1))
            .map(|s| s.as_str());

        match archon_tools::git::pr::create_pr(title, body) {
            Ok(url) => SkillOutput::Text(format!("Pull request created: {url}")),
            Err(e) => SkillOutput::Error(e),
        }
    }
}

/// Create a [`SkillRegistry`] pre-populated with all built-in skills.
pub fn register_builtins() -> SkillRegistry {
    let mut registry = SkillRegistry::new();

    registry.register(Box::new(HelpSkill));
    registry.register(Box::new(CompactSkill));
    registry.register(Box::new(PlanSkill));
    registry.register(Box::new(FastSkill));
    registry.register(Box::new(EffortSkill));
    registry.register(Box::new(CostSkill));
    registry.register(Box::new(StatusSkill));
    registry.register(Box::new(DoctorSkill));
    registry.register(Box::new(GardenSkill));

    // Git skills
    registry.register(Box::new(GitStatusSkill));
    registry.register(Box::new(DiffSkill));
    registry.register(Box::new(BranchSkill));
    registry.register(Box::new(CommitSkill));
    registry.register(Box::new(PrSkill));

    // Expanded skills (CLI-225)
    super::expanded::register_expanded_skills(&mut registry);

    // Phase 1 skill foundation
    registry.register(Box::new(super::to_prd::ToPrdSkill));
    registry.register(Box::new(super::prd_to_spec::PrdToSpecSkill));

    // Phase 2 engineering pack
    registry.register(Box::new(super::engineering_pack::GrillMeSkill));
    registry.register(Box::new(super::engineering_pack::GrillWithDocsSkill));
    registry.register(Box::new(super::engineering_pack::DiagnoseSkill));
    registry.register(Box::new(super::engineering_pack::TddSkill));
    registry.register(Box::new(super::engineering_pack::ZoomOutSkill));

    // Agent management skills (PRD-AGENTS-001)
    registry.register(Box::new(super::agent_skills::CreateAgentSkill));
    registry.register(Box::new(super::agent_skills::ListAgentsSkill));
    registry.register(Box::new(super::agent_skills::AdjustBehaviorSkill));
    registry.register(Box::new(super::agent_skills::EvolveAgentSkill));
    registry.register(Box::new(super::agent_skills::ArchiveAgentSkill));
    registry.register(Box::new(super::agent_skills::AgentHistorySkill));
    registry.register(Box::new(super::agent_skills::RollbackBehaviorSkill));

    registry
}
