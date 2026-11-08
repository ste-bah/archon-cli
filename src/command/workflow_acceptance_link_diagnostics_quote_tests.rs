use super::*;

// Independent shell-word scan: no production lexer or observation metadata.
fn words(text: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut quoted = BTreeSet::new();
    let mut plain = BTreeSet::new();
    let mut word = String::new();
    let mut quote = None;
    let mut had_quote = false;
    let mut escaped = false;
    let mut comment = false;
    for ch in text.chars().chain(std::iter::once('\n')) {
        if comment {
            comment = ch != '\n';
            continue;
        }
        if escaped {
            word.push(ch);
            escaped = false;
        } else if ch == '\\' && quote != Some('\'') {
            escaped = true;
        } else if let Some(end) = quote {
            if ch == end {
                quote = None;
            } else {
                word.push(ch);
            }
        } else if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            had_quote = true;
        } else if ch == '#' && word.is_empty() {
            comment = true;
        } else if ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | ')' | '<' | '>') {
            if !word.is_empty() {
                if had_quote { &mut quoted } else { &mut plain }.insert(std::mem::take(&mut word));
            }
            had_quote = false;
        } else {
            word.push(ch);
        }
    }
    (quoted, plain)
}

pub(super) fn assert_no_quote_only_unowned(
    check: &archon_workflow::task_set_contract::AcceptanceCriterion,
    diagnostics: &Diagnostics,
    root: &Path,
) {
    let (_, text) = crate::command::workflow_task_set::executability::executed_text(check).unwrap();
    let (quoted, plain) = words(text);
    for token in quoted.difference(&plain) {
        assert!(
            !diagnostics
                .unowned
                .contains(&(check.id.clone(), absolute(root, token))),
            "{}: quote-only token {token}",
            check.id
        );
    }
}

#[test]
fn independent_scan_distinguishes_quote_only_and_mixed_occurrences() {
    let (quoted, plain) = words(
        "grep 'data/x.json' log; cat data/y.json; grep \"data/y.json\" log\n# ignored/path\n",
    );
    assert!(quoted.contains("data/x.json"));
    assert!(!plain.contains("data/x.json"));
    assert!(quoted.contains("data/y.json") && plain.contains("data/y.json"));
    assert!(!plain.contains("ignored/path"));
}
