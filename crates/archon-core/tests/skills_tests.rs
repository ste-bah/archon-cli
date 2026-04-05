use archon_core::skills::builtin::register_builtins;
use archon_core::skills::discovery::discover_user_skills;
use archon_core::skills::parser::parse_slash_command;
use archon_core::skills::{Skill, SkillContext, SkillOutput, SkillRegistry};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Parser tests
// ---------------------------------------------------------------------------

#[test]
fn parse_simple_command() {
    let result = parse_slash_command("/help");
    assert!(result.is_some());
    let (cmd, args) = result.unwrap();
    assert_eq!(cmd, "help");
    assert!(args.is_empty());
}

#[test]
fn parse_command_with_args() {
    let result = parse_slash_command("/model opus");
    assert!(result.is_some());
    let (cmd, args) = result.unwrap();
    assert_eq!(cmd, "model");
    assert_eq!(args, vec!["opus"]);
}

#[test]
fn parse_quoted_args() {
    let result = parse_slash_command(r#"/export "my file.md" --format json"#);
    assert!(result.is_some());
    let (cmd, args) = result.unwrap();
    assert_eq!(cmd, "export");
    assert_eq!(args, vec!["my file.md", "--format", "json"]);
}

#[test]
fn parse_empty_string() {
    assert!(parse_slash_command("").is_none());
}

#[test]
fn parse_no_slash() {
    assert!(parse_slash_command("hello").is_none());
}

// ---------------------------------------------------------------------------
// Registry tests
// ---------------------------------------------------------------------------

/// Minimal skill for testing.
struct DummySkill {
    name: String,
    description: String,
    aliases: Vec<String>,
}

impl DummySkill {
    fn new(name: &str, desc: &str) -> Self {
        Self {
            name: name.to_string(),
            description: desc.to_string(),
            aliases: Vec::new(),
        }
    }

    #[allow(dead_code)]
    fn with_aliases(mut self, aliases: Vec<&str>) -> Self {
        self.aliases = aliases.into_iter().map(String::from).collect();
        self
    }
}

impl Skill for DummySkill {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn aliases(&self) -> Vec<&str> {
        self.aliases.iter().map(|s| s.as_str()).collect()
    }
    fn execute(&self, _args: &[String], _ctx: &SkillContext) -> SkillOutput {
        SkillOutput::Text(format!("executed {}", self.name))
    }
}

#[test]
fn register_and_get() {
    let mut reg = SkillRegistry::new();
    reg.register(Box::new(DummySkill::new("help", "Show help")));
    assert!(reg.get("help").is_some());
    assert_eq!(reg.get("help").map(|s| s.name()), Some("help"));
}

#[test]
fn resolve_alias() {
    let mut reg = SkillRegistry::new();
    reg.register(Box::new(DummySkill::new("compact", "Compact context")));
    reg.register_alias("c", "compact");
    let skill = reg.resolve("c");
    assert!(skill.is_some());
    assert_eq!(skill.map(|s| s.name()), Some("compact"));
}

#[test]
fn list_all_returns_registered() {
    let mut reg = SkillRegistry::new();
    reg.register(Box::new(DummySkill::new("a", "A")));
    reg.register(Box::new(DummySkill::new("b", "B")));
    reg.register(Box::new(DummySkill::new("c", "C")));
    assert_eq!(reg.list_all().len(), 3);
}

#[test]
fn completions_prefix_match() {
    let mut reg = SkillRegistry::new();
    reg.register(Box::new(DummySkill::new("compact", "Compact")));
    reg.register(Box::new(DummySkill::new("commit", "Commit")));
    reg.register(Box::new(DummySkill::new("cost", "Cost")));
    let completions = reg.completions("co");
    assert_eq!(completions, vec!["commit", "compact", "cost"]);
}

#[test]
fn completions_exact_match() {
    let mut reg = SkillRegistry::new();
    reg.register(Box::new(DummySkill::new("compact", "Compact")));
    reg.register(Box::new(DummySkill::new("commit", "Commit")));
    let completions = reg.completions("compact");
    assert_eq!(completions, vec!["compact"]);
}

#[test]
fn unknown_command_returns_none() {
    let reg = SkillRegistry::new();
    assert!(reg.get("xyz").is_none());
}

#[test]
fn format_help_non_empty() {
    let mut reg = SkillRegistry::new();
    reg.register(Box::new(DummySkill::new("help", "Show help")));
    let help = reg.format_help();
    assert!(!help.is_empty());
    assert!(help.contains("help"));
}

#[test]
fn format_skill_help() {
    let mut reg = SkillRegistry::new();
    reg.register(Box::new(DummySkill::new("help", "Show all commands")));
    let detail = reg.format_skill_help("help");
    assert!(detail.is_some());
    let text = detail.unwrap();
    assert!(text.contains("help"));
    assert!(text.contains("Show all commands"));
}

// ---------------------------------------------------------------------------
// Discovery tests
// ---------------------------------------------------------------------------

#[test]
fn discover_empty_dir() {
    let tmp = TempDir::new().unwrap();
    let skills = discover_user_skills(tmp.path());
    assert!(skills.is_empty());
}

#[test]
fn discover_skill_from_claude_dir() {
    let tmp = TempDir::new().unwrap();
    let skill_dir = tmp.path().join(".claude/skills/test");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: test\ndescription: A test skill\n---\nDo the thing.\n",
    )
    .unwrap();
    let skills = discover_user_skills(tmp.path());
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "test");
    assert_eq!(skills[0].description, "A test skill");
    assert!(skills[0].body.contains("Do the thing."));
}

#[test]
fn discover_skill_from_archon_dir() {
    let tmp = TempDir::new().unwrap();
    let skill_dir = tmp.path().join(".archon/skills/test");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: archon-test\ndescription: Archon test skill\n---\nArchon body.\n",
    )
    .unwrap();
    let skills = discover_user_skills(tmp.path());
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "archon-test");
}

// ---------------------------------------------------------------------------
// Built-in registration tests
// ---------------------------------------------------------------------------

#[test]
fn builtin_skills_registered() {
    let reg = register_builtins();
    let names: Vec<&str> = reg.list_all().iter().map(|(n, _)| *n).collect();
    for expected in &["help", "compact", "plan", "fast", "effort", "cost"] {
        assert!(
            names.contains(expected),
            "missing built-in skill: {expected}"
        );
    }
}
