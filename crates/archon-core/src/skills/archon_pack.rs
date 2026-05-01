//! Phase 3 archon-specific skills: spec-to-tasks, compose-pipeline,
//! ci-gate-walker, setup-archon-skills, write-a-skill.
//!
//! Each skill embeds its SKILL.md at compile time. Override resolution
//! lets users replace any skill body without recompiling.

use std::sync::OnceLock;

use super::{Skill, SkillContext, SkillOutput, embedded_skill_md, templates};
use super::engineering_pack::{ParsedEmbedded, parse_once};

// Used in tests
#[cfg(test)]
use super::discovery::parse_skill_md;

macro_rules! archon_skill {
    ($struct_name:ident, $embedded_const:ident) => {
        pub struct $struct_name;

        impl $struct_name {
            fn parsed() -> &'static ParsedEmbedded {
                static SLOT: OnceLock<ParsedEmbedded> = OnceLock::new();
                parse_once(&SLOT, embedded_skill_md::$embedded_const)
            }
        }

        impl Skill for $struct_name {
            fn name(&self) -> &str {
                &Self::parsed().name
            }

            fn description(&self) -> &str {
                &Self::parsed().description
            }

            fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
                let body = templates::resolve_skill_body(&Self::parsed().name, &ctx.working_dir)
                    .unwrap_or_else(|| Self::parsed().body.clone());

                let user_block = if args.is_empty() {
                    "Continue with the skill's process using the current conversation \
                     context."
                        .to_string()
                } else {
                    format!("User input for this skill invocation: {}", args.join(" "))
                };

                SkillOutput::Prompt(format!(
                    "{body}\n\n---USER REQUEST---\n\n{user_block}"
                ))
            }
        }
    };
}

archon_skill!(SpecToTasksSkill, SPEC_TO_TASKS);
archon_skill!(ComposePipelineSkill, COMPOSE_PIPELINE);
archon_skill!(CiGateWalkerSkill, CI_GATE_WALKER);
archon_skill!(SetupArchonSkillsSkill, SETUP_ARCHON_SKILLS);
archon_skill!(WriteASkillSkill, WRITE_A_SKILL);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::Skill;

    #[test]
    fn spec_to_tasks_metadata() {
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::env::temp_dir(),
            model: "test".into(),
            agent_registry: None,
        };
        assert_eq!(SpecToTasksSkill.name(), "spec-to-tasks");
        assert!(!SpecToTasksSkill.description().is_empty());
        let out = SpecToTasksSkill.execute(&[], &ctx);
        assert!(matches!(out, SkillOutput::Prompt(_)));
    }

    #[test]
    fn compose_pipeline_metadata() {
        assert_eq!(ComposePipelineSkill.name(), "compose-pipeline");
        assert!(!ComposePipelineSkill.description().is_empty());
    }

    #[test]
    fn ci_gate_walker_metadata() {
        assert_eq!(CiGateWalkerSkill.name(), "ci-gate-walker");
        assert!(!CiGateWalkerSkill.description().is_empty());
    }

    #[test]
    fn setup_archon_skills_metadata() {
        assert_eq!(SetupArchonSkillsSkill.name(), "setup-archon-skills");
        assert!(!SetupArchonSkillsSkill.description().is_empty());
    }

    #[test]
    fn write_a_skill_metadata() {
        assert_eq!(WriteASkillSkill.name(), "write-a-skill");
        assert!(!WriteASkillSkill.description().is_empty());
    }

    #[test]
    fn embedded_archon_skill_md_files_parse() {
        let embedded: [&str; 5] = [
            embedded_skill_md::SPEC_TO_TASKS,
            embedded_skill_md::COMPOSE_PIPELINE,
            embedded_skill_md::CI_GATE_WALKER,
            embedded_skill_md::SETUP_ARCHON_SKILLS,
            embedded_skill_md::WRITE_A_SKILL,
        ];
        for raw in &embedded {
            assert!(parse_skill_md(raw).is_some(), "embedded SKILL.md must parse");
        }
    }

    #[test]
    fn embedded_archon_skill_md_bodies_nonempty() {
        let embedded: [&str; 5] = [
            embedded_skill_md::SPEC_TO_TASKS,
            embedded_skill_md::COMPOSE_PIPELINE,
            embedded_skill_md::CI_GATE_WALKER,
            embedded_skill_md::SETUP_ARCHON_SKILLS,
            embedded_skill_md::WRITE_A_SKILL,
        ];
        for raw in &embedded {
            let parsed = parse_skill_md(raw).unwrap();
            assert!(
                parsed.body.len() >= 10,
                "{} body is {} chars, expected non-empty",
                parsed.name,
                parsed.body.len()
            );
        }
    }
}
