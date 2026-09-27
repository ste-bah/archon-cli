//! Did a failing acceptance check crash in its OWN code?
//!
//! A frozen check is a script the host runs (`sh -s`, the check text on
//! stdin). When it exits non-zero there are two very different reasons:
//!
//! * the check ran and its assertion said the criterion is false — the
//!   product may genuinely be failing, and remediation belongs to the tasks
//!   that implement it;
//! * the check never got to assert anything, because the check's own script
//!   is broken — no product change can make it pass, and only its author can
//!   repair it.
//!
//! This module draws that line from the executor's own observation (exit
//! code and captured stderr) plus the check's text, for any criterion and any
//! PRD. It is deliberately conservative: a failure is a [`ScriptDefect`] only
//! when every condition of one rule below holds; anything else — an
//! assertion, `sys.exit(msg)`, a crash inside a library or product file, a
//! data-dependent exception (`KeyError`, `AttributeError`, a `TypeError` that
//! is not a call-signature mismatch), a missing product binary, a timeout, an
//! operational error, output after the error line — is an ordinary failure.
//! Misreading a product failure as a script defect would send a genuinely
//! failing check to be re-authored instead of to the implementing tasks, so
//! "unknown" is always an ordinary failure.
//!
//! Rules (all conditions of one rule must hold):
//!
//! Python — exit code 1, the check text invokes a python interpreter, the
//! LAST non-empty stderr line is the exception line, and the innermost
//! `File "..."` frame of the final traceback is the check's own inline source
//! (`<stdin>` for `python3 - <<EOF`, `<string>` for `python3 -c`):
//! * `SyntaxError` / `IndentationError` / `TabError`, whose displayed source
//!   line is text of the check (a blank display is accepted only for
//!   `<stdin>`, where Python shows none at end of input);
//! * `NameError` / `UnboundLocalError` naming an identifier the check text
//!   contains;
//! * `TypeError` that is a call-signature mismatch (`f() missing N required
//!   ... argument`, `takes N positional arguments but M were given`, `got an
//!   unexpected keyword argument`, `got multiple values for argument`) on a
//!   function the check itself defines (`def f(`).
//!
//! POSIX shell — the error line comes from the top-level shell reading the
//! check (`sh: line N:`, `bash: -c: line N:`, dash's `sh: N:`; never a named
//! script, `eval`, or `source`), and either:
//! * exit code 2 with a syntax error among the last two stderr lines, whose
//!   echoed source text (when the shell echoes it) is text of the check; or
//! * exit code 127 with `NAME: command not found` as the last line, where
//!   `NAME` is a shell function the check itself defines — a missing product
//!   binary or system tool is NOT the check's own helper.

use std::sync::LazyLock;

use regex::Regex;

use crate::acceptance_scratch::CheckResult;

/// How a check's execution ended, as far as its own code is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckRunClass {
    Passed,
    /// Failed, errored, or could not be classified: the check's verdict.
    Failed,
    /// The check crashed in its own code before it could assert anything.
    ScriptDefect(ScriptDefect),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptDefect {
    /// `python` or `shell`.
    pub interpreter: &'static str,
    /// The rule that matched, in words.
    pub rule: &'static str,
    /// The error line the interpreter printed.
    pub signal: String,
}

impl ScriptDefect {
    /// The finding an author is shown: what crashed and the evidence.
    pub fn finding(&self, id: &str, stderr_tail: &str) -> String {
        format!(
            "check '{id}' crashed in its own {} code when the host executed it against the current tree ({}: {}), so it never asserted its criterion; fix the check's own script defect and keep every assertion it makes. Captured stderr:\n{stderr_tail}",
            self.interpreter, self.rule, self.signal
        )
    }
}

/// Classify one executed check. `command` is the exact check text the host
/// ran (a command check's command or a floor's verifier command).
pub fn classify_check_run(command: &str, result: &CheckResult) -> CheckRunClass {
    if result.operational_error.is_some() {
        return CheckRunClass::Failed;
    }
    match result.exit_code {
        Some(0) => return CheckRunClass::Passed,
        None => return CheckRunClass::Failed,
        Some(_) => {}
    }
    let stderr = String::from_utf8_lossy(&result.stderr);
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    let code = result.exit_code.unwrap_or_default();
    python_defect(command, code, &lines)
        .or_else(|| shell_defect(command, code, &lines))
        .map_or(CheckRunClass::Failed, CheckRunClass::ScriptDefect)
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static pattern")
}

static PYTHON_INVOCATION: LazyLock<Regex> = LazyLock::new(|| re(r"\bpython[0-9.]*\b"));
static FRAME: LazyLock<Regex> = LazyLock::new(|| re(r#"^\s*File "([^"]+)", line (\d+)"#));
static EXCEPTION: LazyLock<Regex> =
    LazyLock::new(|| re(r"^([A-Za-z_][A-Za-z0-9_.]*)(?::\s?(.*))?$"));
static QUOTED: LazyLock<Regex> = LazyLock::new(|| re(r"'([A-Za-z_][A-Za-z0-9_]*)'"));
static SIGNATURE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^([A-Za-z_][A-Za-z0-9_.<>]*)\(\) (?:missing \d+ required (?:positional|keyword-only) arguments?|takes (?:from \d+ to )?\d+ positional arguments? but \d+ (?:were|was) given|got an unexpected keyword argument|got multiple values for argument|takes no arguments)",
    )
});
/// The top-level shell's own prefix: no script path, no `eval`/`source`.
const SHELL_PREFIX: &str = r"^(?:/[^\s:]*/)?(?:sh|bash|dash|ksh)(?:: -c)?: (?:line )?\d+: ";
static SHELL_SYNTAX: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"{SHELL_PREFIX}(?:syntax error|Syntax error)")));
static SHELL_ECHO: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"{SHELL_PREFIX}`(.*)'$")));
static SHELL_NOT_FOUND: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"{SHELL_PREFIX}([A-Za-z_][A-Za-z0-9_.-]*): (?:command )?not found$"
    ))
});

fn contains_word(text: &str, word: &str) -> bool {
    Regex::new(&format!(
        r"(?:^|[^A-Za-z0-9_]){}(?:[^A-Za-z0-9_]|$)",
        regex::escape(word)
    ))
    .is_ok_and(|pattern| pattern.is_match(text))
}

fn python_defect(command: &str, code: i32, lines: &[&str]) -> Option<ScriptDefect> {
    if code != 1 || !PYTHON_INVOCATION.is_match(command) {
        return None;
    }
    let (&last, rest) = lines.split_last()?;
    let exception = EXCEPTION.captures(last.trim_end())?;
    let class = exception.get(1)?.as_str();
    let message = exception.get(2).map_or("", |m| m.as_str());
    // The innermost frame of the final traceback: the last `File` line.
    let frame_at = rest.iter().rposition(|line| FRAME.is_match(line))?;
    let file = FRAME.captures(rest[frame_at])?.get(1)?.as_str().to_string();
    if file != "<stdin>" && file != "<string>" {
        return None;
    }
    // Between the innermost frame and the exception line Python may echo
    // the offending source and mark it with carets; any echoed text must be
    // the check's own, or the frame is not the check's code.
    let after_frame = &rest[frame_at + 1..];
    let caret = |line: &str| line.trim().chars().all(|ch| matches!(ch, '^' | '~'));
    if !after_frame
        .iter()
        .all(|line| caret(line) || command.contains(line.trim()))
    {
        return None;
    }
    let echoed = after_frame.iter().any(|line| !caret(line));
    let rule = match class {
        // With no echo, only a heredoc's end of input is the check's own.
        "SyntaxError" | "IndentationError" | "TabError" if echoed || file == "<stdin>" => {
            "syntax error in the check's inline python"
        }
        "NameError" | "UnboundLocalError"
            if QUOTED
                .captures(message)
                .and_then(|c| c.get(1))
                .is_some_and(|name| contains_word(command, name.as_str())) =>
        {
            "undefined name in the check's inline python"
        }
        "TypeError"
            if SIGNATURE
                .captures(message)
                .and_then(|c| c.get(1))
                .is_some_and(|qualified| defines_callable(command, qualified.as_str())) =>
        {
            "call-signature mismatch on a function the check defines"
        }
        _ => return None,
    };
    Some(ScriptDefect {
        interpreter: "python",
        rule,
        signal: last.trim().to_string(),
    })
}

/// `qualified` (`f`, `Class.method`, `outer.<locals>.f`, `Class.__init__`)
/// is a callable the check text itself defines.
fn defines_callable(command: &str, qualified: &str) -> bool {
    let mut parts = qualified.rsplit('.');
    let Some(function) = parts.next() else {
        return false;
    };
    contains_def(command, function)
        && (function != "__init__"
            || parts
                .next()
                .is_some_and(|class| contains_word(command, &format!("class {class}"))))
}

fn contains_def(command: &str, function: &str) -> bool {
    Regex::new(&format!(r"\bdef\s+{}\s*\(", regex::escape(function)))
        .is_ok_and(|pattern| pattern.is_match(command))
}

fn shell_defect(command: &str, code: i32, lines: &[&str]) -> Option<ScriptDefect> {
    let (&last, _) = lines.split_last()?;
    match code {
        2 => {
            let start = lines.len().saturating_sub(2);
            let at = (start..lines.len()).find(|&i| SHELL_SYNTAX.is_match(lines[i]))?;
            let echoed = lines[at + 1..]
                .iter()
                .find_map(|line| SHELL_ECHO.captures(line))
                .and_then(|captures| captures.get(1))
                .map(|text| text.as_str().trim().to_string());
            if echoed.is_some_and(|text| !text.is_empty() && !command.contains(&text)) {
                return None;
            }
            Some(ScriptDefect {
                interpreter: "shell",
                rule: "syntax error in the check's shell script",
                signal: lines[at].trim().to_string(),
            })
        }
        127 => {
            let name = SHELL_NOT_FOUND.captures(last.trim_end())?.get(1)?.as_str();
            let function = Regex::new(&format!(
                r"(?m)(?:^|[\s;{{(&|])(?:function\s+{name}\b|{name}\s*\(\s*\))",
                name = regex::escape(name)
            ))
            .ok()?;
            if function.is_match(command) {
                Some(ScriptDefect {
                    interpreter: "shell",
                    rule: "call of a shell helper the check defines but never made available",
                    signal: last.trim().to_string(),
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "acceptance_check_crash_tests.rs"]
mod tests;
