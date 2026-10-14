//! Checking one verdict's provenance against the cluster it answers: the
//! weakest or supporting task is in the cluster, and its quote is verbatim
//! in that task. Split from `fidelity_audit.rs` so that file stays under its
//! size budget; a stored verdict passes exactly this check again before it
//! is reused (#434).

use super::{
    ClaimingTask, FidelityVerdict, MAX_QUOTE_CHARS, MAX_REASON_CHARS, MIN_SUPPORT_CHARS,
    MIN_SUPPORT_WORDS, RefusalKind,
};

/// Check one verdict's provenance against its cluster, cutting an over-long
/// `reason`, `quoted_task_text` or `supporting_text` to its limit first.
///
/// Issue-42: a cluster failed the gate twice with "reason longer than 400
/// characters" although both its verdicts were true — the critic had answered
/// the question and merely said too much, and the gate turned a verbose pass
/// into an operational failure. Length is now a cut, not a refusal: the reason
/// keeps its first [`MAX_REASON_CHARS`] characters plus `…`; a quote keeps
/// its first [`MAX_QUOTE_CHARS`] characters and nothing more, so the verbatim
/// check below runs on exactly the text the finding will print. Provenance is
/// still refused outright: a weakest or supporting task outside the cluster,
/// a false verdict that names no weakest task or quotes nothing, a true
/// verdict that quotes nothing in support (#434), or a quote the named task
/// does not contain.
pub(super) fn check_verdict(
    verdict: &mut FidelityVerdict,
    tasks: &[ClaimingTask],
) -> Result<(), (RefusalKind, String)> {
    let id = &verdict.obligation_id;
    let other = |message: String| Err((RefusalKind::Other, message));
    if verdict.reason.trim().is_empty() {
        return Err((
            RefusalKind::BlankRequired("reason"),
            format!("verdict for {id} has an empty reason"),
        ));
    }
    if verdict.reason.chars().count() > MAX_REASON_CHARS {
        let mut cut: String = verdict.reason.chars().take(MAX_REASON_CHARS).collect();
        cut.push('…');
        verdict.reason = cut;
    }
    cut_quote(&mut verdict.quoted_task_text);
    cut_quote(&mut verdict.supporting_text);
    let named = |task_id: &str| tasks.iter().find(|task| task.task_id == task_id);
    let weakest = named(&verdict.weakest_task_id);
    if verdict.necessarily_true {
        if !verdict.weakest_task_id.is_empty() && weakest.is_none() {
            return other(format!(
                "verdict for {id} names weakest task '{}' which is not in the audited cluster",
                verdict.weakest_task_id
            ));
        }
        let Some(supporting) = named(&verdict.supporting_task_id) else {
            return other(format!(
                "true verdict for {id} names supporting task '{}' which is not in the audited cluster",
                verdict.supporting_task_id
            ));
        };
        let what = format!("true verdict for {id}");
        verbatim(
            &what,
            "supporting_text",
            &verdict.supporting_text,
            supporting,
        )?;
        return anchored(&what, &verdict.supporting_text, supporting);
    }
    let Some(weakest) = weakest else {
        return other(format!(
            "false verdict for {id} names weakest task '{}' which is not in the audited cluster",
            verdict.weakest_task_id
        ));
    };
    verbatim(
        &format!("false verdict for {id}"),
        "quoted_task_text",
        &verdict.quoted_task_text,
        weakest,
    )?;
    // The critic judges whether these sentences weaken the obligation; the
    // host only verifies that each sentence came verbatim from this task.
    for quote in &mut verdict.weakening_quotes {
        cut_quote(quote);
        verbatim(
            &format!("false verdict for {id}"),
            "weakening_quotes",
            quote,
            weakest,
        )?;
    }
    Ok(())
}

/// Does `verdict`, as stored, still pass the check a fresh reply passes
/// against `tasks`? A store reuses a verdict only over byte-identical task
/// texts, so this is the re-validation of a record, never a reuse anchor on
/// its own (#434).
pub fn verdict_holds(verdict: &FidelityVerdict, tasks: &[ClaimingTask]) -> bool {
    let mut checked = verdict.clone();
    check_verdict(&mut checked, tasks).is_ok() && checked == *verdict
}

/// A cut that lands after the backslash of an escaped newline would leave a
/// stray backslash no task text contains; a shorter prefix of a verbatim
/// excerpt is still verbatim, so it is dropped.
fn cut_quote(quote: &mut String) {
    if quote.chars().count() > MAX_QUOTE_CHARS {
        let cut: String = quote.chars().take(MAX_QUOTE_CHARS).collect();
        *quote = cut.trim_end_matches('\\').to_string();
    }
}

/// `quote`, the value of `key`, must be non-empty and verbatim in `task`.
fn verbatim(
    what: &str,
    key: &'static str,
    quote: &str,
    task: &ClaimingTask,
) -> Result<(), (RefusalKind, String)> {
    let quote = collapse_whitespace(quote);
    if quote.is_empty() {
        return Err((
            RefusalKind::BlankRequired(key),
            format!("{what} quotes nothing from the task in `{key}`"),
        ));
    }
    if !collapse_whitespace(&task.text).contains(&quote) {
        return Err((
            RefusalKind::NotVerbatim(key),
            format!(
                "{what} quotes text in `{key}` that does not appear verbatim in task '{}'",
                task.task_id
            ),
        ));
    }
    Ok(())
}

/// A true verdict's `supporting_text` (already verbatim in `task`) must be
/// an anchor a reader can check (#434): at least [`MIN_SUPPORT_WORDS`] words
/// or [`MIN_SUPPORT_CHARS`] characters, found exactly once in the task, and
/// in the task's prose — not in or across a code fence, an HTML comment or
/// a heading line, where words can stand without obliging anything.
fn anchored(what: &str, quote: &str, task: &ClaimingTask) -> Result<(), (RefusalKind, String)> {
    let weak = |message: String| Err((RefusalKind::WeakAnchor, message));
    let quote = collapse_whitespace(quote);
    let (words, chars) = (quote.split(' ').count(), quote.chars().count());
    if words < MIN_SUPPORT_WORDS && chars < MIN_SUPPORT_CHARS {
        return weak(format!(
            "{what} quotes {words} words ({chars} characters) in `supporting_text`; it needs at least {MIN_SUPPORT_WORDS} words or {MIN_SUPPORT_CHARS} characters of the task's prose"
        ));
    }
    let found = collapse_whitespace(&task.text)
        .matches(quote.as_str())
        .count();
    if found != 1 {
        return weak(format!(
            "{what} quotes text in `supporting_text` that appears {found} times in task '{}'; quote a passage that appears exactly once",
            task.task_id
        ));
    }
    for region in [Region::Fence, Region::Comment, Region::Heading] {
        if !collapse_whitespace(&masked(&task.text, region)).contains(quote.as_str()) {
            return weak(format!(
                "{what} quotes text in `supporting_text` that lies in or across {} of task '{}'; quote the task's prose",
                region.name(),
                task.task_id
            ));
        }
    }
    Ok(())
}

/// A part of a task text where words are not prose a result rests on.
#[derive(Clone, Copy)]
enum Region {
    Fence,
    Comment,
    Heading,
}

impl Region {
    fn name(self) -> &'static str {
        match self {
            Region::Fence => "a code fence",
            Region::Comment => "an HTML comment",
            Region::Heading => "a heading line",
        }
    }
}

/// A word no quote contains: what a masked region becomes.
const MASK: &str = " \u{1} ";

/// `text` with every `region` replaced by [`MASK`], so a quote found in the
/// result lies wholly outside that region.
fn masked(text: &str, region: Region) -> String {
    match region {
        Region::Comment => {
            let mut out = String::new();
            let mut rest = text;
            while let Some(open) = rest.find("<!--") {
                out.push_str(&rest[..open]);
                out.push_str(MASK);
                rest = match rest[open + 4..].find("-->") {
                    Some(close) => &rest[open + 4 + close + 3..],
                    None => "",
                };
            }
            out.push_str(rest);
            out
        }
        Region::Fence | Region::Heading => {
            let mut fenced = false;
            text.lines()
                .map(|line| {
                    let trimmed = line.trim_start();
                    let marker = trimmed.starts_with("```") || trimmed.starts_with("~~~");
                    let hidden = match region {
                        Region::Fence => {
                            let inside = fenced || marker;
                            fenced ^= marker;
                            inside
                        }
                        _ => trimmed.starts_with('#'),
                    };
                    if hidden { MASK } else { line }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

/// Whitespace-insensitive comparison: a model that re-wraps a line has still
/// quoted it, while one that paraphrases has not. A literal two-character
/// `\n` counts as whitespace too — asked to escape newlines, a critic that
/// escapes them twice has still copied the line, and the decoded backslash-n
/// is packaging, not a change to the words.
fn collapse_whitespace(text: &str) -> String {
    text.replace("\\n", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
