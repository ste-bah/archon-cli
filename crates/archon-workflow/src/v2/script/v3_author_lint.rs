//! Pre-flight lint (Issue-19): a script that defines its own status predicate
//! over the envelope's evidence arrays is a defect the planner must fix.
//!
//! The reference says status predicates MUST be the runtime globals
//! `accepted(env)` / `usable(env)`. Every authored script so far wrote its own
//! anyway — live (wf-719ff3b0) `isAccepted(env)` combined `env.status` with
//! `env.files_changed.length` / `env.commands_run.length`, the arrays sat under
//! `env.result`, and an accepted verify was remediated. The host now mirrors
//! the arrays at the top level, so that script is no longer wrong — but the
//! next hand-rolled predicate will disagree with the host some other way, and
//! a predicate that disagrees does not fail the run, it loops it.
//!
//! The lint is deliberately narrow: it flags a FUNCTION whose name reads as a
//! status predicate (a word starting with `accept`/`usable`/`succe`, or the
//! word `ok`/`okay`/`pass`/`passed`/`passes`) AND whose body reads
//! `.files_changed` or `.commands_run` AND whose body reads `.status` — the
//! three together are a status predicate re-deriving the host's rule. A
//! reporting helper (`summarize`, `boundedEvidenceFor`, `remediationEvidence`)
//! is not named like a predicate; a predicate that delegates to the prelude
//! (`if (typeof accepted === 'function') return accepted(env)`) reads no
//! array; and the prelude itself is never handed to this lint — only the
//! authored source is.

use super::*;

/// Every hand-rolled status predicate in `source`, as planner-facing defects.
pub fn hand_rolled_predicate_defects(source: &str) -> Vec<String> {
    let array_read = regex::Regex::new(r"\.(files_changed|commands_run)\b").expect("static regex");
    let status_read = regex::Regex::new(r"\.status\b").expect("static regex");
    let mut defects = Vec::new();
    for (name, body) in function_definitions(source) {
        if !predicate_name(&name) || !array_read.is_match(&body) || !status_read.is_match(&body) {
            continue;
        }
        let reads = array_read
            .captures_iter(&body)
            .map(|c| format!(".{}", &c[1]))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join("/");
        defects.push(format!(
            "the script defines its own status predicate `{name}` reading `{reads}` beside `.status` — the runtime globals `accepted(env)` / `usable(env)` carry the host's rule (evidence lives under `env.result.*`, the top level holds compact mirrors); delete `{name}` and write `if (usable(impl) && accepted(check))`"
        ));
    }
    defects
}

/// Pre-flight for a FRESHLY authored draft: the dry-run plan check plus the
/// source lints, every defect in one message. The persisted-script resume path
/// runs the plan check alone, so a script an earlier host accepted is not
/// refused on resume for a lint added after it was written.
pub async fn validate_authored_draft(
    source: &str,
    expected_task_ids: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    let mut defects = Vec::new();
    if let Err(plan) = validate_authored_plan(source, expected_task_ids).await {
        defects.push(plan);
    }
    defects.extend(hand_rolled_predicate_defects(source));
    if defects.is_empty() {
        Ok(())
    } else {
        Err(defects.join("; AND "))
    }
}

/// Whether an identifier reads as a status predicate: split on camelCase and
/// underscores, then look at the words — so `isAccepted`, `impl_usable`,
/// `checkOk`, `verifyPassed` and `implSucceeded` match while `lookupTask`,
/// `tokenBudget`, `hookFor`, `summarize` and `boundedEvidenceFor` do not.
fn predicate_name(name: &str) -> bool {
    identifier_words(name).iter().any(|word| {
        word.starts_with("accept")
            || word.starts_with("usable")
            || word.starts_with("succe")
            || matches!(word.as_str(), "ok" | "okay" | "pass" | "passed" | "passes")
    })
}

fn identifier_words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for ch in name.chars() {
        if ch == '_' || ch == '$' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else if ch.is_ascii_uppercase() && !current.is_empty() {
            words.push(std::mem::take(&mut current));
            current.push(ch.to_ascii_lowercase());
        } else {
            current.push(ch.to_ascii_lowercase());
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// `(name, body)` for every named function in `source`: `function name(...)`,
/// and `const|let|var name = [async] (...) => ...` / `= [async] function`.
fn function_definitions(source: &str) -> Vec<(String, String)> {
    let declaration = regex::Regex::new(
        r"(?x)
        \b(?:async\s+)?function\s+(?P<fname>[A-Za-z_$][\w$]*)
        | \b(?:const|let|var)\s+(?P<vname>[A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?
          (?: function\b | \([^()]*\)\s*=> | [A-Za-z_$][\w$]*\s*=> )",
    )
    .expect("static regex");
    declaration
        .captures_iter(source)
        .filter_map(|caps| {
            let name = caps
                .name("fname")
                .or_else(|| caps.name("vname"))?
                .as_str()
                .to_string();
            let body = definition_body(&source[caps.get(0)?.end()..]);
            Some((name, body))
        })
        .collect()
}

/// The text of a function's body starting just after its declaration head
/// (for `function name` that is its parameter list, which balances to depth
/// zero before the block opens): the brace-balanced block when one follows,
/// otherwise (an expression-bodied arrow) the rest of the statement up to the
/// first newline at bracket depth zero. Quoted strings and template literals
/// are skipped whole.
fn definition_body(rest: &str) -> String {
    let bytes = rest.as_bytes();
    let mut i = 0;
    let mut depth: i32 = 0;
    let mut seen_brace = false;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'\'' | b'"' | b'`' => quote = Some(b),
            b'{' | b'(' | b'[' => {
                if b == b'{' {
                    seen_brace = true;
                }
                depth += 1;
            }
            b'}' | b')' | b']' => {
                depth -= 1;
                if depth <= 0 && seen_brace {
                    return rest[..=i].to_string();
                }
            }
            b'\n' if depth <= 0 && !seen_brace => return rest[..i].to_string(),
            _ => {}
        }
        i += 1;
    }
    rest.to_string()
}

#[cfg(test)]
#[path = "v3_author_lint_tests.rs"]
mod v3_author_lint_tests;
