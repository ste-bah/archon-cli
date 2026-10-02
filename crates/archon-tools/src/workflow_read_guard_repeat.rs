//! Issue-124: the same failing command, run again and again with nothing
//! written in between, is refused, and the refusal starts the thrash count.
//!
//! On a live run a review-remediation coder ran one
//! `python3 -c` check 500+ times, every run failing the same way. None of the
//! existing counters saw it: an interpreter command is neither an inspection
//! nor a build (`shell::inspection`, `shell::build_or_test`), so it never
//! spent the read budget, and the thrash count only runs once that budget has
//! been refused (`thrash::observe`).
//!
//! The rule here: a Bash command whose normalised text has already FAILED
//! [`IDENTICAL_FAILURES_BEFORE_REFUSAL`] times since the last substantive
//! write is refused, and the read wall is marked hit, so every call after it
//! is counted by the existing thrash rule and the session ends at its cutoff
//! unless the agent writes. A pass of the command clears its count; a file
//! write that changed the file clears every count; and a shell command that
//! may have edited a file (a write target the shell lexer can read that is
//! not `/dev/…` or scratch, or a heredoc) clears every count but its OWN — an
//! identical command that rewrites the same thing each run is not making
//! progress either. So a coder that edits — with a file tool or from the
//! shell — and re-runs its test, or runs many different commands, is never
//! refused here. Two failing checks run in strict alternation with nothing
//! between them are not caught; once the read wall is hit, the thrash rule
//! is.
//!
//! "Failed" is a non-zero exit, or — the live shape — a command whose final
//! statement echoes `$?` (`...; echo "EXIT=$?"`), which makes the shell exit 0
//! whatever the check did; the echoed status is read back from the output
//! (see [`masked_failure`]). A failure hidden any other way (`|| true`) is not
//! recognised.
use super::{State, normalise_command, shell};
use serde_json::Value;

/// Failures of one command tolerated since the last substantive write; the
/// next run of it is refused.
pub const IDENTICAL_FAILURES_BEFORE_REFUSAL: u32 = 4;

/// The prefix of the refusal, pinned by tests.
pub const REPEATED_FAILURE_MARKER: &str = "repeated failing command:";

/// The refusal for running `command` again, or `None` while it has failed
/// fewer than the limit. A refusal marks the read wall hit.
pub(super) fn admit(state: &mut State, name: &str, input: &Value) -> Option<String> {
    let command = bash_command(name, input)?;
    let failures = *state.failing_runs.get(&command)?;
    if failures < IDENTICAL_FAILURES_BEFORE_REFUSAL {
        return None;
    }
    state.wall_hit = true;
    Some(format!(
        "{REPEATED_FAILURE_MARKER} this exact command has failed {failures} times since your \
         last substantive write, and running it again cannot give a different answer. Change \
         something first: edit the file the check reads with Write, Edit or ApplyPatch, or run \
         a different command that finds out why it fails. If the check cannot pass from inside \
         your worktree, stop and report that as a blocker in your envelope. From now on every \
         call that is not a substantive write counts toward ending this session."
    ))
}

/// Fold one finished call into the counts.
pub(super) fn observe(state: &mut State, name: &str, input: &Value, exit_zero: bool) {
    let Some(command) = bash_command(name, input) else {
        return;
    };
    if may_edit(&command) {
        state.failing_runs.retain(|counted, _| counted == &command);
    }
    if exit_zero {
        state.failing_runs.remove(&command);
    } else {
        let count = state.failing_runs.entry(command).or_default();
        *count = count.saturating_add(1);
    }
}

/// Whether a command may have changed a file the next run of a check reads:
/// a write target the lexer can read off it (`sed -i`, a redirection, `tee`,
/// `cp`/`mv`, a Python `open(.., 'w')`) other than a device or scratch path
/// (`2>/dev/null`, `| tee /tmp/log` edit nothing a check reads), or a heredoc,
/// whose body the lexer does not read and which is how a shell edit is
/// usually spelled.
fn may_edit(command: &str) -> bool {
    command.contains("<<")
        || super::shell_writes::write_targets(command)
            .iter()
            .any(|write| !shell::temp_destination(&write.path))
}

fn bash_command(name: &str, input: &Value) -> Option<String> {
    (name == "Bash").then_some(())?;
    let command = normalise_command(input.get("command").and_then(Value::as_str)?);
    (!command.is_empty()).then_some(command)
}

/// Whether a command that exited 0 did so only because its final statement
/// echoes `$?`, and the status it echoed is not 0.
///
/// The template is the echo's own words (`EXIT=$?` → prefix `EXIT=`, empty
/// suffix); the LAST output line that is exactly the prefix, an integer and
/// the suffix is the echoed status. stdout precedes stderr in the output, and
/// the echo is the last thing written to stdout, so a later matching line can
/// only come from stderr, where a bare status line is not expected.
pub fn masked_failure(command: &str, output: &str) -> bool {
    let segments = shell::commands(command);
    let Some(last) = segments.last() else {
        return false;
    };
    let words: Vec<&str> = last.iter().map(String::as_str).collect();
    let Some((&"echo", args)) = words.split_first() else {
        return false;
    };
    let args: Vec<&str> = args
        .iter()
        .copied()
        .skip_while(|word| matches!(*word, "-n" | "-e" | "-E"))
        .collect();
    let template = args.join(" ");
    if template.matches("$?").count() != 1 {
        return false;
    }
    let Some((prefix, suffix)) = template.split_once("$?") else {
        return false;
    };
    output
        .lines()
        .rev()
        .find_map(|line| {
            line.trim()
                .strip_prefix(prefix)?
                .strip_suffix(suffix)?
                .parse::<i64>()
                .ok()
        })
        .is_some_and(|status| status != 0)
}
