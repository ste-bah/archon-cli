//! A Cargo manifest's test settings as pinned check sources (PLAN-11):
//!
//! - `toml:test:<name>`: the `[[test]]` entry that selects a check's
//!   integration target (its `path`, `harness`, `required-features`, ...),
//!   from its header to the next header. A target auto-discovered from
//!   `tests/` has no entry, which is pinned absent: adding one to point the
//!   target elsewhere is a pinned-source change like editing it;
//! - `toml:table:<header>`: a whole table (`[lib]`, `[profile.test]`, ...);
//! - `toml:key:<table>.<key>`: one `key = ...` line of a table
//!   (`package.autotests`).

/// The key of the `[[test]]` entry named `name`.
pub fn test_entry_key(name: &str) -> String {
    format!("toml:test:{name}")
}

/// Byte spans of every table: (start, end, header line, `name` value).
fn tables(text: &str) -> Vec<(usize, usize, String, Option<String>)> {
    let mut tables = Vec::new();
    let mut open: Option<(usize, String, Option<String>)> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if let Some((start, header, name)) = open.take() {
                tables.push((start, offset, header, name));
            }
            open = Some((offset, trimmed.to_string(), None));
        } else if let Some((_, _, name @ None)) = open.as_mut()
            && let Some(value) = trimmed
                .strip_prefix("name")
                .map(str::trim_start)
                .and_then(|rest| rest.strip_prefix('='))
        {
            *name = Some(value.trim().trim_matches('"').to_string());
        }
        offset += line.len();
    }
    if let Some((start, header, name)) = open {
        tables.push((start, text.len(), header, name));
    }
    tables
}

fn table_span(text: &str, header: &str) -> Option<(usize, usize)> {
    tables(text)
        .into_iter()
        .find(|(_, _, h, _)| h == header)
        .map(|(start, end, _, _)| (start, end))
}

/// `toml:key:<table>.<key>` as (`[table]`, key).
fn key_parts(key: &str) -> Option<(String, &str)> {
    let rest = key.strip_prefix("toml:key:")?;
    let (table, name) = rest.rsplit_once('.')?;
    Some((format!("[{table}]"), name))
}

fn span(text: &str, key: &str) -> Option<(usize, usize)> {
    if let Some(name) = key.strip_prefix("toml:test:") {
        return tables(text)
            .into_iter()
            .find(|(_, _, h, table)| h == "[[test]]" && table.as_deref() == Some(name))
            .map(|(start, end, _, _)| (start, end));
    }
    if let Some(header) = key.strip_prefix("toml:table:") {
        return table_span(text, &format!("[{header}]"));
    }
    let (header, name) = key_parts(key)?;
    let (start, end) = table_span(text, &header)?;
    let mut offset = start;
    for line in text[start..end].split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed
            .strip_prefix(name)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
        {
            return Some((offset, offset + line.len()));
        }
        offset += line.len();
    }
    None
}

/// The entry's text, trailing blank lines excluded.
pub fn entry_text(text: &str, key: &str) -> Option<String> {
    let (start, end) = span(text, key)?;
    Some(text[start..end].trim_end().to_string())
}

/// `text` with the entry replaced by `replacement`, removed when `None`, or
/// -- absent and given one -- appended.
pub fn splice_entry(text: &str, key: &str, replacement: Option<&str>) -> Option<String> {
    match (span(text, key), replacement) {
        (Some((start, end)), Some(entry)) => {
            // The blank lines after the table stay where they were.
            let tail = &text[start..end][text[start..end].trim_end().len()..];
            Some(format!("{}{entry}{tail}{}", &text[..start], &text[end..]))
        }
        (Some((start, end)), None) => Some(format!("{}{}", &text[..start], &text[end..])),
        (None, Some(entry)) if key.starts_with("toml:key:") => {
            let (header, _) = key_parts(key)?;
            Some(match table_span(text, &header) {
                Some((start, _)) => {
                    let after = start
                        + text[start..]
                            .find('\n')
                            .map_or(text.len() - start, |at| at + 1);
                    format!("{}{entry}\n{}", &text[..after], &text[after..])
                }
                None => format!("{}\n{header}\n{entry}\n", text.trim_end()),
            })
        }
        (None, Some(entry)) => {
            let sep = if text.is_empty() || text.ends_with("\n\n") {
                ""
            } else if text.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            Some(format!("{text}{sep}{entry}\n"))
        }
        (None, None) => Some(text.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "[package]\nname = \"p\"\n\n[[test]]\nname = \"a\"\npath = \"tests/a.rs\"\n\n[[test]]\nname = \"b\"\n\n[dependencies]\nx = \"1\"\n";

    #[test]
    fn an_entry_is_read_replaced_removed_and_restored() {
        let key = test_entry_key("a");
        assert_eq!(
            entry_text(MANIFEST, &key).unwrap(),
            "[[test]]\nname = \"a\"\npath = \"tests/a.rs\""
        );
        let pinned = entry_text(MANIFEST, &key).unwrap();
        let moved = MANIFEST.replace("tests/a.rs", "tests/other.rs");
        assert_eq!(splice_entry(&moved, &key, Some(&pinned)).unwrap(), MANIFEST);
        let removed = splice_entry(MANIFEST, &key, None).unwrap();
        assert!(entry_text(&removed, &key).is_none());
        assert!(entry_text(&removed, &test_entry_key("b")).is_some());
        assert!(entry_text(MANIFEST, &test_entry_key("c")).is_none());
        let added = splice_entry(
            "[package]\nname = \"p\"\n",
            &test_entry_key("c"),
            Some("[[test]]\nname = \"c\""),
        )
        .unwrap();
        assert_eq!(
            entry_text(&added, &test_entry_key("c")).unwrap(),
            "[[test]]\nname = \"c\""
        );
    }
}
