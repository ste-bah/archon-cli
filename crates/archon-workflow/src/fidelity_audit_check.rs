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
    if quote.trim().is_empty() {
        return Err((
            RefusalKind::BlankRequired(key),
            format!("{what} quotes nothing from the task in `{key}`"),
        ));
    }
    if !quote_occurrences(&task.text, quote).is_some_and(|count| count > 0) {
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
    let quote = normalized_quote_for_anchor(quote);
    let (words, chars) = (quote.split(' ').count(), quote.chars().count());
    if words < MIN_SUPPORT_WORDS && chars < MIN_SUPPORT_CHARS {
        return weak(format!(
            "{what} quotes {words} words ({chars} characters) in `supporting_text`; it needs at least {MIN_SUPPORT_WORDS} words or {MIN_SUPPORT_CHARS} characters of the task's prose"
        ));
    }
    let found = quote_occurrences(&task.text, quote.as_str()).unwrap_or(0);
    if found != 1 {
        return weak(format!(
            "{what} quotes text in `supporting_text` that appears {found} times in task '{}'; quote a passage that appears exactly once",
            task.task_id
        ));
    }
    for region in [Region::Fence, Region::Comment, Region::Heading] {
        if !quote_occurrences(&masked(&task.text, region), quote.as_str())
            .is_some_and(|count| count > 0)
        {
            return weak(format!(
                "{what} quotes text in `supporting_text` that lies in or across {} of task '{}'; quote the task's prose",
                region.name(),
                task.task_id
            ));
        }
    }
    Ok(())
}

/// Count quote matches after the byte-for-byte check. The fallback is a
/// bounded text normalization, not a similarity score. For ellipses, every
/// piece must contain a complete word token also present in the task, then
/// each piece is located after the previous one so pieces are ordered and do
/// not overlap.
fn quote_occurrences(task: &str, quote: &str) -> Option<usize> {
    if task.contains(quote) {
        return Some(task.matches(quote).count());
    }
    let elisions = quote.replace("...", "…");
    let pieces = elisions.split('…').collect::<Vec<_>>();
    let elided = pieces.len() > 1;
    let task = normalize_text(task, false);
    let pieces = pieces
        .into_iter()
        .map(|piece| normalize_text(piece, true))
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>();
    if pieces.is_empty() {
        return Some(0);
    }
    if elided && pieces.iter().any(|piece| !has_word_token_in(piece, &task)) {
        return Some(0);
    }
    if elided {
        let mut found_chains = 0;
        for (start, first) in task.match_indices(&pieces[0]) {
            let mut cursor = start + first.len();
            let mut complete = true;
            for piece in &pieces[1..] {
                let Some(offset) = task[cursor..].find(piece) else {
                    complete = false;
                    break;
                };
                cursor += offset + piece.len();
            }
            if complete {
                found_chains += 1;
            }
        }
        return Some(found_chains);
    }
    Some(task.matches(&pieces[0]).count())
}

/// At least one complete Unicode alphanumeric token from a piece must occur
/// in the task. This keeps an ellipsis from passing on punctuation or a
/// fragment such as a single boundary mark, without an arbitrary size cap.
fn has_word_token_in(piece: &str, task: &str) -> bool {
    word_tokens(piece).any(|token| word_tokens(task).any(|candidate| candidate == token))
}

fn word_tokens(text: &str) -> impl Iterator<Item = &str> {
    let mut starts = Vec::new();
    let mut start = None;
    for (index, ch) in text.char_indices() {
        if ch.is_alphanumeric() {
            start.get_or_insert(index);
        } else if let Some(begin) = start.take() {
            starts.push((begin, index));
        }
    }
    if let Some(begin) = start {
        starts.push((begin, text.len()));
    }
    starts.into_iter().map(|(begin, end)| &text[begin..end])
}

fn normalized_quote_for_anchor(quote: &str) -> String {
    quote
        .replace("...", "…")
        .split('…')
        .map(|piece| normalize_text(piece, true))
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>()
        .join(" … ")
}

/// Normalize quote punctuation and Markdown wrappers. NFKC is omitted because
/// this crate has no direct or workspace normalization dependency; quote and
/// dash forms, whitespace and formatting remain deliberately bounded.
fn normalize_text(text: &str, is_quote: bool) -> String {
    let mut normalized = String::with_capacity(text.len());
    let chars = text.replace("\\n", " ").chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '‘' | '’' | '“' | '”' | '«' | '»' | '‹' | '›' => normalized.push('"'),
            '–' | '—' | '‑' | '−' => normalized.push('-'),
            '`' => {}
            '*' | '_' => {
                let marker = chars[index];
                let mut end = index + 1;
                while chars.get(end) == Some(&marker) {
                    end += 1;
                }
                let run = end - index;
                let previous = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
                let next = chars.get(end).copied();
                let can_open = next.is_some_and(|ch| !ch.is_whitespace());
                let can_close = previous.is_some_and(|ch| !ch.is_whitespace());
                let paired = chars[end..]
                    .windows(run)
                    .any(|window| window.iter().all(|ch| *ch == marker))
                    || chars[..index]
                        .windows(run)
                        .any(|window| window.iter().all(|ch| *ch == marker));
                let boundary_emphasis = previous
                    .is_none_or(|ch| ch.is_whitespace() || ch.is_ascii_punctuation())
                    || next.is_none_or(|ch| ch.is_whitespace() || ch.is_ascii_punctuation());
                if paired && (can_open || can_close) && (marker == '*' || boundary_emphasis) {
                    index = end;
                } else {
                    normalized.extend(chars[index..end].iter());
                    index = end;
                }
                continue;
            }
            ch => normalized.push(ch),
        }
        index += 1;
    }
    let mut collapsed = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if is_quote {
        let opens = collapsed.matches('[').count();
        let closes = collapsed.matches(']').count();
        if closes > opens {
            collapsed.retain(|ch| ch != ']');
        } else if opens > closes {
            collapsed.retain(|ch| ch != '[');
        }
        collapsed = collapsed
            .trim_matches(|ch: char| {
                ch.is_ascii_punctuation() || matches!(ch, '…' | '“' | '”' | '’' | '‘')
            })
            .to_string();
    }
    collapsed
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

#[cfg(test)]
#[path = "fidelity_audit_check_tests.rs"]
mod tests;
