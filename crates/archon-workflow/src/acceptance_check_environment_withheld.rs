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

/// Remove quoted phrases before considering *any* absence pattern. Quoted
/// variable names and schema labels remain: `'NAME' is required` and
/// `{ NAME: ['Required'] }` are diagnostics, not quoted expectations.
fn unquoted_diagnostics(output: &str) -> String {
    let mut text = output.to_owned();
    let mut at = 0;
    while at < output.len() {
        let ch = output[at..].chars().next().expect("character boundary");
        if !matches!(ch, '\'' | '"' | '`') {
            at += ch.len_utf8();
            continue;
        }
        // A contraction's apostrophe does not open a quoted diagnostic.
        if ch == '\''
            && output[..at]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric)
            && output[at + 1..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric)
        {
            at += 1;
            continue;
        }
        let Some(close) = output[at + 1..].find(ch).map(|offset| at + 1 + offset) else {
            at += 1;
            continue;
        };
        let inside = &output[at + 1..close];
        if inside
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && c != '_')
        {
            // Spaces preserve separation so removing a quote cannot invent a claim.
            text.replace_range(at..close + 1, &" ".repeat(close + 1 - at));
        }
        at = close + 1;
    }
    text
}

/// A withheld variable needs an absence claim, not a mention. Names may be
/// quoted alone. Entire quoted phrases (including multiline expectations)
/// are removed before matching, equally for every diagnostic shape.
fn says_missing(name: &str, output: &str) -> bool {
    let output = unquoted_diagnostics(output);
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
            r#"(?:^\s*|:\s*|\bplease\s+)(?:set|export|provide)\s+(?:the\s+)?['"`$]?\b{name}\b"#
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
        return None;
    }
    let list = named.join(", ");
    Some(format!(
        "the check failed saying the host variable(s) {list} are missing, and this check site withholds them, so the failure is no verdict. {remedy}. If the check tests the missing-variable message itself, unset {list} in the environment archon starts with instead. Otherwise the check must not read them"
    ))
}
