//! Relative sibling paths mentioned by shell checks and the positions that
//! make an absent path a setup failure.

use std::path::PathBuf;

#[path = "task_set_contract_policy_paths_commands.rs"]
pub(crate) mod commands;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiblingPath {
    pub path: PathBuf,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SiblingRevision {
    pub path: PathBuf,
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_repository: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<crate::task_set_contract::TrustedCwd>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiblingSource {
    pub alias: PathBuf,
    pub source: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiblingExclusionReason {
    MissingAtRoot,
    SiblingTreeFromSelectedRoot,
    ProjectRootOrAncestor,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiblingCandidate {
    pub path: PathBuf,
    pub reason: SiblingExclusionReason,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SiblingDiscovery {
    pub siblings: Vec<SiblingSource>,
    pub candidates: Vec<SiblingCandidate>,
}

/// Program text carried by interpreter flags or shell here-documents.
/// Shared with acceptance input discovery so both scanners see the same code.
pub fn program_texts(command: &str) -> Vec<String> {
    let words = crate::check_source_resolve::words(command);
    let commands: Vec<Vec<String>> = words
        .split(|word| matches!(word.as_str(), ";" | "&&" | "||" | "|" | "&"))
        .map(<[String]>::to_vec)
        .collect();
    let mut programs: Vec<String> = inline_programs(&commands)
        .into_iter()
        .map(|(_, _, program)| program)
        .collect();
    programs.extend(heredoc_bodies(command));
    programs
}

/// The position and text of each program passed to a recognized inline flag.
pub fn inline_programs(commands: &[Vec<String>]) -> Vec<(usize, usize, String)> {
    const FLAGS: [&str; 5] = ["-c", "-e", "--eval", "--command", "--exec"];
    let mut found = Vec::new();
    for (command_index, command) in commands.iter().enumerate() {
        let mut argument_index = 1;
        while let Some(argument) = command.get(argument_index) {
            if FLAGS.contains(&argument.as_str()) {
                if let Some(program) = command.get(argument_index + 1) {
                    found.push((command_index, argument_index + 1, program.clone()));
                }
                argument_index += 2;
            } else {
                argument_index += 1;
            }
        }
    }
    found
}

pub(crate) fn heredoc_bodies(command: &str) -> Vec<String> {
    let mut found = Vec::new();
    let lines: Vec<_> = command.lines().collect();
    let mut line = 0;
    while line < lines.len() {
        let Some(at) = lines[line]
            .find("<<")
            .filter(|at| !lines[line][*at..].starts_with("<<<"))
        else {
            line += 1;
            continue;
        };
        let rest = lines[line][at + 2..].trim_start_matches('-').trim_start();
        let delimiter: String = rest
            .trim_start_matches(['\'', '"'])
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let body_start = line + 1;
        let close = (!delimiter.is_empty())
            .then(|| (body_start..lines.len()).find(|index| lines[*index].trim() == delimiter))
            .flatten();
        if let Some(close) = close {
            found.push(lines[body_start..close].join("\n"));
            line = close + 1;
        } else {
            line += 1;
        }
    }
    found
}

/// Resolve existing sibling inputs named by shell checks. `alias` keeps the
/// path spelling the check uses; `source` follows symlinks for materializing
/// their content.
pub fn discover(project: &std::path::Path, checks: &[String]) -> Vec<SiblingSource> {
    discover_with_diagnostics(project, checks).siblings
}

/// Resolve sibling inputs while preserving every sibling-shaped path the
/// checks considered and the generic reason it cannot be moved as evidence.
pub fn discover_with_diagnostics(project: &std::path::Path, checks: &[String]) -> SiblingDiscovery {
    let Ok(project) = archon_shell::paths::canonicalize(project) else {
        return SiblingDiscovery::default();
    };
    let mut siblings = Vec::new();
    let mut candidates = Vec::new();
    for check in checks {
        for named in scan(check) {
            if !names_entry_below_parent(&named.path) {
                continue;
            }
            let mut candidate = normalize(project.join(&named.path));
            // A missing descendant can be a deliverable. Materialize its
            // existing sibling root so policy and execution see the same tree.
            if !candidate.exists()
                && let Some(parent) = project.parent()
                && let Ok(relative) = candidate.strip_prefix(parent)
                && let Some(first) = relative.components().next()
            {
                let sibling = parent.join(first);
                if sibling.exists() {
                    candidate = sibling;
                }
            }
            if !candidate.exists() {
                candidates.push(SiblingCandidate {
                    path: named.path,
                    reason: SiblingExclusionReason::MissingAtRoot,
                });
                continue;
            }
            if candidate == project
                || candidate == project.parent().unwrap_or(&project)
                || project.starts_with(&candidate)
            {
                candidates.push(SiblingCandidate {
                    path: named.path,
                    reason: SiblingExclusionReason::ProjectRootOrAncestor,
                });
                continue;
            }
            if candidate.starts_with(&project) {
                continue;
            }
            candidates.push(SiblingCandidate {
                path: named.path.clone(),
                reason: SiblingExclusionReason::SiblingTreeFromSelectedRoot,
            });
            let alias = if candidate.is_dir()
                || (candidate.is_file() && candidate.parent() == project.parent())
            {
                candidate
            } else if let Some(parent) = candidate.parent() {
                parent.to_path_buf()
            } else {
                continue;
            };
            if !alias.starts_with(&project)
                && !project.starts_with(&alias)
                && let Ok(source) = archon_shell::paths::canonicalize(&alias)
                && !source.starts_with(&project)
                && !project.starts_with(&source)
                && (source.is_dir() || source.is_file())
            {
                siblings.push(SiblingSource { alias, source });
            }
        }
    }
    siblings.sort_by(|left, right| left.alias.cmp(&right.alias));
    siblings.dedup_by(|left, right| left.alias == right.alias);
    candidates.sort_by(|left, right| left.path.cmp(&right.path));
    candidates.dedup_by(|left, right| left.path == right.path);
    SiblingDiscovery {
        siblings,
        candidates,
    }
}

fn normalize(path: PathBuf) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// Find every `../` word, marking only setup positions as required.
///
/// Existing paths are useful inputs even when mentioned as ordinary
/// arguments; missing ordinary arguments are not setup failures.
pub fn scan(command: &str) -> Vec<SiblingPath> {
    scan_from(command, std::path::Path::new(""))
}

#[path = "acceptance_sibling_scan.rs"]
mod scan_paths;
pub(crate) use scan_paths::scan_from;

/// The existing program-text path detection, shared with rooted policy scans.
pub(crate) fn program_parent_paths(program: &str, directory: &std::path::Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let program = strip_quote_prefixes(program);
    for word in crate::check_source_resolve::words(&without_comments(&program)) {
        if let Some(path) = sibling_word(&word, directory) {
            found.push(path);
        }
    }
    // R2: a quoted string glued to the code before it (`run(['../x'`)
    // is no word of its own; it is read from its opening quote.
    for quoted in quoted_parent_paths(&program) {
        if let Some(path) = sibling_word(quoted, directory) {
            found.push(path);
        }
    }
    found
}

/// `text` without its comments: from a `#` that starts a word outside
/// quotes to the end of its line, as the shell reads them.
pub(crate) fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut quote, mut comment, mut previous) = (None, false, '\n');
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if comment {
            if c == '\n' {
                comment = false;
                out.push(c);
            }
            previous = c;
            continue;
        }
        match quote {
            Some(q) if c == q => quote = None,
            Some('"') | None if c == '\\' => {
                out.push(c);
                if let Some(next) = chars.next() {
                    out.push(next);
                }
                previous = c;
                continue;
            }
            Some(_) => {}
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c == '#' && previous.is_whitespace() => {
                comment = true;
                previous = c;
                continue;
            }
            None => {}
        }
        out.push(c);
        previous = c;
    }
    out
}

/// Every quoted string of program text that lexically starts with `../`:
/// the text from just after its opening quote to the same quote closing it.
fn quoted_parent_paths(program: &str) -> Vec<&str> {
    program
        .char_indices()
        .filter(|(_, quote)| matches!(quote, '\'' | '"'))
        .filter_map(|(at, quote)| {
            let rest = &program[at + 1..];
            rest.starts_with("../")
                .then(|| rest.find(quote).map(|end| &rest[..end]))
                .flatten()
        })
        .collect()
}

fn sibling_word(word: &str, directory: &std::path::Path) -> Option<PathBuf> {
    let value = word.split_once('=').map_or(word, |(_, value)| value);
    let value = value.strip_prefix("$PWD/").unwrap_or(value);
    let value = value.trim_matches(|character: char| {
        character.is_ascii_punctuation() && !is_path_character(character)
    });
    if value.is_empty() {
        return None;
    }
    let path = PathBuf::from(value);
    if path.is_absolute() || value.contains(['$', '`', '~']) {
        return None;
    }
    let path = relative_normalize(directory.join(path));
    names_entry_below_parent(&path).then_some(path)
}

/// Characters shared by the sibling and movable input path scanners.
/// Non-ASCII characters remain part of a path. A backslash is not: in check
/// text it is almost always an escape (`\n`, `\.`), which would glue junk to
/// a real path (Issue-489).
pub fn is_path_character(character: char) -> bool {
    !character.is_ascii()
        || character.is_ascii_alphanumeric()
        || matches!(character, '.' | '_' | '/' | '-' | '+' | '@')
}

pub(crate) fn names_entry_below_parent(path: &std::path::Path) -> bool {
    use std::path::Component;
    let mut depth = 0usize;
    let mut escaped = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            Component::ParentDir if !escaped => escaped = true,
            Component::ParentDir => return false,
            _ => return false,
        }
    }
    escaped && depth > 0
}

/// The shell lexer removes quote delimiters. Remove a short ASCII prefix
/// immediately before a quote first, so the resulting token still starts at
/// the quoted path.
fn strip_quote_prefixes(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for index in 1..bytes.len() {
        if !matches!(bytes[index], b'\'' | b'"') {
            continue;
        }
        if !text[index + 1..].starts_with("../") {
            continue;
        }
        let mut start = index;
        while start > cursor && bytes[start - 1].is_ascii_alphabetic() && index - start < 3 {
            start -= 1;
        }
        if start < index {
            out.push_str(&text[cursor..start]);
            cursor = index;
        }
    }
    out.push_str(&text[cursor..]);
    out
}

fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn push(found: &mut Vec<SiblingPath>, path: PathBuf, required: bool) {
    found.push(SiblingPath { path, required });
}

#[cfg(test)]
#[path = "acceptance_sibling_paths_tests.rs"]
mod tests;

/// Lexical normalization that preserves leading parents in a relative path.
fn relative_normalize(path: PathBuf) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir if result.file_name().is_some_and(|p| p != "..") => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// Refuse only an absent sibling root; its descendants may be deliverables.
/// Shared by author preflight and the actual scratch site.
pub fn missing_root(root: &std::path::Path, command: &str) -> Option<String> {
    let root = archon_shell::paths::canonicalize(root).ok()?;
    for named in commands::missing_root_paths(command, &root) {
        let path = normalize(root.join(&named));
        if path.starts_with(&root) {
            continue;
        }
        let Some(parent) = root.ancestors().skip(1).find(|p| path.starts_with(p)) else {
            continue;
        };
        let Some(first) = path
            .strip_prefix(parent)
            .ok()
            .and_then(|p| p.components().next())
        else {
            continue;
        };
        // Only the first component outside the working root is setup.
        // Descendants may be files or directories the work must add.
        if !parent.join(first).exists() {
            let shown = format!(
                "{}{}",
                "../".repeat(root.strip_prefix(parent).ok()?.components().count()),
                first.as_os_str().to_string_lossy()
            );
            return Some(format!(
                "names sibling directory {}, which does not exist next to the working directory; name an existing directory",
                shown
            ));
        }
    }
    None
}
