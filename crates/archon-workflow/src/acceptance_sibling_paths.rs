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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiblingSource {
    pub alias: PathBuf,
    pub source: PathBuf,
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
            let candidate = normalize(project.join(named.path));
            if candidate.starts_with(&project) || !candidate.exists() {
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
                && let Ok(source) = archon_shell::paths::canonicalize(&alias)
                && !source.starts_with(&project)
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
    let words = crate::check_source_resolve::words(command);
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
    word.starts_with("../").then(|| PathBuf::from(word))
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
}
