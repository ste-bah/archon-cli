//! Did a check that failed on a tree give a VERDICT there (Issue 328)?
//!
//! A can-fail proof (A4) is a check failing on the tree before any
//! implementation. That failure proves something only when the check itself
//! decided it: its assertion found the criterion false, or the deliverable
//! it drives is absent there. A run that stopped before the check could
//! decide anything -- a tool it runs is missing, the tree it builds did not
//! compile, its own script crashed, it never exited -- gave NO verdict: it
//! would fail the same way whatever the check asserts. That run is unproven
//! (the host's), never a proof; one that stays unproven on the same base is
//! sent to its author (`workflow_acceptance_executability_silent`), so no
//! check is blocked for good.
//!
//! Test harnesses draw the same line: pytest exits 1 when tests failed and
//! 2-5 when they could not run; POSIX `grep`, `diff` and `cmp` exit 1 for a
//! negative answer and 2 for trouble; a POSIX shell exits 126 or 127 when it
//! cannot start a program. `cargo test` (101) and jest (1) give one status
//! for a failed test and for a compile error; only the output separates
//! them. A check is any script, so the host decides from the check's own
//! text (`workflow_acceptance_executability_verdict_shell`) and its site's
//! search path, never from what its output says alone:
//!
//! 1. exit 126 or 127, and a program the check starts by NAME is not on
//!    its search path (or by an absolute path that does not exist): a tool
//!    or interpreter its environment lacks. A program it starts by a path
//!    in the tree (`./bin/tool`), or an interpreter given a script of the
//!    tree (`bash scripts/new.sh`), is the deliverable not there yet: a
//!    verdict;
//!    A subcommand the check runs (`tool sub`), which the tool rejected
//!    before any assertion ran, and which no program `tool-sub` on the
//!    search path provides, is the same when the host has that program,
//!    the tool lists its commands (with `sub` or without: the listing
//!    never sees the check's tree), or the host could not list them
//!    (Issues 331, 333,
//!    `workflow_acceptance_executability_verdict_subcommand`);
//! 2. no exit status: the run was killed before it reported anything;
//! 3. the check crashed in its own code
//!    ([`archon_workflow::acceptance_check_crash`]);
//! 4. the check starts a compiler or build tool ([`BUILDERS`]), its output
//!    carries a compiler error at a source location (`path:line[:col]:
//!    error`, `path(line,col): error`, an `error...:` header whose span is
//!    `--> path:line`, or a `SyntaxError` in a source file), every such
//!    location is outside the check's own files and the contract's declared
//!    deliverable paths, nothing shows an assertion ran (`N failed`,
//!    `FAILED`, a panic, a failed assertion), and the build is not itself
//!    the check's assertion (its last program is not a build or lint step).
//!    A compile error in the check's own sources (a test written before the
//!    code it calls) or in a deliverable is a verdict.
//!
//! Anything else is the check's verdict on that tree, including a run that
//! did no work (no test matched) and a failure naming no source location.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use archon_workflow::acceptance_check_crash::{CheckRunClass, classify_check_run};
use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::{AcceptanceCheck, AcceptanceContract};
use regex::Regex;

use super::verdict_shell::{Simple, expands, simple_commands};

#[path = "workflow_acceptance_executability_verdict_subcommand.rs"]
mod subcommand;
#[path = "workflow_acceptance_executability_verdict_which.rs"]
mod which;
#[cfg(test)]
pub(crate) use subcommand::HOST_PATH;
pub(crate) use subcommand::unresolved_on_path;

/// Programs whose job is to compile or build: a compiler error they print
/// is the tree's (rule 4). An interpreter compiles the source it imports.
const BUILDERS: &[&str] = &[
    "cargo", "rustc", "go", "gcc", "g++", "cc", "c++", "clang", "clang++", "javac", "kotlinc",
    "scalac", "tsc", "swiftc", "swift", "make", "cmake", "ninja", "gradle", "gradlew", "mvn",
    "dotnet", "npm", "npx", "yarn", "pnpm", "bazel", "python", "python3",
];

/// Arguments that make a build tool run the product, not just build it.
const RUNS_PRODUCT: &[&str] = &["test", "nextest", "run", "bench", "exec", "pytest", "jest"];

/// Where a check runs: its site's environment (its search path among it),
/// the contract's declared deliverable paths, and the host's own search
/// path, where a program the check's path lacks may be installed.
#[derive(Clone)]
pub(crate) struct Context {
    /// The site's search path and `PATHEXT`, as a child started with its
    /// variables sees them (`verdict_which::variable`).
    path: Option<String>,
    pathext: Option<String>,
    deliverables: Vec<String>,
    /// The variables a listing of a tool runs with (Issue 333), and the
    /// search path the check runs on among them. Only the site's own
    /// context gives them (`silent::context`, [`Context::for_scratch`]):
    /// nothing here reads the host's environment, so a listing never sees
    /// the operator's (Issue 282).
    environment: BTreeMap<String, String>,
    host_path: Option<String>,
    /// How long a listing may print nothing before it is given up.
    list_stall: std::time::Duration,
}

impl Context {
    /// The site environment `environment`, from the site's own context
    /// (never the host's: see the field), and every path a floor of
    /// `contract` declares.
    pub(crate) fn new(
        environment: BTreeMap<String, String>,
        contract: &AcceptanceContract,
    ) -> Self {
        let deliverables = (contract.acceptance.iter())
            .chain(&contract.supplementary)
            .filter_map(|entry| match &entry.check {
                AcceptanceCheck::Floor { contract } => Some(contract),
                AcceptanceCheck::Command { .. } => None,
            })
            .flat_map(|floor| {
                [Some(&floor.artifact_path), floor.registry_path.as_ref()]
                    .into_iter()
                    .chain([floor.instance_source_path.as_ref()])
                    .flatten()
                    .cloned()
            })
            .filter(|path| !path.trim().is_empty())
            .collect();
        Self::at(environment, deliverables)
    }

    /// A site whose only variable is the host's own search path.
    #[cfg(test)]
    pub(crate) fn on_host_path(contract: &AcceptanceContract) -> Self {
        let path = subcommand::host_path().map(|path| ("PATH".to_string(), path));
        Self::new(path.into_iter().collect(), contract)
    }

    /// A scratch site of `policy`, as it gives every check the variables
    /// its policy binds and forwards (no declared deliverables).
    pub(crate) fn for_scratch(policy: &archon_workflow::acceptance_scratch::ScratchPolicy) -> Self {
        Self::at(super::sites::scratch_environment(policy), Vec::new())
    }

    fn at(environment: BTreeMap<String, String>, deliverables: Vec<String>) -> Self {
        let windows = cfg!(windows);
        Self {
            path: which::variable(&environment, "PATH", windows).map(str::to_string),
            pathext: which::variable(&environment, "PATHEXT", windows).map(str::to_string),
            deliverables,
            environment,
            host_path: subcommand::host_path(),
            list_stall: subcommand::LIST_STALL,
        }
    }

    /// Whether `name` is a program on the search path.
    fn on_path(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    /// Where the program `name` is on the search path, as this platform's
    /// launcher finds it with the site's own variables (`verdict_which`).
    fn find(&self, name: &str) -> Option<std::path::PathBuf> {
        let path = self.path.as_deref()?;
        which::find_on(path, self.pathext.as_deref(), name, cfg!(windows))
    }

    /// `name` without the executable extension the site's `PATHEXT` adds.
    fn bare_name(&self, name: &str) -> String {
        which::bare_name(name, self.pathext.as_deref())
    }
}

/// Why the failed run `result` of the check text `command` gave no verdict
/// (see the module docs); `None` when it passed, could not run at all (an
/// operational error, reported as such), or its failure is a verdict.
pub(crate) fn no_verdict(command: &str, result: &CheckResult, at: &Context) -> Option<String> {
    if result.operational_error.is_some() || super::baseline::passed(result) {
        return None;
    }
    let Some(code) = result.exit_code else {
        return Some(
            "it ended without an exit status (killed by a signal) before it reported anything"
                .to_string(),
        );
    };
    let commands = simple_commands(command);
    if matches!(code, 126 | 127)
        && let Some(name) = missing_program(&commands, &output(result), at)
    {
        return Some(format!(
            "it starts `{name}`, which is not on its search path (exit {code}): a tool or interpreter its environment lacks"
        ));
    }
    if code != 0
        && let Some(why) = subcommand::missing(&commands, &output(result), at)
    {
        return Some(why);
    }
    if let CheckRunClass::ScriptDefect(defect) = classify_check_run(command, result) {
        return Some(format!(
            "it crashed in its own {} code ({}) before it asserted anything",
            defect.interpreter, defect.rule
        ));
    }
    if matches!(code, 0 | 126 | 127) {
        return None;
    }
    let locations = compile_errors(&output(result));
    let builds = commands
        .iter()
        .any(|c| c.program.as_deref().is_some_and(builder));
    let foreign =
        !locations.is_empty() && (locations.iter()).all(|location| !owned(location, &commands, at));
    (builds && foreign && !build_is_assertion(&commands)).then(|| {
        "the tree it ran on did not build: it stopped at a compiler error outside its own files and the deliverables, before any assertion ran".to_string()
    })
}

/// Whether `result` may have failed for its host (rules 1, 2, 4 and a
/// rejected subcommand, without the check's text): such a run is never remembered as a verdict, so a retry
/// runs it again.
pub(crate) fn may_be_host_failure(result: &CheckResult) -> bool {
    if result.operational_error.is_some() || super::baseline::passed(result) {
        return false;
    }
    match result.exit_code {
        None | Some(126 | 127) => true,
        Some(0) => false,
        Some(_) => {
            let output = output(result);
            !compile_errors(&output).is_empty() || subcommand::rejected(&output)
        }
    }
}

fn output(result: &CheckResult) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    )
}

/// The first program `commands` start that cannot start: a bare name not on
/// the search path, or an absolute path that does not exist. A program
/// that printed under its own name (`bash: scripts/x.sh: No such file or
/// directory`) ran, whatever the search path says: exit 127 is then its
/// own answer, such as a script it was given that is not there.
fn missing_program(commands: &[Simple], output: &str, at: &Context) -> Option<String> {
    (commands.iter())
        .filter_map(|c| c.program.as_deref())
        .filter(|program| !printed_by(output, program, at))
        .find(|program| match which::is_path(program) {
            true => which::is_absolute(program) && !Path::new(program).exists(),
            false => !at.on_path(program),
        })
        .map(str::to_string)
}

/// Whether a line of `output` is the program `program`'s own message: it
/// starts with the program's name (by any path to it, with or without its
/// executable extension), then `: `.
fn printed_by(output: &str, program: &str, at: &Context) -> bool {
    let name = |text: &str| {
        let file = text.rsplit(['/', '\\']).next().unwrap_or(text);
        at.bare_name(file)
    };
    let wanted = name(program);
    (output.lines()).any(|line| {
        let Some((speaker, said)) = line.trim_start().split_once(": ") else {
            return false;
        };
        // `foo: command not found` (as tcsh says it) names what did not run.
        let lacking = said.trim_start().to_ascii_lowercase();
        let speaker = name(speaker.trim());
        !lacking.starts_with("command not found")
            && !lacking.starts_with("not found")
            && !speaker.is_empty()
            && !speaker.contains(char::is_whitespace)
            && match cfg!(windows) {
                true => speaker.eq_ignore_ascii_case(&wanted),
                false => speaker == wanted,
            }
    })
}

/// The program's own name: a build tool whatever directory it is run from.
fn builder(program: &str) -> bool {
    let name = program.rsplit('/').next().unwrap_or(program);
    BUILDERS.contains(&name) || (name.starts_with("python3.") && name.len() > 8)
}

/// Rule 4's exception: the check's last program is a build or lint step,
/// so the build IS its assertion.
fn build_is_assertion(commands: &[Simple]) -> bool {
    // The check's status is its last command's: a builtin (`test`) there
    // asserts something other than the build.
    let Some(last) = commands.last() else {
        return false;
    };
    let Some(program) = last.program.as_deref() else {
        return false;
    };
    let name = program.rsplit('/').next().unwrap_or(program);
    builder(program)
        && !name.starts_with("python")
        && !(last.args.iter()).any(|arg| RUNS_PRODUCT.contains(&arg.as_str()))
}

/// Whether the compile error at `location` is in the check's own files: a
/// path its text names, a deliverable the contract declares, or a file whose
/// stem is a word of the check (a test target it selects by name).
fn owned(location: &str, commands: &[Simple], at: &Context) -> bool {
    let location = location.trim_start_matches("./");
    let words = (commands.iter())
        .flat_map(|c| c.program.iter().chain(&c.args))
        .filter(|word| !word.starts_with('-') && !expands(word));
    let mut identifiers = Vec::new();
    for word in words {
        let path = word.trim_start_matches("./");
        if !path.is_empty() && (location == path || location.ends_with(&format!("/{path}"))) {
            return true;
        }
        identifiers.extend(
            word.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .filter(|part| part.len() >= 2)
                .map(str::to_string),
        );
    }
    let declared = (at.deliverables.iter()).any(|path| {
        let path = path.trim_start_matches("./");
        location == path || location.ends_with(&format!("/{path}"))
    });
    let file = location.rsplit('/').next().unwrap_or(location);
    let stem = file.split('.').next().unwrap_or(file);
    declared || identifiers.iter().any(|word| word == stem)
}

/// Evidence that an assertion ran: a runner's count of failed tests, a
/// failed-test marker, a panic, or a failed assertion.
static ASSERTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b[1-9][0-9]* (?:failed|failing)\b|\bFAILED\b|panicked at|\bAssertionError\b|[Aa]ssertion\b[^\n]*\bfailed",
    )
    .expect("static pattern")
});

/// `path:line[:col]: [fatal ]error...` and `path(line,col): error...`.
static LOCATED_ERROR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\S+?)(?::\d+(?::\d+)?:|\(\d+,\d+\):?)\s+(?:fatal\s+)?error\b")
        .expect("static pattern")
});

/// An error diagnostic's header: `error: ...`, `error[E0425]: ...`.
static ERROR_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:fatal\s+)?error(?:\[[A-Za-z0-9]+\])?:\s").expect("static pattern")
});

/// A diagnostic's primary span: `--> path:line[:col]`.
static SPAN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^-->\s+(\S+?):\d+").expect("static pattern"));

/// A source file's interpreter frame: `File "path", line N`, not a program
/// read from stdin or `-c`.
static SOURCE_FRAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*File "([^"<][^"]*)", line \d+"#).expect("static pattern"));

const SYNTAX_ERRORS: [&str; 3] = ["SyntaxError", "IndentationError", "TabError"];

/// The source files `output` names as compiler error locations, none when
/// an assertion ran (see the module docs).
fn compile_errors(output: &str) -> Vec<String> {
    if ASSERTED.is_match(output) {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut header = false;
    for line in output.lines() {
        if line.trim().is_empty() {
            header = false;
            continue;
        }
        if let Some(located) = LOCATED_ERROR.captures(line) {
            found.push(located[1].to_string());
        } else if header && let Some(span) = SPAN.captures(line.trim_start()) {
            found.push(span[1].to_string());
        }
        // Any other unindented line (a warning's header, plain output) ends
        // the error's block: a span after it is not the error's.
        if !line.starts_with(char::is_whitespace) {
            header = ERROR_HEADER.is_match(line);
        }
    }
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    let syntax = lines.last().is_some_and(|last| {
        let last = last.trim();
        (SYNTAX_ERRORS.iter()).any(|class| {
            last.strip_prefix(class)
                .is_some_and(|rest| rest.starts_with(':'))
        })
    });
    if syntax && let Some(frame) = lines.iter().rev().find_map(|l| SOURCE_FRAME.captures(l)) {
        found.push(frame[1].to_string());
    }
    found
}

#[cfg(test)]
#[path = "workflow_acceptance_executability_verdict_tests.rs"]
mod tests;
