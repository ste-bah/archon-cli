//! Relative sibling paths mentioned by shell checks and the positions that
//! make an absent path a setup failure.

use std::path::PathBuf;

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
    pub cwd: Option<crate::task_set_contract::TrustedCwd>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiblingSource {
    pub alias: PathBuf,
    pub source: PathBuf,
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

fn heredoc_bodies(command: &str) -> Vec<String> {
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
    let Ok(project) = archon_shell::paths::canonicalize(project) else {
        return Vec::new();
    };
    let mut siblings = Vec::new();
    for check in checks {
        for named in scan(check) {
            if !names_entry_below_parent(&named.path) {
                continue;
            }
            let candidate = normalize(project.join(named.path));
            if candidate.starts_with(&project)
                || candidate == project.parent().unwrap_or(&project)
                || project.starts_with(&candidate)
                || !candidate.exists()
            {
                continue;
            }
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
    siblings
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
    let command = strip_quote_prefixes(command);
    let words = crate::check_source_resolve::words(&command);
    let mut found = Vec::new();
    let mut command_start = 0;
    let mut redirect_target = false;
    while command_start < words.len() {
        let end = (command_start..words.len())
            .find(|index| {
                matches!(
                    words[*index].as_str(),
                    ";" | "&&" | "||" | "|" | "&" | "(" | ")"
                )
            })
            .unwrap_or(words.len());
        let segment = &words[command_start..end];
        let command_index = segment
            .iter()
            .position(|word| !is_assignment(word) && !word.starts_with("-"));
        let executable = command_index.map(|index| segment[index].as_str());
        let executable = executable
            .unwrap_or_default()
            .rsplit('/')
            .next()
            .unwrap_or_default();
        let mut required_indexes = Vec::new();
        if matches!(executable, "cd" | "pushd")
            && let Some(index) = command_index.and_then(|command| {
                ((command + 1)..segment.len()).find(|index| !segment[*index].starts_with('-'))
            })
        {
            required_indexes.push(index);
        }
        for index in 0..segment.len() {
            let word = &segment[index];
            if matches!(word.as_str(), "-C" | "--manifest-path" | "--path") {
                if index + 1 < segment.len() {
                    required_indexes.push(index + 1);
                }
            } else if let Some((option, value)) = word.split_once('=')
                && matches!(option, "-C" | "--manifest-path" | "--path")
                && !value.is_empty()
            {
                if let Some(path) = sibling_word(value) {
                    push(&mut found, path, true);
                }
            }
            if matches!(word.as_str(), ">" | ">>" | "<" | "2>" | "1>") {
                redirect_target = true;
            } else if redirect_target {
                required_indexes.push(index);
                redirect_target = false;
            }
        }
        if matches!(executable, "test" | "[") {
            for (index, word) in segment.iter().enumerate() {
                if matches!(word.as_str(), "-f" | "-d" | "-e" | "-x" | "-s")
                    && index + 1 < segment.len()
                {
                    required_indexes.push(index + 1);
                }
            }
        }
        for (index, word) in segment.iter().enumerate() {
            if let Some(path) = sibling_word(word) {
                push(&mut found, path, required_indexes.contains(&index));
            }
        }
        command_start = end + 1;
    }
    for program in program_texts(&command) {
        let program = strip_quote_prefixes(&program);
        for word in crate::check_source_resolve::words(&program) {
            if let Some(path) = sibling_word(&word) {
                push(&mut found, path, true);
            }
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path).then(a.required.cmp(&b.required)));
    let mut merged: Vec<SiblingPath> = Vec::new();
    for item in found {
        if let Some(existing) = merged.iter_mut().find(|old| old.path == item.path) {
            existing.required |= item.required;
        } else {
            merged.push(item);
        }
    }
    merged
}

fn sibling_word(word: &str) -> Option<PathBuf> {
    let value = word.split_once('=').map_or(word, |(_, value)| value);
    let path = PathBuf::from(value);
    names_entry_below_parent(&path).then_some(path)
}

fn names_entry_below_parent(path: &std::path::Path) -> bool {
    let mut components = path.components();
    if !matches!(components.next(), Some(std::path::Component::ParentDir)) {
        return false;
    }
    let mut depth = 0usize;
    for component in components {
        match component {
            std::path::Component::Normal(_) => depth += 1,
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir if depth > 0 => depth -= 1,
            std::path::Component::ParentDir => return false,
            _ => return false,
        }
    }
    depth > 0
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
mod tests {
    use super::*;

    #[test]
    fn shell_positions_and_quoted_fragments_are_classified() {
        // EXPECTED: all required positions parse quotes and `=` forms; prose
        // remains a copy candidate but is not required.
        for command in [
            "cd '../a b'",
            "pushd \"../a\"'b'",
            "git -C=../a status",
            "cargo test --manifest-path=\"../a/Cargo.toml\"",
            "tool --path '../a'",
            "echo hi > '../a/out'",
            "test -f '../a/file'",
            "[ -d '../a/dir' ]",
        ] {
            let paths = scan(command);
            assert!(!paths.is_empty(), "{command}");
            assert!(
                paths.iter().all(|path| path.required),
                "{command}: {paths:?}"
            );
        }
        let prose = scan("echo '../missing'");
        assert_eq!(prose.len(), 1);
        assert!(!prose[0].required);
    }

    #[test]
    fn leading_assignments_are_skipped_when_finding_commands() {
        // EXPECTED before implementation: assignments are environment words,
        // so paths in cd, test, and [ positions remain required.
        for command in [
            "ROOT=1 cd ../missing",
            "A=1 B=2 test -f ../missing",
            "ROOT=1 [ -e ../missing ]",
        ] {
            let paths = scan(command);
            assert_eq!(paths.len(), 1, "{command}: {paths:?}");
            assert!(paths[0].required, "{command}: {paths:?}");
        }
    }

    #[test]
    fn program_text_paths_are_scanned_but_project_paths_are_not_siblings() {
        for command in [
            "python3 <<'PY'\nMANIFEST='../sibling/Cargo.toml'\nPY",
            "python3 -c 'MANIFEST=\"../sibling/Cargo.toml\"'",
        ] {
            let paths = scan(command);
            assert_eq!(paths.len(), 1, "{command}: {paths:?}");
            assert_eq!(paths[0].path, PathBuf::from("../sibling/Cargo.toml"));
            assert!(paths[0].required, "{command}: {paths:?}");
        }
        assert_eq!(
            scan("python3 -c 'MANIFEST=\"crates/inside/Cargo.toml\"'"),
            []
        );
    }

    #[test]
    fn parent_paths_are_not_sibling_entries() {
        for command in [
            "echo ../",
            "echo ..",
            "echo ../..",
            "echo ../../x",
            "echo ../x/../../outside",
            "python3 -c 'value = \"../../x\"'",
        ] {
            assert!(scan(command).is_empty(), "{command}: {:?}", scan(command));
        }
    }

    #[test]
    fn short_letter_prefixes_before_quotes_preserve_sibling_paths() {
        for command in [
            "echo f\"../x/y\"",
            "echo r'../x'",
            "echo b\"../x\"",
            "python3 -c 'value = f\"../x/y\"'",
        ] {
            let paths = scan(command);
            assert_eq!(paths.len(), 1, "{command}: {paths:?}");
            assert!(paths[0].path.starts_with("../"), "{command}: {paths:?}");
        }
    }

    #[test]
    fn discovery_never_returns_the_project_parent_or_ancestor() {
        let tree = tempfile::tempdir().unwrap();
        let project = tree.path().join("parent/project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(tree.path().join("parent/file"), "parent").unwrap();
        let checks = [
            "echo ../".to_string(),
            "echo ../..".to_string(),
            "echo ../../file".to_string(),
            "echo ../project/../..".to_string(),
        ];
        for check in checks {
            assert!(discover(&project, &[check.clone()]).is_empty(), "{check}");
        }
    }
}
