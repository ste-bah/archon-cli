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
    let mut parts = location.split(':');
    let path = parts.next()?.trim();
    let line = parts.next()?;
    let col = parts.next()?;
    let numeric = |text: &str| !text.is_empty() && text.chars().all(|c| c.is_ascii_digit());
    (!path.is_empty() && numeric(line) && numeric(col.trim())).then_some(path)
}

/// Absolute paths and temporary names replaced, so two runs of one failure
/// in different directories read alike.
fn normalised(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            let bare = word.trim_matches(|c: char| "\"'`()[],;".contains(c));
            if bare.starts_with('/') || bare.contains(".tmp") || bare.contains("/tmp/") {
                "<path>"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `test_id`'s failure signature: every panic's location and first message
/// line, normalised; empty when the output shows no panic of it (a failure
/// the host cannot compare, which therefore never matches).
pub(crate) fn signature(output: &str, test_id: &str) -> String {
    panics(output, test_id)
        .into_iter()
        .map(|(location, message)| format!("{} | {}", normalised(&location), normalised(&message)))
        .collect::<Vec<_>>()
        .join(" || ")
}

/// The existing repo-relative files `test_id`'s panic locations name, read
/// under `workdir` (the tree the run was in). Sorted.
pub(crate) fn panic_files(output: &str, test_id: &str, workdir: &Path) -> Vec<String> {
    let mut files: Vec<String> = panics(output, test_id)
        .into_iter()
        .filter_map(|(location, _)| location_path(&location).map(str::to_string))
        .filter(|path| !path.starts_with('/') && workdir.join(path).is_file())
        .collect();
    files.sort();
    files.dedup();
    files
}
