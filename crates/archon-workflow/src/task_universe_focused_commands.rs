//! Which `## Focused Tests` items are shell commands (Obs-119 follow-up).
//!
//! A Focused Tests section is markdown written for a reader: commands in
//! code spans and fenced blocks, and around them prose, fixture names,
//! status words and tool-call instructions. Every item used to become a
//! "declared command", so the host baseline ran file extensions, status
//! words, source file names and `mcp__server__tool` names as shell commands
//! (exit 127), and the author brief listed them as filters to pass.
//!
//! The rule, applied to an item's first code span (or the whole item when it
//! has none, which is what a fenced-block line is):
//!
//! - a statement led by a `NAME=value` assignment is shell, as before;
//! - an `mcp__<server>__<tool>` word is a tool to CALL, never a command. It
//!   is not added to the task's `required_tools` here: the decomposition
//!   lint (`topology_lint::tool_obligations`) already requires every focused
//!   MCP call to be declared there, so a lint-clean task's focused MCP calls
//!   are its required tools, and merging at parse would silence that lint
//!   and could make a "never call ..." line an obligation;
//! - tool-call syntax, an identifier followed by `(`, is not a command;
//! - the first word must be a program word: the shell operators `!`, `(`,
//!   `{`, `[`, `[[` and the builtins `:` and `.`; or `[A-Za-z0-9_+./:@%$-]`
//!   characters, not starting with an uppercase letter (sentence prose) or
//!   `-`, not ending in `:` (a `key:` label, as in `enabled: true`), and not
//!   a bare file name (`notes.rs`, `.ext`: a `.` outside a `./`/`../` path,
//!   unless every character after the last `.` is a digit, as in
//!   `python3.11`);
//! - a single word or a bare path is a command only when it opens the item
//!   (`` `make` `` or `` `make` must pass ``), never a word quoted inside
//!   prose (`` status `failed` ``) or a label (`` `notes.rs`: what it
//!   proves ``).
//!
//! Well-formed items (`` `cargo test -p x` ``, fenced command lines) parse
//! exactly as they did. Known limits: a lowercase prose item with no code
//! span ("run the suite"), or a leading lone identifier (`` `some_word` is
//! recorded ``), still reads as a command, and a program whose name starts
//! with an uppercase letter does not.
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
    let mut comment = false;
    let mut previous = '\n';
    for ch in text.chars() {
        let before = std::mem::replace(&mut previous, ch);
        last_escape = false;
        // An unquoted `#` opening a word comments out the rest of its line.
        if comment {
            comment = ch != '\n';
            continue;
        }
        if escaped {
            escaped = false;
            continue;
        }
        if quote.is_none() && ch == '#' && before.is_whitespace() {
            comment = true;
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
    let (candidate, position) = candidate(entry)?;
    is_shell_command(&candidate, position).then_some(candidate)
}

/// The item's first code span, or the whole item when it has none, and how
/// it sits in the item: `Alone` (the item's entire content), `Leading` (no
/// prose before it, prose after), `Label` (leading, but a `:` follows it,
/// as in `` `notes.rs`: what it proves ``) or `Embedded` (prose before it).
fn candidate(entry: &str) -> Option<(String, Position)> {
    let trimmed = entry.trim();
    let bare = |text: &str| {
        text.trim()
            .trim_matches(|c: char| ".,;:".contains(c))
            .is_empty()
    };
    let (candidate, position) = match trimmed.split_once('`') {
        Some((before, rest)) => {
            let span = rest.split('`').next().unwrap_or(rest);
            let after = rest.get(span.len() + 1..).unwrap_or("");
            let position = if !bare(before) {
                Position::Embedded
            } else if bare(after) {
                Position::Alone
            } else if after.trim_start().starts_with(':') {
                Position::Label
            } else {
                Position::Leading
            };
            (span, position)
        }
        None => (trimmed, Position::Alone),
    };
    let candidate = candidate.trim();
    (!candidate.is_empty()).then(|| (candidate.to_string(), position))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    Alone,
    Leading,
    Label,
    Embedded,
}

fn is_shell_command(candidate: &str, position: Position) -> bool {
    // A single word or a bare path is a command when it opens the item
    // (`` `make` must pass ``), not when prose quotes it (`` status `failed` ``)
    // or it labels a description (`` `notes.rs`: ... ``).
    let standalone = matches!(position, Position::Alone | Position::Leading);
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
    // A bare file name is not a program: `notes.rs`, `.ext`, `a.b_c`.
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
