//! Outcomes depend on execution and diagnostic shapes, never program names.
use super::*;
use archon_workflow::breakage_proof::{Located, LocatedClass, RunOutcome};
use regex::Regex;
use std::sync::LazyLock;
static COLUMN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:\[[A-Z]+\]\s+)?(?P<path>(?:[A-Za-z]:)?[^\s:()]+):(?P<line>\d+):(?P<col>\d+)(?::|\s+-\s+|\s)").expect("static pattern")
});
static BRACKET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:\[[A-Z]+\]\s+)?(?P<path>(?:[A-Za-z]:)?[^\s:()\[\]]+):?[\[(](?P<line>\d+),(?P<col>\d+)[\])]:?\s*").expect("static pattern")
});
// R2 debt: this parse-stop frame is one runtime's shape; review in S8.
static STOP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:^|\s)File "(?P<path>[^"<][^"]*)", line (?P<line>\d+)\s*$"#)
        .expect("static pattern")
});
fn line_number(text: &str, path: &str) -> u32 {
    text.lines()
        .find_map(|line| {
            let direct = line
                .strip_prefix(path)
                .and_then(|rest| rest.strip_prefix(':').or_else(|| rest.strip_prefix('(')));
            let span = line
                .trim_start()
                .strip_prefix("-->")
                .map(str::trim_start)
                .and_then(|rest| rest.strip_prefix(path))
                .and_then(|rest| rest.strip_prefix(':'));
            let frame = line
                .trim_start()
                .strip_prefix("File \"")
                .and_then(|rest| rest.strip_prefix(path))
                .and_then(|rest| rest.strip_prefix("\", line "));
            direct.or(span).or(frame).and_then(|rest| {
                rest.split(|c: char| !c.is_ascii_digit())
                    .next()?
                    .parse()
                    .ok()
            })
        })
        .unwrap_or(0)
}
pub(super) fn locations(
    text: &str,
    resolve: &dyn Fn(&str) -> Option<(String, LocatedClass)>,
) -> Vec<Located> {
    let text = text
        .lines()
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n");
    // R2 debt: the gutter and runtime-frame suffix below are single-runner shapes; review in S8.
    let normalized = text
        .lines()
        .filter(|l| !(l.trim_start().starts_with("File \"") && l.contains(", in ")))
        .map(|l| l.strip_prefix("E   ").unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n");
    let mut found: Vec<_> = verdict::located_lines(&normalized)
        .into_iter()
        .map(|path| {
            let line = line_number(&normalized, &path);
            let (path, class) = resolve(&path).unwrap_or((path, LocatedClass::Unresolved));
            Located { path, line, class }
        })
        .collect();
    for line in text.lines() {
        for form in [&*COLUMN, &*BRACKET, &*STOP] {
            let Some(c) = form.captures(line) else {
                continue;
            };
            let prefix = line
                .strip_prefix('[')
                .and_then(|s| s.split_once(']'))
                .map(|(p, _)| p)
                .unwrap_or("");
            if ["warning", "note", "info", "hint", "help"]
                .iter()
                .any(|w| prefix.eq_ignore_ascii_case(w))
            {
                continue;
            }
            let tail = &line[c.get(0).expect("matched capture").end()..];
            let word = tail
                .trim_start()
                .split(|c: char| !c.is_ascii_alphabetic())
                .next()
                .unwrap_or("");
            if ["warning", "note", "info", "hint", "help"]
                .iter()
                .any(|w| word.eq_ignore_ascii_case(w))
            {
                continue;
            }
            let explicit = prefix.eq_ignore_ascii_case("error")
                || ["error", "fatal"]
                    .iter()
                    .any(|w| word.eq_ignore_ascii_case(w));
            if let Some((path, class)) = resolve(&c["path"])
                .or_else(|| explicit.then(|| (c["path"].into(), LocatedClass::Unresolved)))
            {
                let located = Located {
                    path,
                    line: c["line"].parse().unwrap_or(0),
                    class,
                };
                if !found
                    .iter()
                    .any(|old| old.path == located.path && old.line == located.line)
                {
                    found.push(located);
                }
            }
        }
    }
    found
}
pub(super) fn decide(
    result: &CheckResult,
    elapsed_ms: u64,
    deadline: bool,
    wall_secs: u64,
    resolve: &dyn Fn(&str) -> Option<(String, LocatedClass)>,
    host_cause: &dyn Fn() -> Option<(String, String)>,
) -> RunOutcome {
    if let Some(detail) = &result.operational_error {
        return if deadline {
            RunOutcome::TimedOut { wall_secs }
        } else {
            unrun("operational", detail)
        };
    }
    let Some(exit_code) = result.exit_code else {
        return RunOutcome::NoExitStatus;
    };
    if exit_code == 0 {
        return if result.reports_zero_work() {
            unrun("zero_work", "no work reported")
        } else {
            RunOutcome::Survived { elapsed_ms }
        };
    }
    let located = locations(&verdict::output(result), resolve);
    if !located.is_empty() {
        return RunOutcome::Unviable { located };
    }
    if let Some((detail, code)) = host_cause() {
        return unrun(&code, &detail);
    }
    RunOutcome::Killed {
        exit_code,
        elapsed_ms,
    }
}
pub(super) fn unrun(code: &str, detail: &str) -> RunOutcome {
    RunOutcome::Unrun {
        code: code.into(),
        detail: detail.into(),
    }
}

pub(super) fn refusal(outcome: &RunOutcome) -> String {
    match outcome {
        RunOutcome::Unrun { code, detail } => {
            if detail.starts_with("copy_not_removed:") {
                detail.clone()
            } else {
                format!("{code}: {detail}")
            }
        }
        other => format!("{other:?}"),
    }
}
