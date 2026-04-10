//! Agent loader — parses `.md` files with YAML frontmatter into typed agent definitions
//! for the coding and research pipelines.

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::path::Path;

// ---------------------------------------------------------------------------
// Frontmatter parser
// ---------------------------------------------------------------------------

/// Parse YAML frontmatter delimited by `---` from markdown content.
///
/// Returns `(yaml_value, body)` where `body` is everything after the closing
/// `---` delimiter, trimmed of leading/trailing whitespace.
///
/// If no valid frontmatter delimiters are found, returns an empty YAML mapping
/// and the original content as the body.
pub fn parse_frontmatter(content: &str) -> Result<(serde_yml::Value, String)> {
    // Normalize CRLF → LF so frontmatter delimiters work on Windows.
    let content = &content.replace("\r\n", "\n");

    let empty_result = || {
        (
            serde_yml::Value::Mapping(Default::default()),
            content.to_string(),
        )
    };

    // The first line (possibly with leading whitespace) must be `---`.
    let first_line = content.lines().next().unwrap_or("");
    if first_line.trim() != "---" {
        return Ok(empty_result());
    }

    // Find the byte offset right after the first `---\n`.
    let after_first = match content.find("---\n") {
        Some(pos) => pos + 4,
        None => return Ok(empty_result()),
    };

    // Find the second `---` delimiter in the remainder.
    let rest = &content[after_first..];
    let yaml_end = match rest.find("\n---\n") {
        Some(pos) => pos,
        None => {
            // Handle `\n---` at the very end of the file (no trailing newline after closing ---)
            if rest.ends_with("\n---") {
                rest.len() - 4 // position of the \n before ---
            } else {
                return Ok(empty_result());
            }
        }
    };

    let yaml_text = &rest[..yaml_end];
    let yaml_value: serde_yml::Value =
        serde_yml::from_str(yaml_text).context("failed to parse YAML frontmatter")?;

    // Body starts after the `\n---\n` (or `\n---` at EOF).
    let second_delim_end = after_first + yaml_end + 4; // skip \n---
    let body = if second_delim_end < content.len() {
        let b = &content[second_delim_end..];
        let b = b.strip_prefix('\n').unwrap_or(b);
        b.trim().to_string()
    } else {
        String::new()
    };

    Ok((yaml_value, body))
}

// ---------------------------------------------------------------------------
// Custom deserializer for tool_list (accepts YAML sequence or comma string)
// ---------------------------------------------------------------------------

fn deserialize_tool_list<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    use serde::de;

    struct ToolListVisitor;

    impl<'de> de::Visitor<'de> for ToolListVisitor {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a YAML sequence of strings or a comma-separated string")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Vec<String>, E> {
            Ok(v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect())
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<String>, A::Error> {
            let mut v = Vec::new();
            while let Some(item) = seq.next_element::<String>()? {
                v.push(item);
            }
            Ok(v)
        }

        fn visit_none<E: de::Error>(self) -> Result<Vec<String>, E> {
            Ok(Vec::new())
        }

        fn visit_unit<E: de::Error>(self) -> Result<Vec<String>, E> {
            Ok(Vec::new())
        }
    }

    d.deserialize_any(ToolListVisitor)
}

/// Flexible deserializer for `quality_gates` — accepts a YAML list of strings,
/// a YAML map (converted to `"key: value"` entries), or a single string.
fn deserialize_quality_gates<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    use serde::de;

    struct QualityGatesVisitor;

    impl<'de> de::Visitor<'de> for QualityGatesVisitor {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a YAML sequence, map, or string for quality gates")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Vec<String>, E> {
            Ok(vec![v.to_string()])
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<String>, A::Error> {
            let mut v = Vec::new();
            while let Some(item) = seq.next_element::<String>()? {
                v.push(item);
            }
            Ok(v)
        }

        fn visit_map<M: de::MapAccess<'de>>(self, mut map: M) -> Result<Vec<String>, M::Error> {
            let mut v = Vec::new();
            while let Some((key, val)) = map.next_entry::<String, serde_yml::Value>()? {
                let val_str = match &val {
                    serde_yml::Value::Bool(b) => b.to_string(),
                    serde_yml::Value::Number(n) => n.to_string(),
                    serde_yml::Value::String(s) => s.clone(),
                    _ => format!("{:?}", val),
                };
                v.push(format!("{}: {}", key, val_str));
            }
            Ok(v)
        }

        fn visit_none<E: de::Error>(self) -> Result<Vec<String>, E> {
            Ok(Vec::new())
        }

        fn visit_unit<E: de::Error>(self) -> Result<Vec<String>, E> {
            Ok(Vec::new())
        }
    }

    d.deserialize_any(QualityGatesVisitor)
}

// ---------------------------------------------------------------------------
// CodingAgentDef
// ---------------------------------------------------------------------------

/// Definition of an agent in the coding pipeline, loaded from a markdown file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodingAgentDef {
    /// Derived from the filename stem (e.g. `code-generator` from `code-generator.md`).
    #[serde(skip)]
    pub key: String,

    /// Human-readable name (REQUIRED in frontmatter).
    pub name: String,

    /// Short description of what the agent does (REQUIRED).
    pub description: String,

    #[serde(default)]
    pub color: Option<String>,

    #[serde(default)]
    pub version: Option<String>,

    #[serde(default)]
    pub model: Option<String>,

    #[serde(default)]
    pub algorithm: Option<String>,

    #[serde(default)]
    pub fallback_algorithm: Option<String>,

    #[serde(default, deserialize_with = "deserialize_tool_list", alias = "tools")]
    pub tool_list: Vec<String>,

    #[serde(default)]
    pub parallelizable: bool,

    #[serde(default)]
    pub xp_reward: Option<u32>,

    #[serde(default)]
    pub memory_reads: Vec<String>,

    #[serde(default)]
    pub memory_writes: Vec<String>,

    #[serde(
        default,
        deserialize_with = "deserialize_quality_gates",
        rename = "qualityGates"
    )]
    pub quality_gates: Vec<String>,

    /// The markdown body below the frontmatter — the agent's prompt template.
    #[serde(skip)]
    pub prompt_body: String,
}

// ---------------------------------------------------------------------------
// ResearchAgentDef
// ---------------------------------------------------------------------------

/// Definition of an agent in the research pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchAgentDef {
    #[serde(skip)]
    pub key: String,

    pub name: String,

    pub description: String,

    #[serde(default)]
    pub display_name: Option<String>,

    #[serde(default)]
    pub color: Option<String>,

    #[serde(default)]
    pub model: Option<String>,

    #[serde(default)]
    pub memory_reads: Vec<String>,

    #[serde(default)]
    pub memory_writes: Vec<String>,

    #[serde(default)]
    pub output_artifacts: Vec<String>,

    #[serde(default, deserialize_with = "deserialize_tool_list", alias = "tools")]
    pub tool_list: Vec<String>,

    #[serde(skip)]
    pub prompt_body: String,
}

// ---------------------------------------------------------------------------
// Loader functions
// ---------------------------------------------------------------------------

/// Load all coding agent definitions from `*.md` files in `dir`.
///
/// Files are NOT loaded recursively — only direct children of `dir` are considered.
/// Results are sorted by `key` (filename stem).
pub fn load_coding_agents(dir: &Path) -> Result<Vec<CodingAgentDef>> {
    if !dir.exists() {
        anyhow::bail!("agent directory does not exist: {}", dir.display());
    }

    let pattern = format!("{}/*.md", dir.display());
    let mut agents = Vec::new();

    for entry in glob::glob(&pattern).context("invalid glob pattern")? {
        let path = entry.context("glob entry error")?;
        if !path.is_file() {
            continue;
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;

        let (yaml, body) = parse_frontmatter(&content)
            .with_context(|| format!("failed to parse frontmatter in {}", path.display()))?;

        let mut def: CodingAgentDef = serde_yml::from_value(yaml)
            .with_context(|| format!("failed to deserialize agent from {}", path.display()))?;

        def.key = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        def.prompt_body = body;

        agents.push(def);
    }

    agents.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(agents)
}

/// Load all research agent definitions from `*.md` files in `dir`.
pub fn load_research_agents(dir: &Path) -> Result<Vec<ResearchAgentDef>> {
    if !dir.exists() {
        anyhow::bail!("agent directory does not exist: {}", dir.display());
    }

    let pattern = format!("{}/*.md", dir.display());
    let mut agents = Vec::new();

    for entry in glob::glob(&pattern).context("invalid glob pattern")? {
        let path = entry.context("glob entry error")?;
        if !path.is_file() {
            continue;
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;

        let (yaml, body) = parse_frontmatter(&content)
            .with_context(|| format!("failed to parse frontmatter in {}", path.display()))?;

        let mut def: ResearchAgentDef = serde_yml::from_value(yaml)
            .with_context(|| format!("failed to deserialize agent from {}", path.display()))?;

        def.key = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        def.prompt_body = body;

        agents.push(def);
    }

    agents.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(agents)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_parse_frontmatter_valid() {
        let content = "---\nname: my-agent\ndescription: A test agent\n---\nThis is the body.\n";
        let (yaml, body) = parse_frontmatter(content).expect("should parse valid frontmatter");
        assert_eq!(yaml["name"].as_str().unwrap(), "my-agent");
        assert_eq!(yaml["description"].as_str().unwrap(), "A test agent");
        assert_eq!(body.trim(), "This is the body.");
    }

    #[test]
    fn test_parse_frontmatter_no_delimiters() {
        let content = "Just some plain markdown\nwith multiple lines.\n";
        let (yaml, body) = parse_frontmatter(content).expect("should succeed without delimiters");
        assert!(yaml.is_mapping());
        assert!(body.contains("Just some plain markdown"));
    }

    #[test]
    fn test_parse_frontmatter_empty_body() {
        let content = "---\nname: agent-only\ndescription: No body here\n---\n";
        let (yaml, body) = parse_frontmatter(content).expect("should parse frontmatter-only");
        assert_eq!(yaml["name"].as_str().unwrap(), "agent-only");
        assert!(
            body.trim().is_empty(),
            "body should be empty but got: {:?}",
            body
        );
    }

    #[test]
    fn test_parse_frontmatter_malformed_yaml() {
        let content = "---\n: invalid: [yaml: {\n---\nbody\n";
        let result = parse_frontmatter(content);
        assert!(result.is_err(), "malformed YAML should produce an error");
    }

    #[test]
    fn test_parse_frontmatter_body_preserves_markdown() {
        let body_text =
            "# Heading\n\n- item 1\n- item 2\n\n```rust\nfn main() {}\n```\n\nParagraph here.\n";
        let content = format!("---\nname: md-test\ndescription: test\n---\n{}", body_text);
        let (_, body) = parse_frontmatter(&content).expect("should parse");
        assert!(body.contains("# Heading"), "heading preserved");
        assert!(body.contains("- item 1"), "list preserved");
        assert!(body.contains("```rust"), "code block preserved");
        assert!(body.contains("fn main() {}"), "code content preserved");
        assert!(body.contains("Paragraph here."), "paragraph preserved");
    }

    #[test]
    fn test_parse_frontmatter_no_yaml_delimiters_in_body() {
        let content = "---\nname: delim-test\ndescription: check\n---\nBody content here.\n";
        let (_, body) = parse_frontmatter(content).expect("should parse");
        assert!(
            !body.contains("---"),
            "body should not contain frontmatter delimiters"
        );
    }

    fn write_coding_agent_md(dir: &std::path::Path, filename: &str, frontmatter: &str, body: &str) {
        let content = format!("---\n{}---\n{}", frontmatter, body);
        fs::write(dir.join(filename), content).expect("write test file");
    }

    #[test]
    fn test_load_coding_agents_from_dir() {
        let tmp = tempdir().expect("tempdir");
        let fm = "\
name: Code Generator\n\
description: Generates code from specs\n\
color: \"#ff0000\"\n\
version: \"1.0\"\n\
model: opus\n\
algorithm: LATS\n\
fallback_algorithm: ReAct\n\
tool_list:\n  - Read\n  - Write\n\
parallelizable: true\n\
xp_reward: 50\n\
memory_reads:\n  - task-spec\n\
memory_writes:\n  - generated-code\n\
qualityGates:\n  - compilation\n  - lint\n";
        let body = "You are a code generator agent.\n\nGenerate clean code.\n";
        write_coding_agent_md(tmp.path(), "code-generator.md", fm, body);

        let agents = load_coding_agents(tmp.path()).expect("should load agents");
        assert_eq!(agents.len(), 1);

        let a = &agents[0];
        assert_eq!(a.key, "code-generator");
        assert_eq!(a.name, "Code Generator");
        assert_eq!(a.description, "Generates code from specs");
        assert_eq!(a.color.as_deref(), Some("#ff0000"));
        assert_eq!(a.version.as_deref(), Some("1.0"));
        assert_eq!(a.model.as_deref(), Some("opus"));
        assert_eq!(a.algorithm.as_deref(), Some("LATS"));
        assert_eq!(a.fallback_algorithm.as_deref(), Some("ReAct"));
        assert_eq!(a.tool_list, vec!["Read", "Write"]);
        assert!(a.parallelizable);
        assert_eq!(a.xp_reward, Some(50));
        assert_eq!(a.memory_reads, vec!["task-spec"]);
        assert_eq!(a.memory_writes, vec!["generated-code"]);
        assert_eq!(a.quality_gates, vec!["compilation", "lint"]);
        assert!(a.prompt_body.contains("You are a code generator agent."));
    }

    #[test]
    fn test_load_coding_agents_missing_optional_fields() {
        let tmp = tempdir().expect("tempdir");
        let fm = "name: Minimal Agent\ndescription: Bare minimum\n";
        write_coding_agent_md(tmp.path(), "minimal.md", fm, "Do things.\n");

        let agents = load_coding_agents(tmp.path()).expect("should load");
        assert_eq!(agents.len(), 1);

        let a = &agents[0];
        assert_eq!(a.key, "minimal");
        assert_eq!(a.name, "Minimal Agent");
        assert_eq!(a.description, "Bare minimum");
        assert!(a.color.is_none());
        assert!(a.version.is_none());
        assert!(a.model.is_none());
        assert!(a.algorithm.is_none());
        assert!(a.fallback_algorithm.is_none());
        assert!(a.tool_list.is_empty());
        assert!(!a.parallelizable);
        assert!(a.xp_reward.is_none());
        assert!(a.memory_reads.is_empty());
        assert!(a.memory_writes.is_empty());
        assert!(a.quality_gates.is_empty());
        assert!(a.prompt_body.contains("Do things."));
    }

    #[test]
    fn test_load_coding_agents_empty_dir() {
        let tmp = tempdir().expect("tempdir");
        let agents = load_coding_agents(tmp.path()).expect("should succeed on empty dir");
        assert!(agents.is_empty(), "empty dir should yield empty vec");
    }

    #[test]
    fn test_load_coding_agents_nonexistent_dir() {
        let result = load_coding_agents(std::path::Path::new(
            "/tmp/does-not-exist-agent-loader-test",
        ));
        assert!(result.is_err(), "nonexistent dir should return error");
    }

    #[test]
    fn test_coding_agent_key_from_filename() {
        let tmp = tempdir().expect("tempdir");
        let fm = "name: Key Test\ndescription: Testing key derivation\n";
        write_coding_agent_md(tmp.path(), "my-cool-agent.md", fm, "body\n");

        let agents = load_coding_agents(tmp.path()).expect("should load");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].key, "my-cool-agent");
    }

    #[test]
    fn test_load_coding_agents_skips_non_md_files() {
        let tmp = tempdir().expect("tempdir");
        let fm = "name: Real Agent\ndescription: This is real\n";
        write_coding_agent_md(tmp.path(), "real-agent.md", fm, "body\n");

        fs::write(
            tmp.path().join("config.toml"),
            "[settings]\nkey = \"value\"\n",
        )
        .expect("write toml");
        fs::write(tmp.path().join("notes.txt"), "some notes\n").expect("write txt");

        let agents = load_coding_agents(tmp.path()).expect("should load");
        assert_eq!(agents.len(), 1, "should only load .md files");
        assert_eq!(agents[0].key, "real-agent");
    }

    fn write_research_agent_md(
        dir: &std::path::Path,
        filename: &str,
        frontmatter: &str,
        body: &str,
    ) {
        let content = format!("---\n{}---\n{}", frontmatter, body);
        fs::write(dir.join(filename), content).expect("write test file");
    }

    #[test]
    fn test_load_research_agents_from_dir() {
        let tmp = tempdir().expect("tempdir");
        let fm = "\
name: literature-mapper\n\
display_name: Literature Mapper\n\
description: Maps literature sources\n\
color: \"#00ff00\"\n\
model: sonnet\n\
memory_reads:\n  - research-plan\n\
memory_writes:\n  - literature-map\n\
output_artifacts:\n  - literature-review.md\n\
tool_list:\n  - WebSearch\n  - Read\n";
        let body = "You are a literature mapping agent.\n\nMap all sources.\n";
        write_research_agent_md(tmp.path(), "literature-mapper.md", fm, body);

        let agents = load_research_agents(tmp.path()).expect("should load research agents");
        assert_eq!(agents.len(), 1);

        let a = &agents[0];
        assert_eq!(a.key, "literature-mapper");
        assert_eq!(a.name, "literature-mapper");
        assert_eq!(a.display_name.as_deref(), Some("Literature Mapper"));
        assert_eq!(a.description, "Maps literature sources");
        assert_eq!(a.color.as_deref(), Some("#00ff00"));
        assert_eq!(a.model.as_deref(), Some("sonnet"));
        assert_eq!(a.memory_reads, vec!["research-plan"]);
        assert_eq!(a.memory_writes, vec!["literature-map"]);
        assert_eq!(a.output_artifacts, vec!["literature-review.md"]);
        assert_eq!(a.tool_list, vec!["WebSearch", "Read"]);
        assert!(
            a.prompt_body
                .contains("You are a literature mapping agent.")
        );
    }

    #[test]
    fn test_load_research_agents_missing_optional_fields() {
        let tmp = tempdir().expect("tempdir");
        let fm = "name: basic-researcher\ndescription: Basic research agent\n";
        write_research_agent_md(tmp.path(), "basic-researcher.md", fm, "Research things.\n");

        let agents = load_research_agents(tmp.path()).expect("should load");
        assert_eq!(agents.len(), 1);

        let a = &agents[0];
        assert_eq!(a.key, "basic-researcher");
        assert_eq!(a.name, "basic-researcher");
        assert!(a.display_name.is_none());
        assert_eq!(a.description, "Basic research agent");
        assert!(a.color.is_none());
        assert!(a.model.is_none());
        assert!(a.memory_reads.is_empty());
        assert!(a.memory_writes.is_empty());
        assert!(a.output_artifacts.is_empty());
        assert!(a.tool_list.is_empty());
        assert!(a.prompt_body.contains("Research things."));
    }

    #[test]
    fn test_parse_frontmatter_multiple_yaml_docs() {
        // Body contains `---` which should NOT be treated as a third delimiter.
        let content = "---\nname: x\ndescription: y\n---\nBody with --- in it\nand more text.\n";
        let (yaml, body) = parse_frontmatter(content).expect("should parse");
        assert_eq!(yaml["name"].as_str().unwrap(), "x");
        assert!(
            body.contains("---"),
            "body should preserve the literal --- that appears after the closing delimiter"
        );
        assert!(body.contains("and more text."));
    }

    #[test]
    fn test_parse_frontmatter_unicode_content() {
        let content = "---\nname: 测试代理\ndescription: An agent with 🚀 emoji\n---\n你好世界 🌍\nUnicode body here.\n";
        let (yaml, body) = parse_frontmatter(content).expect("should parse unicode frontmatter");
        assert_eq!(yaml["name"].as_str().unwrap(), "测试代理");
        assert!(
            yaml["description"].as_str().unwrap().contains("🚀"),
            "emoji in frontmatter preserved"
        );
        assert!(
            body.contains("你好世界"),
            "Chinese characters in body preserved"
        );
        assert!(body.contains("🌍"), "emoji in body preserved");
    }

    #[test]
    fn test_coding_agent_deserialize_tool_list_comma_string() {
        let tmp = tempdir().expect("tempdir");
        let fm =
            "name: Comma Tools\ndescription: Tools as comma string\ntools: \"Read, Write, Edit\"\n";
        write_coding_agent_md(tmp.path(), "comma-tools.md", fm, "body\n");

        let agents = load_coding_agents(tmp.path()).expect("should load");
        assert_eq!(agents.len(), 1);
        assert_eq!(
            agents[0].tool_list,
            vec!["Read", "Write", "Edit"],
            "comma-separated string should deserialize into 3 elements"
        );
    }

    #[test]
    fn test_coding_agent_deserialize_quality_gates_single_string() {
        // The custom deserializer accepts a single string (not just a list).
        // Verify that a scalar string value produces a Vec with one element.
        let yaml_str = r#"
name: "Gate String Agent"
description: "Quality gates as single string"
qualityGates: "compilation-only"
"#;
        let yaml: serde_yml::Value = serde_yml::from_str(yaml_str).expect("parse yaml");
        let def: CodingAgentDef = serde_yml::from_value(yaml).expect("deserialize agent");
        assert_eq!(
            def.quality_gates,
            vec!["compilation-only"],
            "single string should produce a Vec with one element"
        );
    }

    #[test]
    fn test_coding_agent_deserialize_quality_gates_map() {
        // The custom deserializer's visit_map path: directly deserialize from
        // YAML string (not via Value round-trip) to exercise the map branch.
        let yaml_str = r#"
name: "Gate Map Agent"
description: "Quality gates as map"
qualityGates:
  minScore: 0.85
  maxRetries: 3
"#;
        let def: CodingAgentDef = serde_yml::from_str(yaml_str).expect("deserialize agent");
        let gates = &def.quality_gates;
        assert_eq!(gates.len(), 2, "map should produce 2 'key: value' entries");
        assert!(
            gates.iter().any(|g| g == "minScore: 0.85"),
            "expected 'minScore: 0.85' in gates, got: {gates:?}"
        );
        assert!(
            gates.iter().any(|g| g == "maxRetries: 3"),
            "expected 'maxRetries: 3' in gates, got: {gates:?}"
        );
    }

    #[test]
    fn test_research_agent_with_all_fields() {
        let tmp = tempdir().expect("tempdir");
        let fm = "\
name: full-researcher\n\
display_name: Full Researcher\n\
description: A fully-populated research agent\n\
color: \"#abcdef\"\n\
model: opus\n\
memory_reads:\n  - context\n  - plan\n\
memory_writes:\n  - findings\n\
output_artifacts:\n  - report.md\n  - data.json\n\
tool_list:\n  - WebSearch\n  - Read\n  - Write\n";
        let body = "You are a comprehensive research agent.\n\nDo all the research.\n";
        write_research_agent_md(tmp.path(), "full-researcher.md", fm, body);

        let agents = load_research_agents(tmp.path()).expect("should load");
        assert_eq!(agents.len(), 1);

        let a = &agents[0];
        assert_eq!(a.key, "full-researcher");
        assert_eq!(a.name, "full-researcher");
        assert_eq!(a.display_name.as_deref(), Some("Full Researcher"));
        assert_eq!(a.description, "A fully-populated research agent");
        assert_eq!(a.color.as_deref(), Some("#abcdef"));
        assert_eq!(a.model.as_deref(), Some("opus"));
        assert_eq!(a.memory_reads, vec!["context", "plan"]);
        assert_eq!(a.memory_writes, vec!["findings"]);
        assert_eq!(a.output_artifacts, vec!["report.md", "data.json"]);
        assert_eq!(a.tool_list, vec!["WebSearch", "Read", "Write"]);
        assert!(a.prompt_body.contains("comprehensive research agent"));
    }
}
