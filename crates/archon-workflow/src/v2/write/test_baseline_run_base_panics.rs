//! A failing test's panic, read from a libtest run's own output: where it
//! failed and why, as a signature two runs can be compared by.

use std::path::Path;

/// Each `(location, message)` of `test_id`'s panics: `thread '<id>' ...
/// panicked at <path>:<l>:<c>:` followed by the message line, or the older
/// `panicked at '<msg>', <path>:<l>:<c>`.
fn panics(output: &str, test_id: &str) -> Vec<(String, String)> {
    let opening = format!("thread '{test_id}'");
    let lines: Vec<&str> = output.lines().map(str::trim).collect();
    let mut found = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        let Some(rest) = line.strip_prefix(&opening) else {
            continue;
        };
        let Some((_, after)) = rest.split_once(" panicked at ") else {
            continue;
        };
        match after.strip_prefix('\'') {
            Some(quoted) => {
                if let Some((message, location)) = quoted.rsplit_once("', ") {
                    found.push((location.to_string(), message.to_string()));
                }
            }
            None => {
                let location = after.strip_suffix(':').unwrap_or(after);
                found.push((location.to_string(), message_after(&lines, at)));
            }
        }
    }
    found
}

/// The panic message below the header at `at`: every line up to a blank
/// one, the next panic, a `note:`, a runner's own line or a test's output
/// header -- an
/// assertion's `left:` / `right:` lines included -- at most eight.
fn message_after(lines: &[&str], at: usize) -> String {
    lines[at + 1..]
        .iter()
        .take_while(|line| {
            !line.is_empty()
                && !line.starts_with("thread '")
                && !line.starts_with("note:")
                && !line.starts_with("---- ")
                && !line.starts_with("failures:")
                && !line.starts_with("test ")
                && !line.starts_with("Running ")
        })
        .take(8)
        .copied()
        .collect::<Vec<_>>()
        .join(" / ")
}

/// `path` out of `path:line:col[...]`, when two numbers follow it.
fn location_path(location: &str) -> Option<&str> {
    let (path_line, col) = location.rsplit_once(':')?;
    let (path, line) = path_line.rsplit_once(':')?;
    let numeric = |text: &str| !text.is_empty() && text.chars().all(|c| c.is_ascii_digit());
    (!path.is_empty() && numeric(line) && numeric(col.trim())).then_some(path)
}

/// Normalize duration tokens and hex addresses, preserving identifiers and paths.
pub(super) fn normalised(text: &str) -> String {
    super::super::test_baseline_parse::mask_run_values(text)
        .trim()
        .to_string()
}

/// `test_id`'s failure signature: every panic's location and message
/// lines, normalized; empty when the output shows no panic of it (a failure
/// the host cannot compare, which therefore never matches).
#[cfg(test)]
pub(crate) fn signature(output: &str, test_id: &str) -> String {
    signature_at(output, test_id, Path::new(""))
}

pub(crate) fn signature_at(output: &str, test_id: &str, root: &Path) -> String {
    let root_pattern = super::super::test_baseline_parse::identity_root_pattern(root);
    let normalize = |text: &str| {
        let text = root_pattern.as_ref().map_or_else(
            || text.to_string(),
            |pattern| pattern.replace_all(text, "${before}<root>/").into_owned(),
        );
        normalised(&text)
    };
    panics(output, test_id)
        .into_iter()
        .map(|(location, message)| format!("{} | {}", normalize(&location), normalize(&message)))
        .collect::<Vec<_>>()
        .join(" || ")
}

/// The existing repo-relative files `test_id`'s panic locations name, read
/// under `workdir` (the tree the run was in). Sorted.
pub(crate) fn panic_files(output: &str, test_id: &str, workdir: &Path) -> Vec<String> {
    let mut files: Vec<String> = panics(output, test_id)
        .into_iter()
        .filter_map(|(location, _)| location_path(&location).map(str::to_string))
        .filter_map(|path| {
            let path = archon_write_plan::lexical_path::portable(&path);
            let relative = if archon_write_plan::lexical_path::rooted(&path) {
                std::iter::once(workdir.to_path_buf())
                    .chain(std::fs::canonicalize(workdir).ok())
                    .find_map(|root| {
                        archon_write_plan::lexical_path::under_root(&path, &root.to_string_lossy())
                    })?
            } else {
                path.trim_start_matches("./").to_string()
            };
            (!relative.split('/').any(|part| part == "..") && workdir.join(&relative).is_file())
                .then_some(relative)
        })
        .collect();
    files.sort();
    files.dedup();
    files
}
