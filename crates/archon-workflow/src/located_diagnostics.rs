//! Located diagnostic formats shared by output readers.
use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Error,
    Other,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub path: String,
    pub line: usize,
    /// Zero-based position in the captured output.
    pub output_line: usize,
    pub kind: Kind,
}
static COLUMN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:\[[A-Z]+\]\s+)?(?P<path>(?:[A-Za-z]:)?[^\s:()]+):(?P<line>\d+):(?P<col>\d+)(?::|\s+-\s+|\s)").expect("static pattern")
});
static BRACKET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:\[[A-Z]+\]\s+)?(?P<path>(?:[A-Za-z]:)?[^\s:()\[\]]+):?[\[(](?P<line>\d+),(?P<col>\d+)[\])]:?\s*").expect("static pattern")
});
static PAREN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<path>\S+?)(?:\((?P<line>\d+),\d+\):?|:(?P<row>\d+):)\s+(?:fatal\s+)?error\b")
        .expect("static pattern")
});
static SPAN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^-->\s+(?P<path>.+?):(?P<line>\d+)(?::\d+)?\s*$").expect("static pattern")
});
static FRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:^|\s)File "(?P<path>[^"<][^"]*)", line (?P<line>\d+)\s*$"#)
        .expect("static pattern")
});

pub fn located_lines(output: &str) -> Vec<Located> {
    let mut found = Vec::new();
    let mut header = None;
    for (output_line, raw) in output.lines().enumerate() {
        let line = raw.trim_end_matches('\r').trim_start();
        if line.is_empty() {
            header = None;
            continue;
        }
        let mut header_line = false;
        for level in ["error", "warning", "note", "info", "hint", "help"] {
            if line
                .strip_prefix(level)
                .is_some_and(|rest| rest.starts_with([':', '[']))
            {
                header_line = true;
                header = Some(if level == "error" {
                    Kind::Error
                } else {
                    Kind::Other
                });
            }
        }
        if let Some(caps) = SPAN.captures(line) {
            if let Some(kind) = header {
                push(&mut found, &caps, kind, output_line);
            }
            continue;
        }
        if let Some(caps) = COLUMN
            .captures(line)
            .or_else(|| BRACKET.captures(line))
            .or_else(|| PAREN.captures(line))
        {
            let rest = line[caps.get(0).unwrap().end()..].trim_start();
            let word = rest
                .split(|ch: char| !ch.is_ascii_alphabetic())
                .next()
                .unwrap_or("");
            let kind = if ["warning", "note", "info", "hint", "help"]
                .iter()
                .any(|level| word.eq_ignore_ascii_case(level))
            {
                Kind::Other
            } else {
                Kind::Error
            };
            push(&mut found, &caps, kind, output_line);
        } else if let Some(caps) = FRAME.captures(line) {
            push(&mut found, &caps, Kind::Error, output_line);
        }
        if !raw.starts_with(char::is_whitespace) && !header_line {
            header = None;
        }
    }
    found
}
fn push(found: &mut Vec<Located>, caps: &regex::Captures<'_>, kind: Kind, output_line: usize) {
    if caps["path"].chars().all(|ch| ch.is_ascii_digit()) {
        return;
    }
    if let Some(line) = caps
        .name("line")
        .or_else(|| caps.name("row"))
        .and_then(|n| n.as_str().parse().ok())
    {
        found.push(Located {
            path: caps["path"].to_string(),
            line,
            output_line,
            kind,
        });
    }
}

#[cfg(test)]
#[path = "located_diagnostics_tests.rs"]
mod tests;
