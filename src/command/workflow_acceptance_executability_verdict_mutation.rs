use super::*;

pub(super) fn reported_missing(output: &str, program: &str) -> bool {
    output.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        let Some(at) = [
            "command not found",
            "not found",
            "no such file",
            "permission denied",
        ]
        .iter()
        .find_map(|report| lower.find(report)) else {
            return false;
        };
        // A program called `command` is not named by the words "command
        // not found" themselves. Only the diagnostic's operand names it.
        let operand = line[..at].trim_end_matches(|c: char| c.is_whitespace() || c == ':');
        // The final diagnostic field names the failed operand, not a wrapper
        // earlier on the line (e.g. xargs: ./tool: Permission denied).
        let operand = operand.rsplit(':').next().unwrap_or(operand).trim();
        let operand = operand
            .strip_prefix("failed to run command ")
            .unwrap_or(operand);
        let operand = operand.trim_matches(['\'', '"', '`']);
        operand == program
            || (operand.starts_with('/')
                && Path::new(operand).file_name().and_then(|s| s.to_str()) == Some(program))
    })
}

/// Only locations in this run's moved inputs, or a diagnostic naming a
/// moved file in the located file's directory, make a build failure owned.
/// No language-specific module syntax is assumed.
pub(crate) fn mutation_build_failure_owned(result: &CheckResult, moved: &[String]) -> bool {
    let output = output(result);
    let locations = compile_errors(&output);
    !locations.is_empty()
        && locations.iter().all(|location| {
            moved.iter().any(|input| {
                let input = input.trim_start_matches("./");
                let location = location.trim_start_matches("./");
                if location == input
                    || location.ends_with(&format!("/{input}"))
                    || input.ends_with(&format!("/{location}"))
                {
                    return true;
                }
                let file = Path::new(input);
                let same_parent = match (Path::new(location).parent(), file.parent()) {
                    (Some(a), Some(b)) => a == b || a.ends_with(b) || b.ends_with(a),
                    _ => false,
                };
                same_parent
                    && output
                        .lines()
                        .filter(|line| line.contains("error") || line.trim_start().starts_with('|'))
                        .any(|line| {
                            let filename = file.file_name().and_then(|s| s.to_str()).unwrap_or("");
                            let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                            line.split(|c: char| {
                                !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/'))
                            })
                            .any(|word| word == filename || word.ends_with(&format!("/{filename}")))
                                || ['`', '\'', '"']
                                    .iter()
                                    .any(|quote| line.contains(&format!("{quote}{stem}{quote}")))
                        })
            })
        })
}

pub(super) fn reported_permission_denied(output: &str, program: &str) -> bool {
    output.lines().any(|line| {
        line.to_ascii_lowercase().contains("permission denied") && reported_missing(line, program)
    })
}

/// A reported denial of an existing program is a site execution failure,
/// even when the host's own search path does not contain that program.
pub(super) fn program_failure(
    name: &str,
    code: i32,
    output: &str,
    at: &Context,
) -> (String, UnprovenCause) {
    let denied = reported_permission_denied(output, name)
        && (at.find(name).is_some() || (which::is_absolute(name) && Path::new(name).is_file()));
    let cause = if denied || at.host_has(name) {
        UnprovenCause::SiteLacks
    } else {
        UnprovenCause::ProgramAbsent
    };
    let condition = if denied {
        "is not executable at the check's site"
    } else {
        "is not on its search path"
    };
    let remedy = if denied {
        format!(
            "the site has `{name}` but cannot run it: the check must not depend on it; check the criterion with what the site provides"
        )
    } else if cause == UnprovenCause::SiteLacks {
        format!(
            "the host has `{name}`, but the check's site does not provide it: the check must not depend on it; check the criterion with what the site provides"
        )
    } else {
        // Issue 557: no host action makes a program available
        // that neither the site nor the host has.
        format!(
            "neither the check's site nor the host has `{name}`, and no host action makes it available: the check must not depend on it; if it does not mean to run `{name}` (for example a word left on a line of its own), remove it; check the criterion with what the site provides"
        )
    };
    (
        format!("it starts `{name}`, which {condition} (exit {code}); {remedy}"),
        cause,
    )
}
