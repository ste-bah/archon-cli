//! Whether a program a check starts printed its own message, so it ran
//! (rule 1 of `workflow_acceptance_executability_verdict`).

use super::Context;

/// Whether a line of `output` is the program `program`'s own message, so
/// it ran: the line starts with the program's name (by any path to it,
/// with or without its executable extension), then `:`, and the rest --
/// after a shell's `line N: ` or `N: ` -- is not a report that the program
/// itself was not found. The shell that runs a check may have the
/// program's own name (`/bin/sh: line 1: sh: command not found` for the
/// check `sh x.sh`), and its report that `sh` was not found is no proof.
pub(super) fn printed_by(output: &str, program: &str, at: &Context) -> bool {
    let name = |text: &str| {
        let file = text.rsplit(['/', '\\']).next().unwrap_or(text);
        at.bare_name(file)
    };
    let same = |a: &str, b: &str| match at.windows {
        true => a.eq_ignore_ascii_case(b),
        false => a == b,
    };
    let wanted = name(program);
    (output.lines()).any(|line| {
        let line = line.trim_start();
        // A drive's colon (`C:\Git\bin\bash.exe: ...`) is not the speaker's.
        let bytes = line.as_bytes();
        let drive = bytes.len() > 2
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/');
        let skip = if drive { 2 } else { 0 };
        let Some(colon) = line[skip..].find(':').map(|at| at + skip) else {
            return false;
        };
        let (speaker, said) = (&line[..colon], &line[colon + 1..]);
        let speaker = speaker.trim();
        if speaker.is_empty()
            || speaker.contains(char::is_whitespace)
            || !same(&name(speaker), &wanted)
        {
            return false;
        }
        // The program a report names, by its file name without extension
        // (`/opt/x/sh`, `bash.exe`) or as the check gave it.
        let itself = |named: &str| same(named, program) || same(&name(named), &wanted);
        !not_found(said, &itself)
    })
}

/// Whether `said`, the rest of a line after its speaker, reports that a
/// program `itself` names was not found: `[line N: ]prog: command not
/// found`, `[N: ]prog: not found` or `prog: No such file or directory`.
/// A rest that starts with `command not found` or `not found` -- tcsh's
/// `foo: Command not found.`, zsh's `command not found: foo` -- is never a
/// program's own message either, whatever it names.
fn not_found(said: &str, itself: &dyn Fn(&str) -> bool) -> bool {
    let mut rest = said.trim_start();
    if let Some(after) = rest.strip_prefix("line ") {
        rest = after;
    }
    if let Some((number, after)) = rest.split_once(": ")
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
    {
        rest = after;
    }
    const REPORTS: [&str; 3] = [
        "command not found",
        "not found",
        "no such file or directory",
    ];
    let lower = rest.to_ascii_lowercase();
    if REPORTS[..2].iter().any(|report| lower.starts_with(report)) {
        return true;
    }
    let Some((named, report)) = rest.split_once(": ") else {
        return false;
    };
    let report = report.trim().to_ascii_lowercase();
    itself(named.trim()) && REPORTS.iter().any(|known| report.starts_with(known))
}
