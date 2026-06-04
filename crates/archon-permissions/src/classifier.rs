use serde::{Deserialize, Serialize};

/// Classification of a shell command's danger level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandClass {
    Safe,
    Risky,
    Dangerous,
}

/// Default safe commands (auto-approve in auto mode).
const DEFAULT_SAFE: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "grep",
    "rg",
    "find",
    "wc",
    "git status",
    "git log",
    "git diff",
    "git branch",
    "npm test",
    "cargo test",
    "cargo check",
    "cargo clippy",
    "python -c",
    "echo",
    "pwd",
    "which",
    "env",
    "date",
    "whoami",
    "tree",
    "file",
    "stat",
    "du",
    "df",
];

/// Default risky commands (prompt in auto mode).
const DEFAULT_RISKY: &[&str] = &[
    "git commit",
    "git checkout",
    "git merge",
    "git rebase",
    "npm install",
    "pip install",
    "cargo build",
    "cargo run",
    "mkdir",
    "cp",
    "mv",
    "touch",
    "ln",
];

/// Default dangerous commands (always prompt).
const DEFAULT_DANGEROUS: &[&str] = &[
    "rm -rf",
    "rm -r",
    "rm -fr",
    "git push",
    "git push --force",
    "git reset --hard",
    "git clean",
    "sudo",
    "chmod",
    "chown",
    "chgrp",
    "kill",
    "pkill",
    "killall",
    "dd",
    "mkfs",
    "fdisk",
    "mount",
    "umount",
    "shutdown",
    "reboot",
    "halt",
    "curl | sh",
    "wget | sh",
];

/// Dangerous substrings that always trigger dangerous classification.
const DANGEROUS_SUBSTRINGS: &[&str] = &[
    "rm -rf",
    "rm -r",
    "rm -fr",
    "sudo ",
    "| sudo",
    "|sudo",
    "| rm",
    "|rm",
    "git push",
    "git reset --hard",
    "> /dev/",
    ">> /dev/",
    ":(){ :|:& };:", // fork bomb
];

/// Classify a shell command string.
pub fn classify_command(
    command: &str,
    user_safe: &[String],
    user_risky: &[String],
    user_dangerous: &[String],
) -> CommandClass {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return CommandClass::Risky;
    }

    // Check unquoted dangerous substrings first (highest priority).
    let lower = unquoted_shell_text(trimmed).to_lowercase();
    for pattern in DANGEROUS_SUBSTRINGS {
        if lower.contains(&pattern.to_lowercase()) {
            return CommandClass::Dangerous;
        }
    }

    // Check shell command chains -- each segment classified independently, worst wins.
    let segments = split_shell_chain(trimmed);
    if segments.len() > 1 {
        let mut worst = CommandClass::Safe;
        for segment in segments {
            let class = classify_command(segment.trim(), user_safe, user_risky, user_dangerous);
            worst = worse(worst, class);
        }
        return worst;
    }

    // Check quoted arguments in bash/sh -c forms, including common flag groups
    // like `bash -lc "..."`.
    if let Some(inner) = extract_shell_c_command(trimmed)
        && !inner.is_empty()
    {
        let inner_class = classify_command(inner, user_safe, user_risky, user_dangerous);
        return worse(CommandClass::Risky, inner_class);
    }

    classify_single_command(trimmed, user_safe, user_risky, user_dangerous)
}

/// Classify a single command (no pipes).
fn classify_single_command(
    command: &str,
    user_safe: &[String],
    user_risky: &[String],
    user_dangerous: &[String],
) -> CommandClass {
    let lower = command.to_lowercase();

    // User overrides take priority (check longest match first)
    for cmd in user_dangerous {
        if lower.starts_with(&cmd.to_lowercase()) {
            return CommandClass::Dangerous;
        }
    }
    for cmd in user_safe {
        if lower.starts_with(&cmd.to_lowercase()) {
            return CommandClass::Safe;
        }
    }
    for cmd in user_risky {
        if lower.starts_with(&cmd.to_lowercase()) {
            return CommandClass::Risky;
        }
    }

    // Check defaults
    for cmd in DEFAULT_DANGEROUS {
        if lower.starts_with(&cmd.to_lowercase()) {
            return CommandClass::Dangerous;
        }
    }
    for cmd in DEFAULT_SAFE {
        if lower.starts_with(&cmd.to_lowercase()) {
            return CommandClass::Safe;
        }
    }
    for cmd in DEFAULT_RISKY {
        if lower.starts_with(&cmd.to_lowercase()) {
            return CommandClass::Risky;
        }
    }

    // Unknown commands default to risky
    CommandClass::Risky
}

/// Return the more dangerous of two classifications.
fn worse(a: CommandClass, b: CommandClass) -> CommandClass {
    match (a, b) {
        (CommandClass::Dangerous, _) | (_, CommandClass::Dangerous) => CommandClass::Dangerous,
        (CommandClass::Risky, _) | (_, CommandClass::Risky) => CommandClass::Risky,
        _ => CommandClass::Safe,
    }
}

fn split_shell_chain(command: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let bytes = command.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    while i < command.len() {
        let ch = command[i..].chars().next().expect("valid char boundary");
        let width = ch.len_utf8();

        if escaped {
            escaped = false;
            i += width;
            continue;
        }
        if ch == '\\' && !in_single {
            escaped = true;
            i += width;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            i += width;
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            i += width;
            continue;
        }

        if !in_single && !in_double {
            let op_len = if ch == '&' && bytes.get(i + 1) == Some(&b'&') {
                2
            } else if ch == '|' && bytes.get(i + 1) == Some(&b'|') {
                2
            } else if ch == '|' || ch == ';' || ch == '\n' {
                1
            } else {
                0
            };

            if op_len > 0 {
                segments.push(command[start..i].trim());
                i += op_len;
                start = i;
                continue;
            }
        }

        i += width;
    }

    segments.push(command[start..].trim());
    segments
}

fn unquoted_shell_text(command: &str) -> String {
    let mut text = String::with_capacity(command.len());
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for ch in command.chars() {
        if escaped {
            if !in_single && !in_double {
                text.push(ch);
            }
            escaped = false;
            continue;
        }
        if ch == '\\' && !in_single {
            escaped = true;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            text.push(' ');
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            text.push(' ');
            continue;
        }
        if in_single || in_double {
            continue;
        }
        text.push(ch);
    }

    text
}

/// Extract the command from shell invocations such as:
/// - `bash -c "..."`
/// - `bash -lc "..."`
/// - `sh -c '...'`
fn extract_shell_c_command(full: &str) -> Option<&str> {
    let trimmed = full.trim_start();
    let after_shell = trimmed
        .strip_prefix("bash")
        .or_else(|| trimmed.strip_prefix("sh"))?;
    if !after_shell.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }

    let mut rest = after_shell.trim_start();
    while let Some(flag_rest) = rest.strip_prefix('-') {
        let token_end = flag_rest
            .find(char::is_whitespace)
            .unwrap_or(flag_rest.len());
        let flags = &flag_rest[..token_end];
        let after_flags = flag_rest[token_end..].trim_start();
        if flags.contains('c') {
            return Some(strip_outer_shell_quotes(after_flags));
        }
        rest = after_flags;
    }

    None
}

fn strip_outer_shell_quotes(value: &str) -> &str {
    let trimmed = value.trim();

    if let Some(rest) = trimmed.strip_prefix('"') {
        rest.strip_suffix('"').unwrap_or(rest)
    } else if let Some(rest) = trimmed.strip_prefix('\'') {
        rest.strip_suffix('\'').unwrap_or(rest)
    } else {
        trimmed
    }
}
