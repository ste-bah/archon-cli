use super::super::verdict_shell::Simple;

/// An exit-2 shell option error can happen before the criterion starts.
/// Recognize invocation rejection and a rejected first `set` command only;
/// later failures remain the check's verdict.
pub(super) fn refusal(
    code: i32,
    command: &str,
    commands: &[Simple],
    output: &str,
) -> Option<String> {
    let has_option_refusal_wording = |line: &str| {
        let line = line.to_ascii_lowercase();
        [
            "invalid option",
            "illegal option",
            "unknown option",
            "bad option",
        ]
        .iter()
        .any(|wording| line.contains(wording))
    };
    if code != 2 {
        return None;
    }
    if commands
        .first()
        .and_then(|first| first.program.as_deref())
        .is_some_and(|program| {
            matches!(
                program.rsplit(['/', '\\']).next().unwrap_or(program),
                "sh" | "bash" | "dash" | "ash" | "ksh"
            )
        })
        && output.lines().any(|line| {
            let line = line.trim_start();
            let Some((speaker, message)) = line.split_once(": ") else {
                return false;
            };
            let speaker = speaker.rsplit(['/', '\\']).next().unwrap_or(speaker);
            matches!(speaker, "sh" | "bash" | "dash" | "ash" | "ksh")
                && has_option_refusal_wording(message)
        })
    {
        return Some(
            "the shell rejected an invocation option before reading the check body".to_string(),
        );
    }
    if commands
        .first()
        .is_some_and(|first| first.program.is_none())
        && command.trim_start().starts_with("set ")
        && output.lines().any(|line| {
            let line = line.trim_start();
            (line.starts_with("set:") || line.contains(": set:"))
                && has_option_refusal_wording(line)
        })
    {
        return Some(
            "the shell rejected an option in the check's first command before any criterion could run"
                .to_string(),
        );
    }
    None
}
