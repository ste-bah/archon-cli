//! Additional observations that never enter the mutation candidate set.
use super::*;

pub(super) fn absolute_reads(
    words: &[(String, Option<PathBuf>)],
    commands: &[Vec<String>],
    written: &BTreeSet<String>,
    is_message: impl Fn(&str) -> bool + Copy,
) -> Vec<String> {
    let messages: BTreeSet<_> = commands
        .iter()
        .filter(|words| message_command(words, is_message))
        .flatten()
        .flat_map(|word| paths::diagnostic_path_tokens(word))
        .collect();
    let others: BTreeSet<_> = commands
        .iter()
        .filter(|words| !message_command(words, is_message))
        .flatten()
        .flat_map(|word| paths::diagnostic_path_tokens(word))
        .collect();
    let written: BTreeSet<_> = written
        .iter()
        .flat_map(|word| paths::diagnostic_path_tokens(word))
        .collect();
    words
        .iter()
        .filter_map(|(word, base)| base.as_ref().map(|_| word))
        .flat_map(|word| paths::diagnostic_path_tokens(word))
        .filter(|token| {
            !written
                .iter()
                .any(|word| Path::new(word).starts_with(token))
                && (!messages.contains(token) || others.contains(token))
        })
        .collect()
}

pub(super) fn nonnumeric_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| !ext.to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
}

fn message_command(words: &[String], is_message: impl Fn(&str) -> bool) -> bool {
    words
        .iter()
        .find(|word| !word.starts_with('\0'))
        .is_some_and(|word| is_message(word))
}

pub(super) fn message_only_tokens(
    commands: &[Vec<String>],
    is_message: impl Fn(&str) -> bool + Copy,
) -> BTreeSet<String> {
    let other: BTreeSet<_> = commands
        .iter()
        .filter(|words| !message_command(words, is_message))
        .flatten()
        .flat_map(|word| path_tokens(word))
        .collect();
    commands
        .iter()
        .filter(|words| message_command(words, is_message))
        .flatten()
        .flat_map(|word| path_tokens(word))
        .filter(|token| !other.contains(token))
        .collect()
}

pub(super) fn quote_words(
    mut quoted: BTreeSet<String>,
    raw: &[Vec<String>],
    expanded: &[Vec<String>],
    unquoted_positions: &BTreeSet<(usize, usize)>,
    is_message: impl Fn(&str) -> bool + Copy,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut unquoted = BTreeSet::new();
    for (index, (source, words)) in raw.iter().zip(expanded).enumerate() {
        let source = source
            .iter()
            .enumerate()
            .filter(|(_, word)| !word.starts_with('\0'));
        for ((position, original), word) in
            source.zip(words.iter().filter(|word| !word.starts_with('\0')))
        {
            if unquoted_positions.contains(&(index, position)) {
                if !message_command(words, is_message)
                    && !original.split_once('=').is_some_and(|(name, _)| {
                        !name.is_empty()
                            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    })
                {
                    unquoted.extend([original.clone(), word.clone()]);
                }
            } else {
                quoted.insert(word.clone());
            }
        }
    }
    (quoted, unquoted)
}

pub(super) fn quoted_tokens(
    quoted: &BTreeSet<String>,
    unquoted: &BTreeSet<String>,
) -> BTreeSet<String> {
    let unquoted: BTreeSet<_> = unquoted.iter().flat_map(|word| path_tokens(word)).collect();
    quoted
        .iter()
        .flat_map(|word| path_tokens(word))
        .filter(|token| !unquoted.contains(token))
        .collect()
}

type Collected = (
    Vec<String>,
    BTreeSet<String>,
    BTreeSet<String>,
    BTreeSet<String>,
);

pub(super) fn collect(
    words: &[(String, Option<PathBuf>)],
    raw: &[Vec<String>],
    commands: &[Vec<String>],
    quoted: BTreeSet<String>,
    unquoted_positions: &BTreeSet<(usize, usize)>,
    written: &BTreeSet<String>,
    is_message: impl Fn(&str) -> bool + Copy,
) -> Collected {
    let reads = absolute_reads(words, commands, written, is_message);
    let mut messages = message_only_tokens(commands, is_message);
    let (quoted, unquoted) = quote_words(quoted, raw, commands, unquoted_positions, is_message);
    let tokens = quoted_tokens(&quoted, &unquoted);
    // Escaped quoted text can yield path fragments, but supplies no read evidence.
    messages.extend(
        quoted
            .difference(&unquoted)
            .filter(|word| word.contains('\\'))
            .flat_map(|word| path_tokens(word))
            .filter(|token| tokens.contains(token)),
    );
    let mut unconfirmed = BTreeSet::new();
    let reads = split_absolute(reads, &quoted, &unquoted, &mut unconfirmed);
    (reads, tokens, messages, unconfirmed)
}

pub(super) fn split_absolute(
    reads: Vec<String>,
    quoted: &BTreeSet<String>,
    unquoted: &BTreeSet<String>,
    unconfirmed: &mut BTreeSet<String>,
) -> Vec<String> {
    let plain: BTreeSet<_> = unquoted
        .iter()
        .flat_map(|word| paths::diagnostic_path_tokens(word))
        .collect();
    let quoted: BTreeSet<_> = quoted
        .iter()
        .flat_map(|word| paths::diagnostic_path_tokens(word))
        .collect();
    reads
        .into_iter()
        .filter(|path| {
            if quoted.contains(path) && !plain.contains(path) {
                unconfirmed.insert(path.clone());
                false
            } else {
                true
            }
        })
        .collect()
}

pub(super) fn record_bare(
    token: &str,
    mapped: &str,
    eligible: bool,
    quoted: &BTreeSet<String>,
    missing: &mut BTreeSet<String>,
    unconfirmed: &mut BTreeSet<String>,
) {
    if eligible {
        if quoted.contains(token) {
            unconfirmed.insert(mapped.to_string());
        } else if !token.contains('/') && nonnumeric_extension(Path::new(token)) {
            missing.insert(mapped.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_metadata_preserves_a4_candidates_and_eligible() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("present.txt"), "base").unwrap();
        let absolute = format!("inspect {}", root.join("absent.json").display());
        for (command, expected, eligible) in [
            (
                "inspect data/missing.json report.json",
                vec![("data/missing.json", Some(ExclusionReason::MissingAtRoot))],
                vec!["data/missing.json"],
            ),
            (
                "bash scripts/absent.sh",
                vec![("scripts/absent.sh", Some(ExclusionReason::CheckMechanics))],
                vec![],
            ),
            (
                "echo \"data/x.json missing\"",
                vec![("data/x.json", Some(ExclusionReason::CheckMechanics))],
                vec![],
            ),
            (
                "inspect present.txt",
                vec![("present.txt", None)],
                vec!["present.txt"],
            ),
            (absolute.as_str(), vec![], vec![]),
        ] {
            let entry: AcceptanceCriterion = serde_json::from_value(serde_json::json!({
                "id": "AC-X-001", "criterion": "output", "check": {"kind": "command", "command": command, "cwd": "repo_root"},
                "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": "fixture"}
            })).unwrap();
            let actual = input_discovery_for_check(&entry, root, root, None);
            assert_eq!(
                actual.eligible.iter().map(Path::new).collect::<Vec<_>>(),
                eligible.iter().map(Path::new).collect::<Vec<_>>(),
                "{command}"
            );
            // Compare every pre-existing candidate field; the new bool is
            // observation metadata and is never read by A4.
            let candidates: Vec<_> = actual
                .candidates
                .iter()
                .map(|c| {
                    (
                        Path::new(&c.path),
                        c.exclusion,
                        c.move_path.as_deref(),
                        c.move_root.as_deref(),
                    )
                })
                .collect();
            let expected: Vec<_> = expected
                .into_iter()
                .map(|(p, e)| (Path::new(p), e, None, None))
                .collect();
            assert_eq!(candidates, expected, "{command}");
        }
    }
}
