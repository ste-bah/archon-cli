use std::collections::HashSet;

use archon_core::skills::SkillRegistry;
use archon_tui::commands::{CommandInfo, CommandKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillCollision<'a> {
    pub skill: &'a str,
    pub primary: &'a str,
}

impl SkillCollision<'_> {
    pub fn warning(&self) -> String {
        format!(
            "skill '/{}' is shadowed by primary command '/{}'; primary dispatch wins",
            self.skill, self.primary
        )
    }
}

pub fn build_command_catalog(
    primaries: Vec<(&str, &str)>,
    skills: &SkillRegistry,
) -> Vec<CommandInfo> {
    let primary_names: HashSet<_> = primaries.iter().map(|(name, _)| *name).collect();
    let mut catalog: Vec<_> = primaries
        .into_iter()
        .map(|(name, description)| CommandInfo {
            name: format!("/{name}"),
            description: description.to_string(),
            kind: CommandKind::Primary,
        })
        .chain(
            skills
                .list_all()
                .into_iter()
                .filter(|(name, _)| !primary_names.contains(name))
                .map(|(name, description)| CommandInfo {
                    name: format!("/{name}"),
                    description: description.to_string(),
                    kind: CommandKind::Skill,
                }),
        )
        .collect();

    catalog.sort_by_cached_key(|command| {
        (
            match command.kind {
                CommandKind::Primary => 0,
                CommandKind::Skill => 1,
            },
            command.name.to_ascii_lowercase(),
        )
    });
    catalog
}

pub fn primary_skill_collisions<'a>(
    primary_names: impl IntoIterator<Item = &'a str>,
    skills: &'a SkillRegistry,
) -> Vec<SkillCollision<'a>> {
    primary_names
        .into_iter()
        .filter_map(|primary| {
            skills.get(primary).map(|skill| SkillCollision {
                skill: skill.name(),
                primary,
            })
        })
        .collect()
}

pub fn warn_primary_skill_collisions<'a>(
    primary_names: impl IntoIterator<Item = &'a str>,
    skills: &'a SkillRegistry,
) -> Vec<SkillCollision<'a>> {
    let collisions = primary_skill_collisions(primary_names, skills);
    for collision in &collisions {
        tracing::warn!("{}", collision.warning());
    }
    collisions
}

#[cfg(test)]
mod tests {
    use archon_core::skills::{Skill, SkillContext, SkillOutput};

    use super::*;

    struct TestSkill {
        name: &'static str,
        description: &'static str,
    }

    impl Skill for TestSkill {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            self.description
        }

        fn execute(&self, _args: &[String], _ctx: &SkillContext) -> SkillOutput {
            SkillOutput::Text(String::new())
        }
    }

    fn registry(skills: &[(&'static str, &'static str)]) -> SkillRegistry {
        let mut registry = SkillRegistry::new();
        for &(name, description) in skills {
            registry.register(Box::new(TestSkill { name, description }));
        }
        registry
    }

    fn write_skill(root: &std::path::Path, name: &str, description: &str) {
        let skill_dir = root.join(".archon/skills").join(name);
        std::fs::create_dir_all(&skill_dir).expect("skill directory");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n"),
        )
        .expect("skill definition");
    }

    #[test]
    fn catalog_includes_existing_registered_skill_without_discovery() {
        let skills = registry(&[("deploy-check", "Check deployment health")]);

        let catalog = build_command_catalog(vec![("help", "Show help")], &skills);

        assert!(catalog.iter().any(|command| {
            command.name == "/deploy-check" && command.kind == CommandKind::Skill
        }));
    }

    #[test]
    fn primaries_sort_before_skills_then_alphabetically() {
        let skills = registry(&[("z-skill", "Last skill"), ("a-skill", "First skill")]);

        let catalog = build_command_catalog(
            vec![
                ("z-primary", "Last command"),
                ("a-primary", "First command"),
            ],
            &skills,
        );

        let names: Vec<_> = catalog
            .iter()
            .map(|command| command.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["/a-primary", "/z-primary", "/a-skill", "/z-skill"]
        );
    }

    #[test]
    fn shadowed_canonical_skill_is_omitted_and_reports_winning_primary() {
        let skills = registry(&[
            ("help", "Skill help"),
            ("deploy-check", "Check deployment health"),
        ]);

        let catalog = build_command_catalog(vec![("help", "Show help")], &skills);
        let collisions = primary_skill_collisions(["help"], &skills);

        assert!(
            !catalog
                .iter()
                .any(|command| command.name == "/help" && command.kind == CommandKind::Skill)
        );
        assert_eq!(collisions.len(), 1);
        assert_eq!(collisions[0].skill, "help");
        assert_eq!(collisions[0].primary, "help");
    }

    #[test]
    fn catalog_contains_each_canonical_skill_once_without_alias_rows() {
        let mut skills = registry(&[("help", "Skill help")]);
        skills.register_alias("?", "help");

        let catalog = build_command_catalog(Vec::new(), &skills);

        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].name, "/help");
    }

    #[test]
    fn catalog_includes_skill_discovered_from_temp_archon_root() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        write_skill(temp_dir.path(), "deploy-check", "Check deployment health");
        let skills = super::super::slash_context_builder::build_skill_registry(temp_dir.path());

        let catalog = build_command_catalog(vec![("help", "Show help")], &skills);

        assert!(catalog.iter().any(|command| {
            command.name == "/deploy-check" && command.kind == CommandKind::Skill
        }));
    }

    #[test]
    fn installed_plugin_bundle_skill_uses_production_startup_registry() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        write_skill(
            temp_dir.path(),
            "plugin-deploy-check",
            "Plugin deployment check",
        );
        let skills = super::super::slash_context_builder::build_skill_registry(temp_dir.path());

        let catalog = build_command_catalog(Vec::new(), &skills);

        assert!(catalog.iter().any(|command| {
            command.name == "/plugin-deploy-check" && command.kind == CommandKind::Skill
        }));
    }

    #[test]
    #[ignore = "deterministic startup catalog smoke"]
    #[tracing_test::traced_test]
    fn skill_autocomplete_live_smoke() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        write_skill(temp_dir.path(), "deploy-check", "Check deployment health");
        write_skill(temp_dir.path(), "help", "Skill help");
        let skills = super::super::slash_context_builder::build_skill_registry(temp_dir.path());
        let primaries = crate::command::registry::default_registry();
        let catalog = build_command_catalog(primaries.primaries_with_descriptions(), &skills);
        let collisions = warn_primary_skill_collisions(
            primaries
                .primaries_with_descriptions()
                .into_iter()
                .map(|(name, _)| name),
            &skills,
        );

        assert!(catalog.iter().any(|command| {
            command.name == "/deploy-check"
                && command.kind == CommandKind::Skill
                && command.kind.label() == "[skill]"
        }));
        assert!(catalog.iter().any(|command| {
            command.name == "/help"
                && command.kind == CommandKind::Primary
                && command.kind.label() == "[command]"
        }));
        assert!(
            !catalog
                .iter()
                .any(|command| { command.name == "/help" && command.kind == CommandKind::Skill })
        );
        assert!(collisions.iter().any(|collision| collision.skill == "help"));
        assert!(logs_contain(
            "skill '/help' is shadowed by primary command '/help'; primary dispatch wins"
        ));
    }
}
