//! Which `## Focused Tests` items are shell commands, and which name an MCP
//! tool to call instead (Obs-119 follow-up).
//!
//! A Focused Tests section is markdown written for a reader: commands in
//! code spans and fenced blocks, and around them prose, fixture names,
//! status words and tool-call instructions. Every item used to become a
//! "declared command", so the host baseline ran `.pine`, `captured_error`,
//! `validation_report.rs` and `mcp__server__tool` as shell commands (exit
//! 127), and the author brief listed them as filters to pass.
//!
//! The rule, applied to an item's first code span (or the whole item when it
//! has none, which is what a fenced-block line is):
//!
//! - a statement led by a `NAME=value` assignment is shell, as before;
//!
//! - an `mcp__<server>__<tool>` word is a TOOL to call, never a command: it
//!   is classified as one of the task's required tools
//!   ([`focused_test_tool`]) when it LEADS the item (the item instructs the
//!   call); a tool named later in prose ("never call ...") is not;
//! - tool-call syntax, an identifier followed by `(`, is not a command;
//! - the first word must be a program word: shell operators `!`, `(`, `{`,
//!   `[`, `[[` and the builtins `:` and `.`; or `[A-Za-z0-9_+./:@%$-]` characters, not starting with an
//!   uppercase letter (sentence prose) or `-`, not ending in `:` (a `key:`
//!   label, as in `enabled: true`), and not a bare file name
//!   (`notes.rs`, `.pine`: a `.` outside a `./`/`../` path, unless every
//!   character after the last `.` is a digit, as in `python3.11`);
//! - a single word, or a path alone, is a command only when it is the whole
//!   item (`` `make` ``), never a word quoted inside prose (`` `failed` ``).
//!
//! Well-formed items (`` `cargo test -p x` ``, fenced command lines) parse
//! exactly as they did. A lowercase prose item with no code span ("run the
//! suite") still reads as a command: nothing in its text says otherwise.
//!
//! A fenced command a shell would continue onto the next line (an open
//! quote, a trailing `\`) is one command, joined with its continuation
//! lines ([`continues`]); it used to be split into one "command" per line.

/// Whether a shell reading `text` would still be inside a quoted string or
/// a trailing-backslash continuation at its end.
pub(crate) fn continues(text: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut last_escape = false;
    for ch in text.chars() {
        last_escape = false;
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, ch) {
            (Some('\''), '\'') => quote = None,
            (Some('\''), _) => {}
            (_, '\\') => {
                escaped = true;
                last_escape = true;
            }
            (Some('"'), '"') => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(ch),
            (None, _) => {}
        }
    }
    quote.is_some() || (escaped && last_escape)
}

/// The command out of one declared focused-test item, or `None` when the
/// item names no shell command.
pub(crate) fn focused_test_command(entry: &str) -> Option<String> {
    let (candidate, standalone, _) = candidate(entry)?;
    is_shell_command(&candidate, standalone).then_some(candidate)
}

/// The MCP tool a focused-test item instructs the agent to call, when its
/// leading word is one (`` `mcp__srv__tool` — inputs: ... ``), with any
/// call parentheses dropped.
pub(crate) fn focused_test_tool(entry: &str) -> Option<String> {
    let (candidate, _, leading) = candidate(entry)?;
    if !leading {
        return None;
    }
    let word = candidate.split_whitespace().next()?;
    let name = word.split('(').next().unwrap_or(word);
    mcp_tool_name(name).then(|| name.to_string())
}

/// A task's declared required tools, with the MCP tools its Focused Tests
/// items instruct it to call added (sorted, deduplicated).
pub(crate) fn with_focused_test_tools(declared: Vec<String>, focused: &[String]) -> Vec<String> {
    let mut tools = declared;
    tools.extend(focused.iter().filter_map(|entry| focused_test_tool(entry)));
    tools.sort();
    tools.dedup();
    tools
}

/// The item's first code span, or the whole item when it has none; whether
/// that is the item's entire content; and whether it LEADS the item (no
/// prose before it).
fn candidate(entry: &str) -> Option<(String, bool, bool)> {
    let trimmed = entry.trim();
    let bare = |text: &str| {
        text.trim()
            .trim_matches(|c: char| ".,;:".contains(c))
            .is_empty()
    };
    let (candidate, standalone, leading) = match trimmed.split_once('`') {
        Some((before, rest)) => {
            let span = rest.split('`').next().unwrap_or(rest);
            let after = rest.get(span.len() + 1..).unwrap_or("");
            (span, bare(before) && bare(after), bare(before))
        }
        None => (trimmed, true, true),
    };
    let candidate = candidate.trim();
    (!candidate.is_empty()).then(|| (candidate.to_string(), standalone, leading))
}

fn is_shell_command(candidate: &str, standalone: bool) -> bool {
    let mut words = candidate.split_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    // An assignment-led statement (`v=/dir`, `d=$(mktemp -d) && ...`,
    // `RUST_LOG=x cargo test`) is shell: its value cannot be split on
    // whitespace reliably, so it is not judged further.
    if assignment(first) {
        return true;
    }
    let has_arguments = words.next().is_some();
    // Shell operators and the `:` / `.` builtins.
    if matches!(first, "!" | "(" | "{" | "[" | "[[" | "." | ":") {
        return has_arguments || (first == ":" && standalone);
    }
    // A subshell spelled against its first word: `(cd dir && make)`.
    let first = first.trim_start_matches('(');
    if first.is_empty() || mcp_tool_name(first.split('(').next().unwrap_or(first)) {
        return false;
    }
    if tool_call_syntax(first) || !program_word(first) {
        return false;
    }
    has_arguments || standalone
}

/// `NAME=value`, NAME a shell identifier.
fn assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// `mcp__<server>__<tool>`, the MCP tool naming the host binds.
fn mcp_tool_name(word: &str) -> bool {
    word.strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
        .is_some_and(|(server, tool)| {
            !server.is_empty()
                && !tool.is_empty()
                && tool.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// `name(` or `name()`: an identifier immediately followed by a parenthesis.
fn tool_call_syntax(word: &str) -> bool {
    word.split_once('(').is_some_and(|(name, _)| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | ':'))
    })
}

fn program_word(word: &str) -> bool {
    let Some(first) = word.chars().next() else {
        return false;
    };
    // Sentence prose, an option, or a `key:` label (`enabled: true`).
    if first.is_ascii_uppercase() || first == '-' || word.ends_with(':') {
        return false;
    }
    if !word
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_+./:@%$-".contains(c))
    {
        return false;
    }
    if word.contains('/') {
        // A path: `./x.sh`, `scripts/check`, `/usr/bin/env`. A leading `.`
        // must open a relative path.
        return first != '.' || word.starts_with("./") || word.starts_with("../");
    }
    // A bare file name is not a program: `notes.rs`, `.pine`, `a.b_c`.
    match word.rsplit_once('.') {
        None => true,
        Some((stem, suffix)) => {
            !stem.is_empty() && !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
        }
    }
}

#[cfg(test)]
#[path = "task_universe_focused_commands_tests.rs"]
mod tests;
