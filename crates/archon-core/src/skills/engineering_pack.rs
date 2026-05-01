//! Phase 2 engineering-pack skills adapted from mattpocock/skills (MIT).
//!
//! Each skill embeds its SKILL.md at compile time. Override resolution
//! lets users replace any skill body without recompiling.

use std::sync::OnceLock;

use super::discovery::parse_skill_md;
use super::{Skill, SkillContext, SkillOutput, embedded_skill_md, templates};

/// Lazy parse of an embedded SKILL.md into a (name, description, body) triple.
/// Cached process-wide via OnceLock.
///
/// **Visibility (`pub(crate)`):** required so Phase 3's `archon_pack.rs`
/// can reuse this struct + `parse_once` without forking the macro.
pub(crate) struct ParsedEmbedded {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) body: String,
}

pub(crate) fn parse_once(
    slot: &'static OnceLock<ParsedEmbedded>,
    raw: &'static str,
) -> &'static ParsedEmbedded {
    slot.get_or_init(|| {
        let parsed =
            parse_skill_md(raw).expect("embedded SKILL.md must parse — caught by build-time test");
        ParsedEmbedded {
            name: parsed.name,
            description: parsed.description,
            body: parsed.body,
        }
    })
}

macro_rules! engineering_skill {
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

                SkillOutput::Prompt(format!("{body}\n\n---USER REQUEST---\n\n{user_block}"))
            }
        }
    };
}

engineering_skill!(GrillMeSkill, GRILL_ME);
engineering_skill!(GrillWithDocsSkill, GRILL_WITH_DOCS);
engineering_skill!(DiagnoseSkill, DIAGNOSE);
engineering_skill!(TddSkill, TDD);
engineering_skill!(ZoomOutSkill, ZOOM_OUT);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::Skill;

    #[test]
    fn grill_me_metadata() {
        let ctx = SkillContext {
            session_id: "test".into(),
            working_dir: std::env::temp_dir(),
            model: "test".into(),
            agent_registry: None,
        };
        assert_eq!(GrillMeSkill.name(), "grill-me");
        assert!(!GrillMeSkill.description().is_empty());
        // Smoke: execute returns a Prompt
        let out = GrillMeSkill.execute(&[], &ctx);
        assert!(matches!(out, SkillOutput::Prompt(_)));
    }

    #[test]
    fn grill_with_docs_metadata() {
        assert_eq!(GrillWithDocsSkill.name(), "grill-with-docs");
        let desc = GrillWithDocsSkill.description();
        assert!(
            desc.contains("context")
                || desc.contains("glossary")
                || desc.contains("CONTEXT")
                || desc.contains("documentation")
        );
    }

    #[test]
    fn diagnose_metadata() {
        assert_eq!(DiagnoseSkill.name(), "diagnose");
        assert!(!DiagnoseSkill.description().is_empty());
    }

    #[test]
    fn tdd_metadata() {
        assert_eq!(TddSkill.name(), "tdd");
        assert!(!TddSkill.description().is_empty());
    }

    #[test]
    fn zoom_out_metadata() {
        assert_eq!(ZoomOutSkill.name(), "zoom-out");
        assert!(!ZoomOutSkill.description().is_empty());
    }

    #[test]
    fn embedded_skill_md_files_parse() {
        let embedded: [&str; 5] = [
            embedded_skill_md::GRILL_ME,
            embedded_skill_md::GRILL_WITH_DOCS,
            embedded_skill_md::DIAGNOSE,
            embedded_skill_md::TDD,
            embedded_skill_md::ZOOM_OUT,
        ];
        for raw in &embedded {
            assert!(
                parse_skill_md(raw).is_some(),
                "embedded SKILL.md must parse"
            );
        }
    }

    #[test]
    fn embedded_skill_md_bodies_nonempty() {
        let embedded: [&str; 5] = [
            embedded_skill_md::GRILL_ME,
            embedded_skill_md::GRILL_WITH_DOCS,
            embedded_skill_md::DIAGNOSE,
            embedded_skill_md::TDD,
            embedded_skill_md::ZOOM_OUT,
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
