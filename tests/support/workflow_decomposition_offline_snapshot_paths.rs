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

// The inherited/default site PATH varies by test host. Author shape tests
// verify this fact; lifecycle snapshots compare all the other prompt bytes.
fn site_path_template() -> &'static str {
    include_str!("../../src/command/acceptance_scratch_policy.rs")
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix('"')
                .and_then(|line| line.strip_suffix('"'))
                .filter(|line| line.contains("PATH={path}"))
        })
        .expect("host site PATH sentence")
}

pub(super) fn without_site_path_fact(text: &str) -> String {
    let (prefix, suffix) = site_path_template().split_once("{path}").unwrap();
    text.split_inclusive('\n')
        .filter(|line| {
            let line = line.strip_suffix('\n').unwrap_or(line);
            !(line.starts_with(prefix) && line.ends_with(suffix))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{root_paths_with_forward_slashes, without_site_path_fact};

    #[test]
    fn only_the_complete_host_path_fact_is_excluded_from_lifecycle_snapshots() {
        let fact = super::site_path_template().replace("{path}", "/fixture/bin");
        assert_eq!(
            without_site_path_fact(&format!("before\n{fact}\nafter")),
            "before\nafter"
        );
        assert_eq!(
            without_site_path_fact(
                "Checks run at the check site with PATH=other instructions\nafter"
            ),
            "Checks run at the check site with PATH=other instructions\nafter"
        );
        assert_eq!(
            without_site_path_fact(&format!("quoted: {fact}")),
            format!("quoted: {fact}")
        );
    }

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
