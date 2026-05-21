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
        let path_instruction = match explicit_prd_output_path(args) {
            Some(path) => format!(
                "Use this exact output path: `{path}`. Do not choose a \
                 different PRD slug or path."
            ),
            None => "Pick a kebab-case slug for the PRD subfolder (4-6 words max, \
                 derived from the PRD title or main feature name), then write \
                 to `prds/<your-slug>/PRD.md`."
                .to_string(),
        };
        let user_block = format!(
            "Use the ai-agent-prd framework above to write a PRD.\n\
             \n\
             Source material: the current conversation context.{extra}\n\
             \n\
             OUTPUT REQUIREMENTS:\n\
             1. {path_instruction}\n\
             2. Use the Write tool to create the PRD. The tool input MUST be \
                a JSON object with string fields named exactly \
                `file_path` and `content`, for example: \
                {{\"file_path\":\"prds/example/PRD.md\",\"content\":\"...\"}}.\n\
             3. `file_path` must be the PRD path string, not omitted, not \
                nested, and not called `path` or `filename`.\n\
             4. Create parent directories as needed through the Write tool.\n\
             5. After writing, print the path you wrote to.\n\
             6. Do NOT print the full PRD content to the conversation — only \
                write it to the file."
        );
        SkillOutput::Prompt(format!("{template}\n\n---USER REQUEST---\n\n{user_block}"))
    }
}

fn explicit_prd_output_path(args: &[String]) -> Option<String> {
    args.iter().find_map(|arg| {
        let start = arg.find("prds/")?;
        let candidate = &arg[start..];
        let cleaned = candidate.trim_matches(|c: char| {
            matches!(c, '`' | '"' | '\'' | ',' | '.' | ';' | ':' | '(' | ')')
        });
        (cleaned.ends_with("PRD.md") && cleaned.contains('/')).then(|| cleaned.to_string())
    })
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
            session_store: None,
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
            session_store: None,
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
            session_store: None,
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
    fn to_prd_with_explicit_output_path_pins_write_path_and_schema() {
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::env::temp_dir(),
            model: "test".into(),
            agent_registry: None,
            session_store: None,
        };
        let args: Vec<String> = vec![
            "Write".into(),
            "it".into(),
            "to".into(),
            "prds/gss-alert-disposition-platform/PRD.md.".into(),
        ];
        let out = ToPrdSkill.execute(&args, &ctx);
        match out {
            SkillOutput::Prompt(s) => {
                assert!(s.contains(
                    "Use this exact output path: `prds/gss-alert-disposition-platform/PRD.md`"
                ));
                assert!(s.contains("`file_path` and `content`"));
                assert!(s.contains("\"file_path\":\"prds/example/PRD.md\""));
            }
            _ => panic!("expected Prompt"),
        }
    }

    #[test]
    fn explicit_prd_output_path_ignores_non_prd_paths() {
        let args = vec!["docs/example.md".to_string()];
        assert_eq!(explicit_prd_output_path(&args), None);
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
