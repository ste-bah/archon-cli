//! What counts as NEW CONTENT for the no-progress window (Issue 391).
//!
//! The window used to renew on any admitted call whose tool and canonical
//! arguments the session had not used before. A re-read of a file at a new
//! offset or limit has new arguments and returns nothing new: live, read-only
//! authors paged the same files and their own transcript in fresh slices for
//! hours, and every slice renewed the window.
//!
//! So a call that READS is judged on what it returned to the model:
//!
//! - `Read` returns numbered lines. It is novel only when it returned a line
//!   number this session has not been given from that file, or a line whose
//!   text changed since it was given. Its arguments do not matter: a new
//!   slice of seen lines is not novel, and the same slice of a changed file
//!   is.
//! - `read-own-evidence` returns this session's own transcript: everything
//!   in it is a message the session already sent or received. It is never
//!   novel, whatever slice it returns.
//! - `Grep`, `Glob`, and any other call whose arguments page through a
//!   result ([`PAGING_KEYS`]), keep the lines they returned by source; a
//!   paged call is novel only when its arguments are new AND it returned a
//!   line not given before from the same source.
//!
//! The source is the call with its paging arguments removed: the file, or the
//! search with its pattern, path and flags. Every other call keeps the
//! argument rule, so a repeated identical call never renews, and an error
//! result (a missing file, an offset past the end) is judged by its arguments
//! as before. What is compared is the content the model received, after the
//! provider cap, so a part the cap cut away stays unseen.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use serde_json::Value;

/// Arguments that select a slice of one result rather than a different one.
const PAGING_KEYS: &[&str] = &["offset", "limit", "char_offset", "head_limit"];

/// Arguments that change neither which content a call returns nor its text.
const PRESENTATION_KEYS: &[&str] = &["force_refresh"];

/// The tool that reads this session's own transcript back
/// (`evidence_recovery::ReadOwnEvidence`).
const OWN_EVIDENCE_TOOL: &str = "read-own-evidence";

/// One finished call of a tool round, as the window judges it.
#[derive(Debug, Clone)]
pub(super) struct ObservedCall {
    pub(super) name: String,
    /// The decoded input; `None` when it never parsed.
    pub(super) input: Option<Value>,
    /// The input exactly as the model sent it.
    pub(super) raw_input: String,
    /// The guard refused it: it never ran, so it is never progress.
    pub(super) refused: bool,
    pub(super) is_error: bool,
    /// The result text the model received (after the provider cap).
    pub(super) delivered: String,
}

impl ObservedCall {
    /// The canonical arguments, the same text the repeat guard compares.
    pub(super) fn arguments(&self) -> String {
        self.input.as_ref().map_or_else(
            || self.raw_input.trim().to_string(),
            archon_tools::repeat_tool_guard::canonical_arguments,
        )
    }
}

/// What a call contributed, for the window's record of the last novelty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Novelty {
    /// Nothing this session had not been given.
    None,
    /// A call with arguments this session had not used.
    NewCall,
    /// Arguments used before, but the result held content not given before
    /// (a file that changed between two identical reads).
    NewContent,
}

/// Everything this session has been given, by source.
#[derive(Debug, Default)]
pub(super) struct ContentLedger {
    calls: HashSet<u64>,
    /// `Read` lines: (source, line number) -> digest of the text given.
    lines: HashMap<(u64, u64), u64>,
    /// Lines of searches and paged results: digest of (source, line).
    given: HashSet<u64>,
}

impl ContentLedger {
    /// Judge one admitted call and record what it returned.
    pub(super) fn observe(&mut self, call: &ObservedCall, arguments: &str) -> Novelty {
        let new_call = self
            .calls
            .insert(digest(&("tool", call.name.as_str(), arguments)));
        if call.name == OWN_EVIDENCE_TOOL {
            return Novelty::None;
        }
        let by_arguments = if new_call {
            Novelty::NewCall
        } else {
            Novelty::None
        };
        let Some(input) = call.input.as_ref().filter(|_| !call.is_error) else {
            return by_arguments;
        };
        let source = source_digest(&call.name, input);
        if call.name == "Read"
            && let Some(lines) = numbered_lines(&call.delivered)
        {
            let mut novel = false;
            for (number, text) in lines {
                let text = digest(&text);
                novel |= self.lines.insert((source, number), text) != Some(text);
            }
            return match (novel, new_call) {
                (false, _) => Novelty::None,
                (true, true) => Novelty::NewCall,
                (true, false) => Novelty::NewContent,
            };
        }
        // A search's lines, and a paged result's, are kept by source, so a
        // later re-slice is judged against what earlier calls already gave.
        // Other results (a shell command's output) are not kept: their
        // arguments alone decide, as before.
        let paged = pages(input);
        if !paged
            && !archon_tools::workflow_read_guard::WorkflowReadGuard::is_read_only_inspection_tool(
                &call.name,
            )
        {
            return by_arguments;
        }
        let mut novel = false;
        for line in call.delivered.lines() {
            novel |= self.given.insert(digest(&(source, line)));
        }
        if novel || !paged {
            by_arguments
        } else {
            Novelty::None
        }
    }
}

fn pages(input: &Value) -> bool {
    input
        .as_object()
        .is_some_and(|object| PAGING_KEYS.iter().any(|key| object.contains_key(*key)))
}

/// The call with its paging (and presentation) arguments removed.
fn source_digest(name: &str, input: &Value) -> u64 {
    let mut source = input.clone();
    if let Some(object) = source.as_object_mut() {
        for key in PAGING_KEYS.iter().chain(PRESENTATION_KEYS) {
            object.remove(*key);
        }
    }
    digest(&(
        "source",
        name,
        archon_tools::repeat_tool_guard::canonical_arguments(&source),
    ))
}

/// `Read` output: `N\ttext` per line. Lines without a number — a note the
/// guard appended, the cap's omission marker — carry no file content. `None`
/// when no line has a number (a guard message in place of the content): the
/// call is then judged by its arguments.
fn numbered_lines(delivered: &str) -> Option<Vec<(u64, &str)>> {
    let lines = delivered
        .lines()
        .filter_map(|line| {
            let (number, text) = line.split_once('\t')?;
            let number = number.trim_start();
            if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            Some((number.parse().ok()?, text))
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then_some(lines)
}

fn digest(value: &impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
#[path = "progress_novelty_tests.rs"]
mod tests;
