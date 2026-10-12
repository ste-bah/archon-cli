//! Root-relative path spelling in normalised records.

/// A path under the normalised project root, written with the platform's
/// separator (Windows displays joined paths with `\\`), spelled with `/`.
/// Only the path word after each `<project-root>` changes; it ends at
/// whitespace, a quote or closing punctuation.
/// Windows records some paths with `\` (product defect #438); remove this
/// step for a field once #438 makes that field use `/`.
pub(super) fn root_paths_with_forward_slashes(text: &str) -> String {
    const ROOT: &str = "<project-root>";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(ROOT) {
        let (before, from_root) = rest.split_at(start + ROOT.len());
        out.push_str(before);
        let end = from_root
            .find(|ch: char| {
                ch.is_whitespace() || matches!(ch, '"' | '\'' | '`' | ',' | ';' | ')' | ']' | '}')
            })
            .unwrap_or(from_root.len());
        out.push_str(&from_root[..end].replace('\\', "/"));
        rest = &from_root[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::root_paths_with_forward_slashes;

    #[test]
    fn windows_paths_under_the_root_use_forward_slashes_and_nothing_else_changes() {
        assert_eq!(
            root_paths_with_forward_slashes(
                r"# lint <project-root>\tasks\PRD-X\TASK-X-010.md parsed; keep C:\other\x and a\b"
            ),
            r"# lint <project-root>/tasks/PRD-X/TASK-X-010.md parsed; keep C:\other\x and a\b"
        );
        assert_eq!(
            root_paths_with_forward_slashes(r#"["<project-root>\.archon\<run-id>\x.json","y\z"]"#),
            r#"["<project-root>/.archon/<run-id>/x.json","y\z"]"#
        );
        let unix = "<project-root>/tasks/PRD-X (unchanged)";
        assert_eq!(root_paths_with_forward_slashes(unix), unix);
    }
}
