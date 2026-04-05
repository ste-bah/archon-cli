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

    // Check for dangerous substrings first (highest priority)
    let lower = trimmed.to_lowercase();
    for pattern in DANGEROUS_SUBSTRINGS {
        if lower.contains(&pattern.to_lowercase()) {
            return CommandClass::Dangerous;
        }
    }

    // Check pipe chains -- each segment classified independently, worst wins
    if trimmed.contains('|') {
        let segments: Vec<&str> = trimmed.split('|').collect();
        let mut worst = CommandClass::Safe;
        for segment in &segments {
            let class =
                classify_single_command(segment.trim(), user_safe, user_risky, user_dangerous);
            worst = worse(worst, class);
        }
        return worst;
    }

    // Check quoted arguments in bash -c / sh -c
    if (trimmed.starts_with("bash -c") || trimmed.starts_with("sh -c")) && trimmed.len() > 8 {
        // Extract the quoted command and classify it
        let inner = extract_quoted_command(trimmed);
        if !inner.is_empty() {
            let inner_class = classify_command(inner, user_safe, user_risky, user_dangerous);
            return worse(CommandClass::Risky, inner_class);
        }
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

/// Extract the command from bash -c "..." or bash -c '...'
fn extract_quoted_command(full: &str) -> &str {
    // Find the first quote after -c
    let after_c = if let Some(idx) = full.find("-c") {
        &full[idx + 2..]
    } else {
        return "";
    };

    let trimmed = after_c.trim();

    if let Some(rest) = trimmed.strip_prefix('"') {
        rest.strip_suffix('"').unwrap_or(rest)
    } else if let Some(rest) = trimmed.strip_prefix('\'') {
        rest.strip_suffix('\'').unwrap_or(rest)
    } else {
        trimmed
    }
}
