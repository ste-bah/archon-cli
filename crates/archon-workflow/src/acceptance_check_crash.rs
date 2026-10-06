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
//! when every condition of one rule below holds, and each rule ties the error
//! to a line of the check's OWN source that is wrong whatever the product
//! does. Anything else — an assertion, `sys.exit(msg)`, any traceback frame
//! in a library or product file, a data-dependent exception (`KeyError`,
//! `AttributeError`, a `TypeError` that is not a call-signature mismatch), a
//! name the product was meant to provide (star import, `exec`), a call on a
//! product object, a missing product binary, a nested or generated shell
//! script, a timeout, an operational error, output after the error line — is
//! an ordinary failure. Misreading a product failure as a script defect would
//! send a genuinely failing check to be re-authored instead of to the tasks
//! that implement it, so "unknown" is always an ordinary failure.
//!
//! Python — exit code 1, the LAST non-empty stderr line is the exception
//! line, EVERY frame of the final traceback is the check's own inline
//! program (`<stdin>` from its single `python - <<TAG` heredoc, or
//! `<string>` from its single `python -c '...'`), and line N of the
//! innermost frame is looked up in that program:
//! * `SyntaxError` / `IndentationError` / `TabError` raised compiling the
//!   program itself (no traceback header, one frame), whose echoed text, if
//!   any, is that line;
//! * `NameError` for a name line N uses as a bare identifier, in a program
//!   that never binds it (no assignment, `def`, `class`, parameter, loop or
//!   `as` target — a name bound on a branch the product's output skipped is
//!   data-dependent), never imports it, and does not fill its namespace
//!   dynamically (`import *`, `exec`, `eval`, `globals()`, ...);
//! * `TypeError` that is a call-signature mismatch (`f() missing N required
//!   ... argument`, `takes N positional arguments but M were given`, `got an
//!   unexpected keyword argument`, `got multiple values for argument`) where
//!   `f` is a function the program defines (`def f(`) and does not import,
//!   line N calls it by its bare name with no `*`/`**` unpacking (an
//!   argument count taken from data may be the product's), and any class in
//!   its qualified name is a class the program defines.
//!
//! POSIX shell — the error line comes from a shell reading a script from
//! stdin (`sh: line N:`, dash's `sh: N:`; never a named script, `-c`,
//! `eval`, or `source`), and either:
//! * exit code 2 with a syntax error among the last two stderr lines, and the
//!   check's own text fails the executing shell's parse-only mode (`sh -n`) —
//!   a syntax error in a generated or nested script does not; or
//! * exit code 127 with `NAME: command not found` as the last line, where
//!   `NAME` is a shell function the check itself defines and line N of the
//!   check calls — a missing product binary or system tool is not the check's
//!   own helper.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::LazyLock;

use regex::Regex;

use crate::acceptance_scratch::CheckResult;

/// How a check's execution ended, as far as its own code is concerned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckRunClass {
    Passed,
    /// Failed, errored, or could not be classified: the check's verdict.
    Failed,
    /// The check crashed in its own code before it could assert anything.
    ScriptDefect(ScriptDefect),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptDefect {
    /// `python` or `shell`.
    pub interpreter: String,
    /// The rule that matched, in words.
    pub rule: String,
    /// The error line the interpreter printed.
    pub signal: String,
}

impl ScriptDefect {
    /// The finding an author is shown: what crashed and the evidence.
    pub fn finding(&self, id: &str, stderr_tail: &str) -> String {
        format!(
            "check '{id}' crashed in its own {} code when the host executed it against the current tree ({}: {}), so it never asserted its criterion; fix exactly that script defect, keep every assertion and every call into the deliverable, and never replace a call to the deliverable with a stand-in. Captured stderr:\n{stderr_tail}",
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
    if let Some(classification) = &result.classification {
        return classification.crash.clone();
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

/// Host-computed verdict from raw streams, preserved across redaction/reuse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckClassification {
    pub passed: bool,
    pub zero_work: bool,
    pub crash: CheckRunClass,
}

impl CheckResult {
    /// Call before changing any captured stream. The command is host-authorized.
    pub fn classify_raw(&mut self, command: &str) {
        if self.classification.is_some() {
            return;
        }
        let zero_work = crate::acceptance::output_reports_zero_work(
            &String::from_utf8_lossy(&self.stdout),
            &String::from_utf8_lossy(&self.stderr),
        );
        let mut crash = classify_check_run(command, self);
        if let CheckRunClass::ScriptDefect(defect) = &mut crash {
            // Raw diagnostic text belongs only in the redacted evidence.
            defect.signal = "see the fenced stderr below".into();
        }
        self.classification = Some(CheckClassification {
            passed: self.operational_error.is_none() && self.exit_code == Some(0) && !zero_work,
            zero_work,
            crash,
        });
    }
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static pattern")
}

const TRACEBACK: &str = "Traceback (most recent call last):";
static FRAME: LazyLock<Regex> = LazyLock::new(|| re(r#"^\s*File "([^"]+)", line (\d+)"#));
static EXCEPTION: LazyLock<Regex> =
    LazyLock::new(|| re(r"^([A-Za-z_][A-Za-z0-9_.]*)(?::\s?(.*))?$"));
static QUOTED: LazyLock<Regex> = LazyLock::new(|| re(r"'([A-Za-z_][A-Za-z0-9_]*)'"));
static SIGNATURE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^([A-Za-z_][A-Za-z0-9_.<>]*)\(\) (?:missing \d+ required (?:positional|keyword-only) arguments?|takes (?:from \d+ to )?\d+ positional arguments? but \d+ (?:were|was) given|got an unexpected keyword argument|got multiple values for argument|takes no arguments)",
    )
});
static HEREDOC: LazyLock<Regex> = LazyLock::new(|| {
    re(r#"(?m)\bpython[0-9.]*\b[^\n]*?<<(-?)[ \t]*['"]?([A-Za-z_][A-Za-z0-9_]*)['"]?"#)
});
static DASH_C: LazyLock<Regex> = LazyLock::new(|| re(r"\bpython[0-9.]*\b[^\n']*?\s-c\s+'([^']*)'"));
/// Constructs that fill a program's namespace from outside its text.
static DYNAMIC: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"import\s+\*|\b(?:exec|eval|compile|globals|locals|vars|__import__|setattr|getattr)\s*\(|__builtins__|\bbuiltins\b",
    )
});
/// A shell reading its script from stdin: no script path, `-c`, `eval`.
const SHELL_PREFIX: &str = r"^(?:/[^\s:]*/)?(?:sh|bash|dash|ksh): (?:line )?(\d+): ";
static SHELL_SYNTAX: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"{SHELL_PREFIX}(?:syntax error|Syntax error)")));
static SHELL_NOT_FOUND: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"{SHELL_PREFIX}([A-Za-z_][A-Za-z0-9_.-]*): (?:command )?not found$"
    ))
});

fn word(name: &str) -> String {
    format!(
        r"(?:^|[^A-Za-z0-9_]){}(?:[^A-Za-z0-9_]|$)",
        regex::escape(name)
    )
}

fn matches(pattern: &str, text: &str) -> bool {
    Regex::new(pattern).is_ok_and(|pattern| pattern.is_match(text))
}

/// `name` used as a bare identifier (not an attribute) in `line`.
fn bare(line: &str, name: &str) -> bool {
    matches(
        &format!(
            r"(?:^|[^A-Za-z0-9_.]){}(?:[^A-Za-z0-9_]|$)",
            regex::escape(name)
        ),
        line,
    )
}

/// Whether `program` binds `name` anywhere: assignment, augmented or
/// walrus assignment, `def`/`class`, `for`/`with`/`except ... as`, a
/// parameter, or `global`/`nonlocal`.
fn binds(program: &str, name: &str) -> bool {
    let n = regex::escape(name);
    [
        format!(r"(?:^|[^A-Za-z0-9_.]){n}\s*(?:[-+*/%&|^@]|//|\*\*|<<|>>)?=[^=]"),
        format!(r"(?:^|[^A-Za-z0-9_.]){n}\s*,[^\n]*[^=!<>]=[^=]"),
        format!(r"(?:^|[^A-Za-z0-9_.]){n}\s*:="),
        format!(r"\b(?:def|class|as|global|nonlocal)\s+{n}\b"),
        format!(r"\bfor\b[^\n]*\b{n}\b[^\n]*\bin\b"),
        format!(r"\bdef\s+\w+\s*\([^)]*\b{n}\b"),
        format!(r"\blambda\b[^:\n]*\b{n}\b[^:\n]*:"),
    ]
    .iter()
    .any(|pattern| matches(pattern, program))
}

fn imports(program: &str, name: &str) -> bool {
    matches(
        &format!(r"(?m)^\s*(?:from\s+\S+\s+)?import\s[^\n]*{}", word(name)),
        program,
    )
}

/// The check's single inline python program behind frame file `file`.
fn inline_program(command: &str, file: &str) -> Option<String> {
    match file {
        "<stdin>" => {
            let mut found = HEREDOC.captures_iter(command);
            let only = found.next()?;
            if found.next().is_some() {
                return None;
            }
            let strip_tabs = !only.get(1)?.as_str().is_empty();
            let tag = only.get(2)?.as_str();
            let body = command[only.get(0)?.end()..].split_once('\n')?.1;
            let mut program = Vec::new();
            for line in body.lines() {
                let terminator = if strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line
                };
                if terminator == tag {
                    return Some(program.join("\n"));
                }
                program.push(line);
            }
            None
        }
        "<string>" => {
            let mut found = DASH_C.captures_iter(command);
            let only = found.next()?;
            if found.next().is_some() {
                return None;
            }
            Some(only.get(1)?.as_str().to_string())
        }
        _ => None,
    }
}

fn python_defect(command: &str, code: i32, lines: &[&str]) -> Option<ScriptDefect> {
    if code != 1 {
        return None;
    }
    let (&last, rest) = lines.split_last()?;
    let exception = EXCEPTION.captures(last.trim_end())?;
    let class = exception.get(1)?.as_str();
    let message = exception.get(2).map_or("", |m| m.as_str());
    // The final traceback block: after the last header (a compile-time
    // syntax error of the program itself has none).
    let header = rest.iter().rposition(|line| line.trim() == TRACEBACK);
    let block = &rest[header.map_or(0, |at| at + 1)..];
    let frames: Vec<(String, usize)> = block
        .iter()
        .filter_map(|line| FRAME.captures(line))
        .filter_map(|c| {
            Some((
                c.get(1)?.as_str().to_string(),
                c.get(2)?.as_str().parse().ok()?,
            ))
        })
        .collect();
    let (file, number) = frames.last()?.clone();
    // Every frame must be the check's own program: a product or library
    // frame anywhere means the product took part in the failure.
    if frames.iter().any(|(other, _)| *other != file) {
        return None;
    }
    let program = inline_program(command, &file)?;
    let source = program.lines().nth(number.checked_sub(1)?);
    let frame_at = block.iter().rposition(|line| FRAME.is_match(line))?;
    let echoed: Vec<&str> = block[frame_at + 1..]
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.chars().all(|ch| matches!(ch, '^' | '~')))
        .collect();
    // Echoed source, if any, must be the line the frame names.
    if let Some(text) = echoed.first()
        && source.is_none_or(|source| !source.contains(text))
    {
        return None;
    }
    let rule = match class {
        "SyntaxError" | "IndentationError" | "TabError" => {
            // Compiling the program itself: no header, a single frame, at a
            // line of the program or just past its end.
            if header.is_some() || frames.len() != 1 || number > program.lines().count() + 1 {
                return None;
            }
            "syntax error in the check's inline python"
        }
        // A name bound anywhere in the program (a branch the product's
        // output skipped, a local read too early) is data-dependent; only a
        // name the program never binds at all is its own defect.
        "NameError" => {
            let name = QUOTED.captures(message)?.get(1)?.as_str();
            let source = source?;
            if !bare(source, name)
                || imports(&program, name)
                || binds(&program, name)
                || DYNAMIC.is_match(&program)
            {
                return None;
            }
            "undefined name in the check's inline python"
        }
        "TypeError" => {
            let qualified = SIGNATURE.captures(message)?.get(1)?.as_str();
            let source = source?;
            // `f(*runtime)` / `f(**runtime)`: the argument count comes from
            // data, which may be the product's output.
            if source.contains('*') || !own_callable_called(&program, source, qualified) {
                return None;
            }
            "call-signature mismatch on a function the check defines"
        }
        _ => return None,
    };
    Some(ScriptDefect {
        interpreter: "python".into(),
        rule: rule.into(),
        signal: last.trim().to_string(),
    })
}

/// `qualified` (`f`, `Class.method`, `outer.<locals>.f`, `Class.__init__`)
/// is a callable `program` defines and does not import, and `source` (the
/// crashing line) calls it by its own name rather than as some object's
/// attribute.
fn own_callable_called(program: &str, source: &str, qualified: &str) -> bool {
    if DYNAMIC.is_match(program) {
        return false;
    }
    let segments: Vec<&str> = qualified.split('.').collect();
    let Some((&function, scopes)) = segments.split_last() else {
        return false;
    };
    let defines = |kind: &str, name: &str| {
        matches(
            &format!(r"(?m)^\s*{kind}\s+{}\b", regex::escape(name)),
            program,
        )
    };
    // Every enclosing class in the qualified name is the program's own.
    let classes: Vec<&str> = scopes
        .iter()
        .copied()
        .filter(|scope| *scope != "<locals>" && !defines("def", scope))
        .collect();
    if classes
        .iter()
        .any(|class| !defines("class", class) || imports(program, class))
    {
        return false;
    }
    if !defines("def", function) || imports(program, function) {
        return false;
    }
    match (function, classes.last()) {
        // A constructor is called by its class's bare name.
        ("__init__", Some(class)) => bare(source, class),
        ("__init__", None) => false,
        // A method of the program's own class may be called as an attribute.
        (_, Some(_)) => source.contains(&format!("{function}(")),
        (_, None) => bare(source, function) && source.contains(&format!("{function}(")),
    }
}

/// Whether the executing shell's parse-only mode rejects `command`.
fn shell_rejects(command: &str) -> bool {
    let child = archon_shell::spawn::command(archon_shell::resolve_posix_shell())
        .args(["-n", "-s"])
        .env_clear()
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    let written = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(command.as_bytes()).is_ok());
    child
        .wait()
        .is_ok_and(|status| written && !status.success())
}

fn shell_defect(command: &str, code: i32, lines: &[&str]) -> Option<ScriptDefect> {
    let (&last, _) = lines.split_last()?;
    match code {
        2 => {
            let start = lines.len().saturating_sub(2);
            let at = (start..lines.len()).find(|&i| SHELL_SYNTAX.is_match(lines[i]))?;
            if !shell_rejects(command) {
                return None;
            }
            Some(ScriptDefect {
                interpreter: "shell".into(),
                rule: "syntax error in the check's shell script".into(),
                signal: lines[at].trim().to_string(),
            })
        }
        127 => {
            let found = SHELL_NOT_FOUND.captures(last.trim_end())?;
            let number: usize = found.get(1)?.as_str().parse().ok()?;
            let name = found.get(2)?.as_str();
            let defined = matches(
                &format!(
                    r"(?m)(?:^|[\s;{{(&|])(?:function\s+{name}\b|{name}\s*\(\s*\))",
                    name = regex::escape(name)
                ),
                command,
            );
            let called = command
                .lines()
                .nth(number.checked_sub(1)?)
                .is_some_and(|line| matches(&word(name), line));
            (defined && called).then(|| ScriptDefect {
                interpreter: "shell".into(),
                rule: "call of a shell helper the check defines but never made available".into(),
                signal: last.trim().to_string(),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "acceptance_check_crash_tests.rs"]
mod tests;
