//! The evidence of a failed command that an agent is shown.
//!
//! A bare byte tail of a long output loses the line that states the failure
//! whenever anything follows it, and a head-slice of that tail keeps only
//! the noise before it (a `cargo run` prints hundreds of warning lines, then
//! the program's one `Error:` line). The excerpt built here always carries
//! the END of the output and every line that states a failure -- detected
//! generically, not per language -- and never exceeds its byte budget.
//! Consumers pass it on verbatim; they never cut it again.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

/// Longest single line the excerpt keeps whole; a longer one keeps its head
/// and its end around a marker.
const MAX_LINE_BYTES: usize = 400;

/// Below this budget no structure fits: the excerpt is the output's end.
const MIN_STRUCTURED_BUDGET: usize = 240;

/// A line stating a failure, in any of the common shapes: `error:`,
/// `Error: x`, `error[E0308]:`, `error TS2322:`, `FATAL`, `ValueError`,
/// `Exception:`, `panicked at`, `Traceback`, assertions and their values,
/// `FAILED`/`FAIL`, `failed`, `not ok`, shell and loader failures, `make`'s
/// `Error 2`, `npm ERR!`, Go's `file.go:12: msg`, Jest's `●`, aborts and
/// signals.
static FAILURE_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\b(?i:error|fatal)(?:\[[^\]]*\]|\s+[A-Z]+\d+)?\s*[:!]",
        r"|^\s*(?i:error|fatal|panic)\b|^\s*(?:Exception|Killed|Terminated)\b",
        r"|\w(?:Error|Exception)\b|\bError\s+\d+\b|\bERR!",
        r"|\bpanicked\b|\bpanic:|\bTraceback\b",
        r"|(?i:\bassert)|^\s*(?:left|right|Expected|Received|expected|received|actual)\b\s*:|^\s*●",
        r"|\bFAIL(?:ED|URE|S)?\b|\b(?:[Ff]ailed|[Ff]ailure)\b|^\s*not ok\b",
        r"|\bcommand not found\b|:\s*not found\b|No such file or directory|Permission denied",
        r"|\bundefined(?::| reference\b)|\bUndefined symbols?\b|\bcannot find\b",
        r"|^\s*[\w./-]*\.[A-Za-z]\w*:\d+(?::\d+)?:\s",
        r"|\b(?:Abort(?:ed)?|Segmentation fault|core dumped|Uncaught|[Uu]nhandled)\b",
    ))
    .expect("failure line pattern")
});

/// Lines that restate code or report success, never a failure: warning,
/// note and help diagnostics (bare or after a `file:line:` location), a
/// compiler's source-excerpt gutter and span lines, and passing-test lines.
static NOISE_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^\s*(?:\S+:\d+(?::\d+)?:\s*)?(?:(?i:warning|warn|note|help)\b|\w*Warning\b)",
        r"|^\s*(?:\d*\s*\||-->|=\s*(?:note|help)\b)",
        r"|\btest result: ok\b|\.\.\. ok\s*$|^\s*ok\s|\b0 failed\b|^\s*PASS\b",
    ))
    .expect("noise pattern")
});

/// Whether `line` states a failure (and is not warning or success noise).
pub fn is_failure_line(line: &str) -> bool {
    !NOISE_LINE.is_match(line) && FAILURE_LINE.is_match(line)
}

/// The excerpt of `bytes` an agent is shown: the whole output when it fits
/// `budget` bytes; otherwise every error line before the tail (each with
/// its line number) followed by the output's last lines, never more than
/// `budget` bytes in all.
pub fn failure_evidence(bytes: &[u8], budget: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_end();
    if text.len() <= budget {
        return text.to_string();
    }
    if budget < MIN_STRUCTURED_BUDGET {
        return end_of(text, budget).to_string();
    }
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let header = |earlier: bool, shown: usize| {
        let what = if earlier {
            "earlier error lines, then its"
        } else {
            "its"
        };
        format!(
            "[output: {total} lines, {} bytes; {what} last {shown} lines]",
            text.len()
        )
    };
    let omitted_marker = |omitted: usize| format!("[{omitted} more error lines omitted]");
    let last_marker =
        |start: usize| format!("[last {} lines, L{}-L{total}]", total - start, start + 1);
    // The markers at their longest (no count exceeds `total`), each with its
    // newline: what they can take is set aside exactly.
    let overhead = header(true, total).len()
        + omitted_marker(total).len()
        + format!("[last {total} lines, L{total}-L{total}]").len()
        + 3;
    let content = budget.saturating_sub(overhead);
    // No single line may take more than a third of the content.
    let limit = MAX_LINE_BYTES.min(content / 3).max(16);
    // The tail is guaranteed at least half the content; error lines before
    // it may take the rest, and whatever they leave goes back to the tail.
    let floor_start = tail_start(&lines, content / 2, limit);
    let floor_bytes: usize = lines[floor_start..]
        .iter()
        .map(|line| clip_line(line, limit).len() + 1)
        .sum();
    let room = content.saturating_sub(floor_bytes);
    let (picked, dropped, used) = failure_lines(&lines[..floor_start], room, limit);
    let start = tail_start(&lines, content - used, limit).min(floor_start);
    let earlier: Vec<&(usize, String)> =
        picked.iter().filter(|(index, _)| *index < start).collect();
    let omitted = dropped.iter().filter(|index| **index < start).count();
    let structured = !earlier.is_empty() || omitted > 0;
    let mut out = String::with_capacity(budget);
    out.push_str(&header(structured, total - start));
    out.push('\n');
    if structured {
        for (_, line) in earlier {
            out.push_str(line);
            out.push('\n');
        }
        if omitted > 0 {
            out.push_str(&omitted_marker(omitted));
            out.push('\n');
        }
        out.push_str(&last_marker(start));
        out.push('\n');
    }
    for line in &lines[start..] {
        out.push_str(&clip_line(line, limit));
        out.push('\n');
    }
    let out = out.trim_end();
    // Every part above is accounted for; should that ever fail, the END is
    // what survives.
    end_of(out, budget).to_string()
}

/// Index of the first line of the longest suffix whose clipped lines fit
/// `room` bytes; always at least the last line.
fn tail_start(lines: &[&str], room: usize, limit: usize) -> usize {
    let mut used = 0;
    let mut start = lines.len();
    while start > 0 {
        let cost = clip_line(lines[start - 1], limit).len() + 1;
        if used + cost > room && start < lines.len() {
            break;
        }
        used += cost;
        start -= 1;
    }
    start
}

/// The failure lines of `lines` (index, text), deduplicated, fitted to `room`
/// bytes: the first three kept, the rest filled from the end backwards
/// (nearest the exit). Each keeps its line text raw at the start (so a
/// reader of `file:line: error` shapes still parses it), its line number
/// after it, and the location line that directly follows it, if any
/// (`--> file:line`, `at frame`, `File "x", line n`). Returns them in output
/// order, the indices left out, and the bytes used.
fn failure_lines(
    lines: &[&str],
    room: usize,
    limit: usize,
) -> (Vec<(usize, String)>, Vec<usize>, usize) {
    let mut seen = HashSet::new();
    let found: Vec<(usize, String)> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| is_failure_line(line))
        .filter(|(_, line)| seen.insert(line.trim()))
        .map(|(index, line)| {
            let mut text = format!("{}  [L{}]", clip_line(line.trim_end(), limit), index + 1);
            if let Some(next) = lines.get(index + 1).filter(|next| is_location_line(next)) {
                text.push('\n');
                text.push_str(&clip_line(next.trim_end(), limit));
            }
            (index, text)
        })
        .collect();
    let cost = |(_, text): &(usize, String)| text.len() + 1;
    let mut keep = vec![false; found.len()];
    let mut used = 0;
    let order = (0..found.len().min(3)).chain((3..found.len()).rev());
    for position in order {
        let each = cost(&found[position]);
        if used + each > room {
            continue;
        }
        used += each;
        keep[position] = true;
    }
    let (picked, dropped): (Vec<_>, Vec<_>) =
        found.into_iter().zip(keep).partition(|(_, kept)| *kept);
    let picked = picked.into_iter().map(|(entry, _)| entry).collect();
    let dropped = dropped.into_iter().map(|((index, _), _)| index).collect();
    (picked, dropped, used)
}

/// A line naming where the failure above it happened.
fn is_location_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("-->") || trimmed.starts_with("at ") || trimmed.starts_with("File \"")
}

/// `line` whole when it fits `limit` bytes, else its head and its end.
fn clip_line(line: &str, limit: usize) -> String {
    if line.len() <= limit {
        return line.to_string();
    }
    let half = limit / 2;
    let head = floor_boundary(line, half);
    let tail = ceil_boundary(line, line.len() - half);
    format!("{} [...] {}", &line[..head], &line[tail..])
}

/// The last at most `budget` bytes of `text`, on a character boundary.
fn end_of(text: &str, budget: usize) -> &str {
    &text[ceil_boundary(text, text.len().saturating_sub(budget))..]
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
#[path = "failure_evidence_tests.rs"]
mod tests;
