use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("project root")
}

fn agent_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("md" | "toml")
            ) {
                out.push(path);
            }
        }
    }

    let dir = project_root().join(".archon/agents");
    let mut files = Vec::new();
    walk(&dir, &mut files);
    files.sort();

    // A walk that finds nothing is not a clean tree, it is a broken scan. Both
    // tests below assert that a list built from these files is empty, so if
    // `.archon/agents` is renamed, emptied, or excluded from a checkout they
    // pass without inspecting a single agent definition. Refuse instead: the
    // directory is tracked and holds hundreds of files, so anything near zero
    // means the scan target moved.
    assert!(
        files.len() > 100,
        "found only {} agent definition(s) under {} — the scan target has moved, \
         and a guard that inspects nothing must not report success",
        files.len(),
        dir.display()
    );

    files
}

fn frontmatter(text: &str) -> Option<&str> {
    if !text.starts_with("---") {
        return None;
    }
    let end = text[3..].find("\n---")?;
    Some(&text[3..end + 3])
}

fn clean_tool(raw: &str) -> Option<String> {
    let cleaned = raw
        .split('#')
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"')
        .trim_matches('\'');
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

fn declared_tools(frontmatter: &str) -> Vec<String> {
    let lines: Vec<&str> = frontmatter.lines().collect();
    let mut tools = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        let Some((key, rest)) = trimmed.split_once(':') else {
            i += 1;
            continue;
        };
        if key != "tools" && key != "allowed_tools" {
            i += 1;
            continue;
        }

        let base_indent = line.len() - trimmed.len();
        let rest = rest.trim();
        if !rest.is_empty() && rest != "|" && rest != ">" {
            tools.extend(rest.split(',').filter_map(clean_tool));
            i += 1;
            continue;
        }

        i += 1;
        while i < lines.len() {
            let child = lines[i];
            if child.trim().is_empty() {
                i += 1;
                continue;
            }
            let child_indent = child.len() - child.trim_start().len();
            if child_indent <= base_indent {
                break;
            }
            if let Some(item) = child.trim_start().strip_prefix("- ") {
                tools.extend(clean_tool(item));
            }
            i += 1;
        }
    }
    tools
}

#[test]
fn agent_tool_declarations_match_archon_tool_names() {
    let mut allowed: HashSet<String> =
        archon_core::dispatch::create_default_registry(project_root(), None)
            .tool_names()
            .into_iter()
            .map(str::to_string)
            .collect();

    // Session-wired and conditional tools.
    allowed.extend(
        [
            "AgentCatalog",
            "memory_store",
            "memory_recall",
            "LeannSearch",
            "LeannFindSimilar",
        ]
        .into_iter()
        .map(str::to_string),
    );

    let mut unknown = Vec::new();
    let mut declarations_inspected = 0usize;
    for file in agent_files() {
        let text = fs::read_to_string(&file).expect("read agent file");
        let Some(fm) = frontmatter(&text) else {
            continue;
        };
        for tool in declared_tools(fm) {
            declarations_inspected += 1;
            if !allowed.contains(&tool) {
                unknown.push(format!("{}: {tool}", file.display()));
            }
        }
    }

    // The `continue` above is the second way this test can inspect nothing: if
    // frontmatter parsing or `declared_tools` stops recognising the on-disk
    // format, every file is skipped and `unknown` is empty for the wrong reason.
    // Say how many declarations were actually checked, and fail if none were.
    assert!(
        declarations_inspected > 0,
        "read the agent files but extracted no tool declarations at all — \
         frontmatter parsing has stopped matching the on-disk format, so this \
         check is inspecting nothing"
    );

    assert!(
        unknown.is_empty(),
        "agent declarations reference non-Archon tools ({declarations_inspected} \
         declaration(s) inspected):\n{}",
        unknown.join("\n")
    );
}

#[test]
fn agent_prompts_do_not_reference_removed_tool_names() {
    let removed = [
        "mcp__",
        "MultiEdit",
        "NotebookRead",
        "TodoRead",
        "memory_search",
        "claude-flow memory",
        "God Agent",
    ];
    let mut hits = Vec::new();
    for file in agent_files() {
        let text = fs::read_to_string(&file).expect("read agent file");
        for needle in removed {
            if text.contains(needle) {
                hits.push(format!("{}: {needle}", file.display()));
            }
        }
    }

    assert!(
        hits.is_empty(),
        "agent prompts still reference removed tool/runtime names:\n{}",
        hits.join("\n")
    );
}
