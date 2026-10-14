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
    let normalized = normalized_quote_for_anchor(quote);
    let (words, chars) = (normalized.split(' ').count(), normalized.chars().count());
    if words < MIN_SUPPORT_WORDS && chars < MIN_SUPPORT_CHARS {
        return weak(format!(
            "{what} quotes {words} words ({chars} characters) in `supporting_text`; it needs at least {MIN_SUPPORT_WORDS} words or {MIN_SUPPORT_CHARS} characters of the task's prose"
        ));
    }
    // Keep the critic's original quote for provenance matching. Re-matching
    // its normalized form can change an exact excerpt cut inside Markdown
    // (for example, midway through an inline-code span) into unmatched syntax.
    let found = quote_occurrences(&task.text, quote).unwrap_or(0);
    if found != 1 {
        return weak(format!(
            "{what} quotes text in `supporting_text` that appears {found} times in task '{}'; quote a passage that appears exactly once",
            task.task_id
        ));
    }
    for region in [Region::Fence, Region::Comment, Region::Heading] {
        if !quote_occurrences(&masked(&task.text, region), quote).is_some_and(|count| count > 0) {
            return weak(format!(
                "{what} quotes text in `supporting_text` that lies in or across {} of task '{}'; quote the task's prose",
                region.name(),
                task.task_id
            ));
        }
    }
    Ok(())
}

/// Count exact matches before whole-quote normalized matches. Only when
/// neither matches can the critic confirmation path expand an ellipsis.
fn quote_occurrences(task: &str, quote: &str) -> Option<usize> {
    if task.contains(quote) {
        return Some(task.matches(quote).count());
    }
    let task = normalize_text(task, false);
    let quote = normalize_text(quote, true);
    if quote.is_empty() {
        return Some(0);
    }
    Some(task.matches(&quote).count())
}

/// Expand an elided supporting quote to its one contiguous task passage.
/// Matches must be unique, word bounded and contained in one paragraph.
pub fn expand_supporting_quote(task: &str, quote: &str) -> Option<String> {
    let quote = quote.replace("...", "…");
    let pieces = quote
        .split('…')
        .map(|piece| normalize_text(piece, true))
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>();
    if pieces.len() < 2 {
        return None;
    }
    let mut chains = Vec::new();
    for paragraph in paragraphs(task) {
        let normalized = normalize_text(paragraph, false);
        for (start, first) in word_bounded_matches(&normalized, &pieces[0]) {
            let ends = chain_ends(&normalized, &pieces[1..], start + first.len());
            for end in ends {
                chains.push((paragraph, start, end));
                if chains.len() > 1 {
                    return None;
                }
            }
        }
    }
    if chains.len() != 1 {
        return None;
    }
    let (paragraph, start, end) = chains.pop()?;
    let raw_start = raw_offset_for_normalized(paragraph, start)?;
    let raw_end = raw_offset_for_normalized(paragraph, end)?;
    Some(paragraph.get(raw_start..raw_end)?.trim_end().to_string())
}

fn paragraphs(text: &str) -> Vec<&str> {
    let mut paragraphs = Vec::new();
    let mut paragraph_start = 0;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if line.trim().is_empty() {
            if paragraph_start < offset {
                paragraphs.push(&text[paragraph_start..offset]);
            }
            paragraph_start = offset + line.len();
        }
        offset += line.len();
    }
    if paragraph_start < text.len() {
        paragraphs.push(&text[paragraph_start..]);
    }
    paragraphs
}

fn chain_ends(text: &str, pieces: &[String], cursor: usize) -> Vec<usize> {
    if pieces.is_empty() {
        return vec![cursor];
    }
    let mut ends = Vec::new();
    for (offset, matched) in word_bounded_matches(&text[cursor..], &pieces[0]) {
        ends.extend(chain_ends(
            text,
            &pieces[1..],
            cursor + offset + matched.len(),
        ));
        if ends.len() > 1 {
            ends.truncate(2);
            break;
        }
    }
    ends
}

/// Compare two complete quote strings under the provenance normalization.
pub fn same_normalized_quote(left: &str, right: &str) -> bool {
    normalize_text(left, true) == normalize_text(right, true)
}

/// Whether a quote already matches the task exactly or under whole-quote
/// provenance normalization.
pub fn task_contains_quote(task: &str, quote: &str) -> bool {
    !quote.trim().is_empty() && quote_occurrences(task, quote).is_some_and(|count| count > 0)
}

fn word_bounded_matches<'a>(text: &'a str, piece: &str) -> Vec<(usize, &'a str)> {
    text.match_indices(piece)
        .filter(|(start, matched)| {
            let before = text[..*start].chars().next_back();
            let after = text[*start + matched.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
        .collect()
}

fn raw_offset_for_normalized(text: &str, target: usize) -> Option<usize> {
    let mut previous = 0;
    for (offset, _) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        if normalize_text(&text[..offset], false).len() > target {
            return Some(previous);
        }
        previous = offset;
    }
    Some(text.len())
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
    let paired_emphasis = paired_emphasis_markers(&chars);
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '‘' | '’' | '“' | '”' | '«' | '»' | '‹' | '›' => normalized.push('"'),
            '–' | '—' | '‑' | '−' => normalized.push('-'),
            '`' => {
                let mut end = index + 1;
                while chars.get(end) == Some(&'`') {
                    end += 1;
                }
                let run = end - index;
                let mut search = end;
                let mut close = None;
                while search < chars.len() {
                    if chars[search] != '`' {
                        search += 1;
                        continue;
                    }
                    let mut run_end = search + 1;
                    while chars.get(run_end) == Some(&'`') {
                        run_end += 1;
                    }
                    if run_end - search == run {
                        close = Some(search);
                        break;
                    }
                    search = run_end;
                }
                if let Some(close) = close {
                    normalized.extend(chars[end..close].iter());
                    index = close + run;
                    continue;
                }
                normalized.extend(chars[index..end].iter());
                index = end;
                continue;
            }
            '*' | '_' => {
                let marker = chars[index];
                let mut end = index + 1;
                while chars.get(end) == Some(&marker) {
                    end += 1;
                }
                let previous = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
                let next = chars.get(end).copied();
                let can_open = next.is_some_and(|ch| !ch.is_whitespace());
                let can_close = previous.is_some_and(|ch| !ch.is_whitespace());
                let boundary_emphasis = previous
                    .is_none_or(|ch| ch.is_whitespace() || ch.is_ascii_punctuation())
                    || next.is_none_or(|ch| ch.is_whitespace() || ch.is_ascii_punctuation());
                if paired_emphasis[index]
                    && (can_open || can_close)
                    && (marker == '*' || boundary_emphasis)
                {
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

fn paired_emphasis_markers(chars: &[char]) -> Vec<bool> {
    let mut paired = vec![false; chars.len()];
    let mut openings = Vec::<(char, usize, usize)>::new();
    let mut index = 0;
    while index < chars.len() {
        let marker = chars[index];
        if !matches!(marker, '*' | '_') {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while chars.get(end) == Some(&marker) {
            end += 1;
        }
        let previous = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(end).copied();
        let can_open = next.is_some_and(|ch| !ch.is_whitespace());
        let can_close = previous.is_some_and(|ch| !ch.is_whitespace());
        let opening = openings
            .iter()
            .rposition(|(open_marker, run, _)| *open_marker == marker && *run == end - index);
        if can_close {
            if let Some(open_index) = opening {
                let (_, _, open_start) = openings.remove(open_index);
                paired[open_start..open_start + (end - index)].fill(true);
                paired[index..end].fill(true);
            } else if can_open {
                openings.push((marker, end - index, index));
            }
        } else if can_open {
            openings.push((marker, end - index, index));
        }
        index = end;
    }
    paired
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
