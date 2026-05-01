//! TASK-#XXX SKILL-FOUNDATION: /to-prd skill — turn current conversation
//! into a PRD using the ai-agent-prd framework.

use super::{Skill, SkillContext, SkillOutput, templates};

pub struct ToPrdSkill;

impl Skill for ToPrdSkill {
    fn name(&self) -> &str {
        "to-prd"
    }

    fn description(&self) -> &str {
        "Turn the current conversation context into a PRD using the ai-agent-prd \
         framework. Writes to prds/<slug>/PRD.md."
    }

    fn aliases(&self) -> Vec<&str> {
        vec!["prd"]
    }

    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
        let (template, source) = templates::resolve_template("ai-agent-prd", &ctx.working_dir);
        if matches!(source, templates::TemplateSource::Missing) {
            return SkillOutput::Error(
                "ai-agent-prd template not found (embedded fallback missing)".to_string(),
            );
        }
        let extra = if args.is_empty() {
            String::new()
        } else {
            format!("\n\nAdditional input from the user: {}", args.join(" "))
        };
        let user_block = format!(
            "Use the ai-agent-prd framework above to write a PRD.\n\
             \n\
             Source material: the current conversation context.{extra}\n\
             \n\
             OUTPUT REQUIREMENTS:\n\
             1. Pick a kebab-case slug for the PRD subfolder (4-6 words max, \
                derived from the PRD title or main feature name).\n\
             2. Use the Write tool to create the file at:\n\
                `prds/<your-slug>/PRD.md`\n\
                (relative to the current working directory; create parent dirs).\n\
             3. After writing, print the path you wrote to.\n\
             4. Do NOT print the full PRD content to the conversation — only \
                write it to the file."
        );
        SkillOutput::Prompt(format!("{template}\n\n---USER REQUEST---\n\n{user_block}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::Skill;

    #[test]
    fn to_prd_emits_prompt_with_template_prefix() {
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::env::temp_dir(),
            model: "test".into(),
            agent_registry: None,
        };
        let out = ToPrdSkill.execute(&[], &ctx);
        match out {
            SkillOutput::Prompt(s) => {
                assert!(s.starts_with(templates::AI_AGENT_PRD));
                assert!(s.contains("---USER REQUEST---"));
            }
            _ => panic!("expected Prompt"),
        }
    }

    #[test]
    fn to_prd_no_args_uses_conversation_context() {
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::env::temp_dir(),
            model: "test".into(),
            agent_registry: None,
        };
        let out = ToPrdSkill.execute(&[], &ctx);
        match out {
            SkillOutput::Prompt(s) => {
                assert!(!s.contains("Additional input from the user"));
            }
            _ => panic!("expected Prompt"),
        }
    }

    #[test]
    fn to_prd_with_args_includes_extra_block() {
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::env::temp_dir(),
            model: "test".into(),
            agent_registry: None,
        };
        let args: Vec<String> = vec!["focus".into(), "on".into(), "auth".into()];
        let out = ToPrdSkill.execute(&args, &ctx);
        match out {
            SkillOutput::Prompt(s) => {
                assert!(s.contains("focus on auth"));
            }
            _ => panic!("expected Prompt"),
        }
    }

    #[test]
    fn to_prd_aliases_includes_prd() {
        assert!(ToPrdSkill.aliases().contains(&"prd"));
    }

    #[test]
    fn to_prd_name_and_description() {
        assert_eq!(ToPrdSkill.name(), "to-prd");
        assert!(!ToPrdSkill.description().is_empty());
    }
}
