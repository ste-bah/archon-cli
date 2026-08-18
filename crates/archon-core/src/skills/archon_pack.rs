//! Phase 3 archon-specific skills: spec-to-tasks, compose-pipeline,
//! ci-gate-walker, setup-archon-skills, write-a-skill.
//!
//! Each skill embeds its SKILL.md at compile time. Override resolution
//! lets users replace any skill body without recompiling.

use std::sync::OnceLock;

use super::engineering_pack::{ParsedEmbedded, parse_once};
use super::{Skill, SkillContext, SkillOutput, embedded_skill_md, templates};

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

            fn agent_invocable(&self) -> bool {
                true
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

                SkillOutput::Prompt(format!("{body}\n\n---USER REQUEST---\n\n{user_block}"))
            }
        }
    };
}

archon_skill!(SpecToTasksSkill, SPEC_TO_TASKS);
archon_skill!(ComposePipelineSkill, COMPOSE_PIPELINE);
archon_skill!(CiGateWalkerSkill, CI_GATE_WALKER);
archon_skill!(SetupArchonSkillsSkill, SETUP_ARCHON_SKILLS);
archon_skill!(WriteASkillSkill, WRITE_A_SKILL);

// #187 method pack.
archon_skill!(VerifyDoneSkill, VERIFY_DONE);
archon_skill!(ExecutePlanSkill, EXECUTE_PLAN);
archon_skill!(LandBranchSkill, LAND_BRANCH);

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
            session_store: None,
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

    /// The #187 method pack has to be reachable by the model, not just
    /// registered. A skill that is not agent-invocable never reaches the
    /// catalogue, so it can only ever be run by hand.
    #[test]
    fn the_method_pack_is_agent_invocable() {
        assert!(ExecutePlanSkill.agent_invocable());
        assert!(VerifyDoneSkill.agent_invocable());
        assert!(LandBranchSkill.agent_invocable());
    }

    #[test]
    fn method_pack_metadata() {
        assert_eq!(ExecutePlanSkill.name(), "execute-plan");
        assert_eq!(VerifyDoneSkill.name(), "verify-done");
        assert_eq!(LandBranchSkill.name(), "land-branch");
        for desc in [
            ExecutePlanSkill.description(),
            VerifyDoneSkill.description(),
            LandBranchSkill.description(),
        ] {
            // "Use when" or "Use before" — both open with the situation. The
            // point is that the first words tell the model when this applies,
            // not what the skill is about.
            assert!(
                desc.starts_with("Use when") || desc.starts_with("Use before"),
                "a description is the trigger condition, not a summary: {desc}"
            );
        }
    }

    /// The completion gate blocks on board items in `gaps_remain`, and
    /// `verify-done` is what tells the model to raise them. That coupling is
    /// invisible — the gate is Rust, the instruction is markdown, and nothing
    /// but this test connects them. Change either side and the gate silently
    /// never fires.
    ///
    /// The status is taken from `BoardStatus` rather than written as a literal
    /// so the check survives a rename on the Rust side too. Asserting a
    /// hardcoded `"gaps_remain"` would only catch the markdown drifting; if the
    /// enum's string representation changed, the skill would keep telling the
    /// model to raise a status the gate no longer queries, and this test would
    /// still pass.
    #[test]
    fn verify_done_teaches_the_contract_the_gate_enforces() {
        let body = parse_skill_md(embedded_skill_md::VERIFY_DONE)
            .expect("verify-done must parse")
            .body;

        let gate_status = archon_memory::board::BoardStatus::GapsRemain.to_string();
        assert!(
            body.contains(&gate_status),
            "must name the status the gate reads ({gate_status})"
        );
        assert!(body.contains("BoardRaise"), "must say how to record a gap");
        assert!(
            body.contains("BoardResolve"),
            "must say how to clear one, or the block is a deadlock"
        );
        assert!(
            body.contains("completion_gate"),
            "must name the config knob that turns it off"
        );
    }

    /// Each step points at the next, so following one skill leads through the
    /// chain instead of dead-ending.
    #[test]
    fn the_method_pack_chains_forward() {
        let execute = parse_skill_md(embedded_skill_md::EXECUTE_PLAN)
            .expect("parses")
            .body;
        let verify = parse_skill_md(embedded_skill_md::VERIFY_DONE)
            .expect("parses")
            .body;

        assert!(
            execute.contains("/verify-done"),
            "execute-plan should hand off to verification"
        );
        assert!(
            verify.contains("/land-branch"),
            "verify-done should hand off to landing"
        );
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
            assert!(
                parse_skill_md(raw).is_some(),
                "embedded SKILL.md must parse"
            );
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
