//! A failing check that needed a host variable its site withheld (Issue 345).
//!
//! Before Issue 345 a site with no policy gave a check every host variable,
//! so a check may read one it no longer gets. Its failure is then no verdict
//! on the product, and only the operator can repair it -- but only when the
//! remedy works: the variable must be one `environment_allowlist` can forward
//! (Issue 282's data rule accepts it), and the check's output must say it is
//! missing or unset. A bare mention is no evidence: every panicking Rust test
//! prints "run with `RUST_BACKTRACE=1`", and a variable the allowlist refuses
//! would give the same error every round, so the run could never recover.

use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;

use super::{SHELL_OWN, lookup};

/// The host variables a check given `environment` does not get.
pub fn withheld(
    host: &BTreeMap<String, String>,
    environment: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    (host.keys())
        .filter(|name| !SHELL_OWN.contains(&name.as_str()) && lookup(environment, name).is_none())
        .cloned()
        .collect()
}

/// Whether `output` says the variable `name` is missing or unset:
/// - `NAME is not set` (and unset, not defined, undefined, missing,
///   required, must be set, the shells' `unbound variable` and `parameter
///   not set`);
/// - `NAME environment variable` (`X environment variable is not set`),
///   `environment variable NAME`, `missing NAME`;
/// - `set` / `setting` / `export` / `provide` `(the) NAME` (django-environ's
///   "Set the X environment variable", the OpenAI SDK's "by setting the X
///   environment variable");
/// - `NAME` then, within 80 characters on its line, not found, is empty, not
///   provided or required (python-decouple's "X not found. Declare it as
///   envvar", zod/t3-env's `{ X: [ 'Required' ] }`);
/// - `NAME` on one line and `Field required` or `Required` on the next
///   (pydantic);
/// - Python's `KeyError: 'NAME'`.
///
/// The name's case is ignored only on Windows, as [`withheld`] ignores it
/// there. A message that does not name the variable -- Rust's
/// `env::var(..)` error `NotPresent`, Go's empty `os.Getenv` -- cannot be
/// told from a product failure, so it stays a verdict. Nothing later
/// catches it either: the freeze's passability judge (Issue 275) is shown
/// that baseline output, but it refutes only a check whose own setup breaks
/// a rule the product enforces, and accepts ambiguous output. Such a check
/// needs its variable named in `environment_allowlist`.
fn says_missing(name: &str, output: &str) -> bool {
    let escaped = regex::escape(name);
    let name = if cfg!(windows) {
        escaped
    } else {
        format!("(?-i:{escaped})")
    };
    let patterns = [
        format!(
            r#"\b{name}\b['"`]?\s*:?\s*(?:is\s+)?(?:not\s+set|unset|not\s+defined|undefined|missing|required|must\s+be\s+set|unbound\s+variable|parameter\s+(?:null\s+or\s+)?not\s+set)"#
        ),
        format!(r#"(?:environment\s+variable|env\s+var|missing)\s*:?\s*['"`$]?\b{name}\b"#),
        format!(r#"\b{name}\b['"`]?\s+env(?:ironment)?\s+var(?:iable)?"#),
        format!(
            r#"\b(?:set(?:ting)?|export(?:ing)?|provid(?:e|ing))\s+(?:the\s+)?['"`$]?\b{name}\b"#
        ),
        format!(r#"\b{name}\b[^\n]{{0,80}}(?:not\s+found|is\s+empty|not\s+provided|required)"#),
        format!(r#"\b{name}\b[^\n]*\n[^\n]*\b(?:field\s+)?required\b"#),
        format!(r#"KeyError:\s*['"]{name}['"]"#),
    ];
    patterns.iter().any(|pattern| {
        Regex::new(&format!("(?i){pattern}")).is_ok_and(|regex| regex.is_match(output))
    })
}

/// The operational error of a check that failed with `outputs` (its stdout
/// and stderr) saying a withheld variable is missing, when the allowlist can
/// forward that variable; `None` otherwise, and the failure stays a verdict.
pub fn withheld_error(outputs: &[&[u8]], withheld: &BTreeSet<String>) -> Option<String> {
    let outputs: Vec<String> = (outputs.iter())
        .map(|output| String::from_utf8_lossy(output).into_owned())
        .collect();
    let named: Vec<&str> = (withheld.iter())
        .filter(|name| archon_shell::data_environment::check_data_variable(name).is_ok())
        .filter(|name| outputs.iter().any(|output| says_missing(name, output)))
        .map(String::as_str)
        .collect();
    if named.is_empty() {
        return None;
    }
    let list = named.join(", ");
    Some(format!(
        "the check failed saying the host variable(s) {list} are missing, and this acceptance site does not give a check them (Issue 345: a check gets PATH, the locale, the toolchain locators and only the host variables its policy forwards), so the failure is no verdict. If the check needs them, name them in [workflow.acceptance_execution] environment_allowlist; otherwise the check must not read them"
    ))
}
