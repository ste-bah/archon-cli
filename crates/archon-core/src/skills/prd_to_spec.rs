//! TASK-#XXX SKILL-FOUNDATION: /prd-to-spec skill — decompose a PRD into
//! atomic task specs using the prdtospec framework.

use super::{Skill, SkillContext, SkillOutput, templates};

pub struct PrdToSpecSkill;

impl Skill for PrdToSpecSkill {
    fn name(&self) -> &str {
        "prd-to-spec"
    }

    fn description(&self) -> &str {
        "Decompose a PRD into atomic per-phase task specs using the prdtospec \
         framework. Writes to tasks/phase<N>/task<M>.md."
    }

    fn aliases(&self) -> Vec<&str> {
        vec!["decompose-prd"]
    }

    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
        let (template, source) = templates::resolve_template("prdtospec", &ctx.working_dir);
        if matches!(source, templates::TemplateSource::Missing) {
            return SkillOutput::Error(
                "prdtospec template not found (embedded fallback missing)".to_string(),
            );
        }
        if args.is_empty() {
            return SkillOutput::Error("Usage: /prd-to-spec <path/to/PRD.md>".to_string());
        }
        let prd_path = &args[0];
        let user_block = format!(
            "Use the prdtospec framework above to decompose the PRD at \
             `{prd_path}` into atomic per-phase task specs.\n\
             \n\
             OUTPUT REQUIREMENTS:\n\
             1. Read the PRD with the Read tool.\n\
             2. For each phase identified in the PRD, write task files at:\n\
                `tasks/phase<N>/task<M>.md`\n\
                (relative to the current working directory; create parent dirs).\n\
             3. After writing all files, print a summary list of paths created.\n\
             4. Do NOT print full task content to the conversation — only \
                write to files."
        );
        SkillOutput::Prompt(format!("{template}\n\n---USER REQUEST---\n\n{user_block}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::Skill;

    #[test]
    fn prd_to_spec_requires_path_arg() {
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::env::temp_dir(),
            model: "test".into(),
            agent_registry: None,
        };
        let out = PrdToSpecSkill.execute(&[], &ctx);
        match out {
            SkillOutput::Error(s) => assert!(s.contains("Usage:")),
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn prd_to_spec_emits_prompt_with_path() {
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::env::temp_dir(),
            model: "test".into(),
            agent_registry: None,
        };
        let args: Vec<String> = vec!["docs/feature.md".into()];
        let out = PrdToSpecSkill.execute(&args, &ctx);
        match out {
            SkillOutput::Prompt(s) => {
                assert!(s.contains("docs/feature.md"));
            }
            _ => panic!("expected Prompt"),
        }
    }

    #[test]
    fn prd_to_spec_emits_template_prefix() {
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::env::temp_dir(),
            model: "test".into(),
            agent_registry: None,
        };
        let args: Vec<String> = vec!["docs/feature.md".into()];
        let out = PrdToSpecSkill.execute(&args, &ctx);
        match out {
            SkillOutput::Prompt(s) => {
                assert!(s.starts_with(templates::PRD_TO_SPEC));
            }
            _ => panic!("expected Prompt"),
        }
    }

    #[test]
    fn prd_to_spec_aliases_includes_decompose_prd() {
        assert!(PrdToSpecSkill.aliases().contains(&"decompose-prd"));
    }
}
