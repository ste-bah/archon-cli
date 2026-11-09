//! Focused commands come from declarations and executable availability.
//! Code spans are read in whole, leading, and embedded positions alike.
//! Shell assignments and syntax retain their existing command semantics.
//! Missing executables remain visible through the host execution report.

pub(crate) fn continues(text: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut last_escape = false;
    let mut comment = false;
    let mut previous = '\n';
    for ch in text.chars() {
        let before = std::mem::replace(&mut previous, ch);
        last_escape = false;
        if comment {
            comment = ch != '\n';
            continue;
        }
        if escaped {
            escaped = false;
            continue;
        }
        if quote.is_none() && ch == '#' && before.is_whitespace() {
            comment = true;
            continue;
        }
        match (quote, ch) {
            (Some('\''), '\'') => quote = None,
            (Some('\''), _) => {}
            (_, '\\') => {
                escaped = true;
                last_escape = true;
            }
            (Some('"'), '"') => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(ch),
            (None, _) => {}
        }
    }
    quote.is_some() || (escaped && last_escape)
}

/// Read the first code span, or the complete unfenced item. A span can occur
/// anywhere in prose; its leading program must be declared or executable at
/// the site. No prose vocabulary determines whether a program is runnable.
#[cfg(test)]
pub fn focused_test_command(entry: &str) -> Option<String> {
    focused_test_command_at(entry, &[], std::path::Path::new(""))
}

pub fn focused_test_command_at(
    entry: &str,
    declared: &[String],
    root: &std::path::Path,
) -> Option<String> {
    focused_test_command_with_path(entry, declared, root, std::ffi::OsStr::new(""))
}

pub fn focused_test_command_with_path(
    entry: &str,
    declared: &[String],
    root: &std::path::Path,
    toolchain_path: &std::ffi::OsStr,
) -> Option<String> {
    // Raw shell statements may contain quoted backticks in their output text.
    let leading = entry.split_whitespace().next()?;
    if assignment(leading)
        || matches!(
            leading,
            "!" | "(" | "{" | "[" | "[[" | "." | ":" | "for" | "while" | "if" | "case"
        )
    {
        return Some(entry.trim().to_string());
    }
    let span = entry.split_once('`');
    let candidate = span
        .map_or(entry, |(_, rest)| rest.split('`').next().unwrap_or(rest))
        .trim();
    let first = candidate.split_whitespace().next()?;
    // Unfenced sentence-style items retain their lexical prose distinction.
    // A declaration can still identify an uppercase program explicitly.
    if span.is_none()
        && first.starts_with(char::is_uppercase)
        && !declared.iter().any(|tool| tool == first)
        && !assignment(first)
    {
        return None;
    }
    // A code span must identify a declared or available program.
    let mut remaining = candidate;
    let first = loop {
        let (word, rest) = shell_word(remaining)?;
        remaining = rest;
        if word == "!" {
            continue;
        }
        if assignment(word) {
            if let Some((_, substitution)) = word.split_once("$(") {
                break shell_word(substitution)?
                    .0
                    .trim_end_matches([')', '\'', '"']);
            }
            continue;
        }
        break word;
    }
    .trim_start_matches('(')
    .trim_matches(['\'', '"']);
    if first.is_empty()
        || mcp_tool_name(first.split('(').next().unwrap_or(first))
        || tool_call_syntax(first)
    {
        return None;
    }
    (declared.iter().any(|tool| tool == first) || executable(first, root, toolchain_path))
        .then(|| candidate.to_string())
}

fn shell_word(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    if text.is_empty() {
        return None;
    }
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch.is_whitespace() && quote.is_none() {
            return Some((&text[..index], &text[index..]));
        }
        match (quote, ch) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('\''), _) => {}
            (_, '\\') => escaped = true,
            (None, '\'' | '"') => quote = Some(ch),
            _ => {}
        }
    }
    Some((text, ""))
}

fn executable(program: &str, root: &std::path::Path, toolchain_path: &std::ffi::OsStr) -> bool {
    let path = std::path::Path::new(program);
    if path.components().count() > 1 || path.is_absolute() {
        let repository = if root.as_os_str().is_empty() {
            return false;
        } else {
            root.canonicalize()
        };
        let file = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        return repository
            .ok()
            .zip(file.canonicalize().ok())
            .is_some_and(|(root, file)| file.starts_with(root) && executable_file(&file));
    }
    {
        std::env::split_paths(toolchain_path).any(|dir| {
            let dir = if dir.is_absolute() {
                dir
            } else if root.as_os_str().is_empty() {
                return false;
            } else {
                root.join(dir)
            };
            let file = dir.join(program);
            if executable_file(&file) {
                return true;
            }
            #[cfg(windows)]
            if let Some(extensions) = std::env::var_os("PATHEXT") {
                return extensions
                    .to_string_lossy()
                    .split(';')
                    .any(|suffix| executable_file(&dir.join(format!("{program}{suffix}"))));
            }
            false
        })
    }
}
fn executable_file(path: &std::path::Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

fn mcp_tool_name(word: &str) -> bool {
    word.strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
        .is_some_and(|(server, tool)| {
            !server.is_empty()
                && !tool.is_empty()
                && tool.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

fn tool_call_syntax(word: &str) -> bool {
    word.split_once('(').is_some_and(|(name, _)| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | ':'))
    })
}

/// Find the repository containing the task, then resolve operands from its root.
pub fn repository_root(source: &str) -> std::path::PathBuf {
    let path = std::path::Path::new(source);
    for parent in path.ancestors() {
        let record = parent.join("repository.lock");
        if !record.exists() {
            continue;
        }
        let root = std::fs::read(&record)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| {
                value
                    .get("repository_root")?
                    .as_str()
                    .map(std::path::PathBuf::from)
            });
        if let Some(root) = root {
            return root;
        }
        tracing::warn!(record = %record.display(), "recorded repository root is unreadable");
        return std::path::PathBuf::new();
    }
    path.ancestors()
        .find(|parent| parent.join(".git").exists())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "task_universe_command_site_tests.rs"]
pub(crate) mod site_tests;

/// A simple program name suitable for advisory capability comparison.
/// Assignment prefixes and negation are syntax; repository paths are operands,
/// not tool capability names. Compound statements need no guessed capability.
pub fn leading_program(command: &str) -> Option<&str> {
    let mut words = command.split_whitespace();
    let first = loop {
        let word = words.next()?;
        if word == "!" {
            continue;
        }
        if assignment(word) {
            // A whitespace split cannot recover a quoted value, substitution,
            // or compound statement. Do not invent a program from its tail.
            if word.contains(['(', '\'', '"', ';']) {
                return None;
            }
            continue;
        }
        break word;
    };
    (!first.is_empty()
        && !first.starts_with('-')
        && first
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "_+.-".contains(ch))
        && !matches!(first, "for" | "while" | "if" | "case" | "."))
    .then_some(first)
}
