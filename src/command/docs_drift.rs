//! Documentation drift checks for slash-command references.

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    use archon_core::skills::builtin::register_builtins;

    use crate::command::registry::default_registry;

    #[test]
    fn docs_do_not_reference_unknown_slash_commands() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/reference/slash-commands.md");
        let markdown = fs::read_to_string(path).expect("slash command docs exist");
        let documented = documented_slash_names(&markdown);
        let registry = default_registry();
        let skills = register_builtins();

        assert!(
            !documented.is_empty(),
            "slash command docs yielded no names"
        );

        let unknown: Vec<_> = documented
            .into_iter()
            .filter(|name| {
                !registry.is_primary(name)
                    && registry.primary_for_alias(name).is_none()
                    && skills.resolve(name).is_none()
            })
            .collect();

        assert!(
            unknown.is_empty(),
            "docs/reference/slash-commands.md references unknown slash commands/skills: {unknown:?}"
        );
    }

    fn documented_slash_names(markdown: &str) -> BTreeSet<String> {
        markdown
            .lines()
            .filter_map(command_table_cell)
            .flat_map(backticked_slash_names)
            .collect()
    }

    fn command_table_cell(line: &str) -> Option<&str> {
        let trimmed = line.trim();
        if !trimmed.starts_with("| `/") {
            return None;
        }

        trimmed.split('|').nth(1)
    }

    fn backticked_slash_names(cell: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = cell;

        while let Some(start) = rest.find("`/") {
            let after_slash = &rest[start + 2..];
            let end = after_slash
                .find(|c: char| c == '`' || c.is_whitespace() || c == '<' || c == '(')
                .unwrap_or(after_slash.len());
            let name = &after_slash[..end];
            if !name.is_empty() {
                names.push(name.to_string());
            }
            rest = &after_slash[end..];
        }

        names
    }
}
