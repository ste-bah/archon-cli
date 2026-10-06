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

/// A text diagnostic is evidence only outside quoted expectations, or inside
/// a recognised actual-error envelope (Rust Err(String), JSON error/message).
/// Scan delimiters with escape parity: a backslash-escaped quote never closes
/// its enclosing phrase. Unknown quoted prose stays ambiguous and is noted,
/// never used to change the verifier's result. This is a grammar, not an
/// inference that any mention of a host variable explains a failing test.
fn unquoted_diagnostics(output: &str) -> String {
    diagnostic_text(output, false)
}

fn diagnostic_text(output: &str, in_error: bool) -> String {
    let mut text = String::with_capacity(output.len());
    let mut at = 0;
    while at < output.len() {
        let ch = output[at..].chars().next().expect("character boundary");
        let contraction = ch == '\''
            && output[..at]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric)
            && output[at + 1..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric);
        if !matches!(ch, '\'' | '"' | '`') || contraction {
            text.push(ch);
            at += ch.len_utf8();
            continue;
        }
        let mut end = at + 1;
        let mut escaped = false;
        while end < output.len() {
            let next = output[end..].chars().next().expect("character boundary");
            if next == ch && !escaped {
                break;
            }
            escaped = next == '\\' && !escaped;
            end += next.len_utf8();
        }
        let closed = end < output.len();
        let inside = &output[at + 1..end];
        let prefix = output[..at].rsplit('\n').next().unwrap_or("");
        let expecting = Regex::new(r"(?i)\b(?:expected|expecting|assert(?:ion)?)\b")
            .expect("literal regex")
            .is_match(prefix);
        let actual = in_error
            || prefix
                .replace('`', "")
                .trim_end()
                .ends_with("on an Err value:")
            || Regex::new(r#"(?i)(?:\berror:|["'](?:error|message|detail)["']\s*:)[ \t]*$"#)
                .expect("literal regex")
                .is_match(prefix);
        let label = inside
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !expecting && (actual || label) {
            if actual && ch == '"' {
                if closed && let Ok(decoded) = serde_json::from_str::<String>(&output[at..end + 1])
                {
                    text.push_str(&diagnostic_text(&decoded, true));
                } else {
                    // Invalid/truncated quoting supplies no absence evidence.
                    // The caller retains the verifier result with a note.
                    text.push_str(&" ".repeat(end + usize::from(closed) - at));
                }
            } else {
                text.push(ch);
                text.push_str(inside);
                if closed {
                    text.push(ch);
                }
            }
        } else {
            text.push_str(&" ".repeat(end + usize::from(closed) - at));
        }
        at = end + usize::from(closed);
    }
    // Remove only expectation lines, retaining any later actual diagnostic.
    // Do this within decoded error envelopes too, before JSON prefixes can
    // hide the line's expectation marker.
    text.lines()
        .map(|line| {
            if line
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("expected ")
            {
                " ".repeat(line.len())
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A withheld variable needs an absence claim, not a mention. Names may be
/// quoted alone. Actual-error envelopes are decoded; quoted expectations
/// (including multiline ones) cannot supply absence evidence.
fn says_missing(name: &str, output: &str) -> bool {
    let output = unquoted_diagnostics(output);
    // An expectation line, including a malformed quoted expectation, is not
    // an assertion that the child lacked data. Keep it ambiguous.
    let output = output
        .lines()
        .filter(|line| {
            !line
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("expected ")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let escaped = regex::escape(name);
    let name = if cfg!(windows) {
        escaped
    } else {
        format!("(?-i:{escaped})")
    };
    let named = format!(r#"(?:(?:^|[^'"`\w]){name}\b['"`]?|['"`]{name}['"`])"#);
    let absent = r"(?:is\s+)?(?:not\s+set|unset|not\s+defined|undefined|missing|required|must\s+be\s+set|unbound\s+variable|parameter\s+(?:null\s+or\s+)?not\s+set|not\s+found|empty|not\s+provided)\b";
    let env = r"env(?:ironment)?\s+var(?:iable)?";
    let patterns = [
        format!(r#"{named}\s*:?\s*{absent}"#),
        format!(r#"(?:{env})\s*:?\s*['"`$]?\b{name}\b['"`]?\s*:?\s*{absent}"#),
        format!(r#"{named}\s+{env}\s*:?\s*{absent}"#),
        format!(r#"\bmissing\s+(?:{env}\s+)?['"`$]?\b{name}\b"#),
        // An imperative request is a diagnostic; an affirmative description
        // such as "we set NAME" is not. SDKs also explain a must-be-set error
        // with "by setting the NAME environment variable".
        format!(
            r#"(?:\bplease\s+(?:set|export|provide)\s+(?:the\s+)?['"`$]?\b{name}\b['"`]?(?:\s+environment\s+variable)?(?:\s+and\s+retry)?[.!]?[ \t]*$|\bImproperlyConfigured:\s*set\s+(?:the\s+)?['"`$]?\b{name}\b['"`]?\s+environment\s+variable[.!]?[ \t]*$)"#
        ),
        format!(r#"\bmust\s+be\s+set[^\n]*\bby\s+setting\s+(?:the\s+)?['"`$]?\b{name}\b"#),
        format!(r#"{named}['"`:\[\]{{}} \t]*(?:not\s+found|is\s+empty|not\s+provided|required)\b"#),
        format!(
            r#"^[ \t]*{name}[ \t]*\r?\n[ \t]*field[ \t]+required(?:[ \t]*\r?$|[ \t]+\[type=missing)"#
        ),
        format!(r#"KeyError:\s*['"]{name}['"]"#),
    ];
    patterns.iter().any(|pattern| {
        Regex::new(&format!("(?im){pattern}")).is_ok_and(|regex| regex.is_match(&output))
    })
}

/// The operational error of a check that failed with `outputs` (its stdout
/// and stderr) saying a withheld variable is missing, when the allowlist can
/// forward that variable; `None` otherwise, and the failure stays a verdict.
pub fn withheld_error(outputs: &[&[u8]], withheld: &BTreeSet<String>) -> Option<String> {
    withheld_error_with_remedy(
        outputs,
        withheld,
        "Name the needed variables in [workflow.acceptance_execution] environment_allowlist; a run with no such section must add the section to forward a variable, and that moves its checks to the scratch site",
    )
}

pub(super) fn withheld_error_with_remedy(
    outputs: &[&[u8]],
    withheld: &BTreeSet<String>,
    remedy: &str,
) -> Option<String> {
    let outputs: Vec<String> = (outputs.iter())
        .map(|output| String::from_utf8_lossy(output).into_owned())
        .collect();
    let named: Vec<&str> = (withheld.iter())
        .filter(|name| archon_shell::data_environment::check_data_variable(name).is_ok())
        .filter(|name| outputs.iter().any(|output| says_missing(name, output)))
        .map(String::as_str)
        .collect();
    if named.is_empty() {
        if let Some(note) = withheld_note(outputs_as_bytes(&outputs).as_slice(), withheld) {
            eprintln!("{note}");
        }
        return None;
    }
    let list = named.join(", ");
    Some(format!(
        "the check failed saying the host variable(s) {list} are missing, and this check site withholds them, so the failure is no verdict. {remedy}. If the check tests the missing-variable message itself, unset {list} in the environment archon starts with instead. Otherwise the check must not read them"
    ))
}

fn outputs_as_bytes(outputs: &[String]) -> Vec<&[u8]> {
    outputs.iter().map(|output| output.as_bytes()).collect()
}

/// A name without a recognised absence diagnostic does not establish cause.
/// Preserve the exit status and verdict, and attach a visible note. Only names
/// really withheld from this child are considered; values are never disclosed.
pub fn withheld_note(outputs: &[&[u8]], withheld: &BTreeSet<String>) -> Option<String> {
    let named: Vec<&str> = withheld
        .iter()
        .filter(|name| archon_shell::data_environment::check_data_variable(name).is_ok())
        .filter(|name| {
            let escaped = regex::escape(name);
            let pattern = if cfg!(windows) {
                format!("(?i)\\b{escaped}\\b")
            } else {
                format!("\\b{escaped}\\b")
            };
            let names = Regex::new(&pattern).expect("escaped variable regex");
            outputs
                .iter()
                .any(|output| names.is_match(&String::from_utf8_lossy(output)))
                && !outputs
                    .iter()
                    .any(|output| says_missing(name, &String::from_utf8_lossy(output)))
        })
        .map(String::as_str)
        .collect();
    (!named.is_empty()).then(|| format!(
        "Note: output mentions withheld variable(s) {}; no recognised absence diagnostic establishes an environment failure. The verifier's real result is retained.", named.join(", ")
    ))
}
