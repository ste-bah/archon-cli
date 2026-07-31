//! Mechanical gauntlet — gates G-B(post) / G-C / G-D / G-F + P2b exemplar-leak.
//!
//! Byte-faithful port of `scripts/fcdp/gauntlet.py`. G-A and «Qnn» substitution live
//! elsewhere in this crate (`ga_compare`, `substitute_quote_ids`); this module covers
//! the remaining mechanical gates the orchestrator runs each cycle.
//!
//! Fidelity notes:
//!   * `fancy-regex` gives Python-`re`-equivalent backtracking + lookaround, needed for
//!     the G-D gendered-pronoun negative-lookahead (`(?!\s+\w+,)`, the appositive-exemption
//!     fix from PR#51 commit 7dde7fc8) and the straight-quote lookbehind (`(?<!\\)"`).
//!   * Defect/advisory context windows are sliced by CHARACTER (not byte) offset to match
//!     Python string slicing over multibyte prose (guillemets, em-dash, Greek terms).
//!   * Defect and advisory ordering reproduces gauntlet.py's append order exactly.

use crate::{Pack, QuoteBank};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct GauntletReport {
    pub defects: Vec<String>,
    pub advisories: Vec<String>,
    pub pass: bool,
}

/// One G-D matcher: plain (no lookaround) or fancy (lookaround), paired with its label.
enum Matcher {
    Plain(regex::Regex),
    Fancy(fancy_regex::Regex),
}

impl Matcher {
    /// First match's (byte_start, byte_end), mirroring `re.search`.
    fn find(&self, s: &str) -> Option<(usize, usize)> {
        match self {
            Matcher::Plain(re) => re.find(s).map(|m| (m.start(), m.end())),
            Matcher::Fancy(re) => re.find(s).ok().flatten().map(|m| (m.start(), m.end())),
        }
    }
}

fn plain(p: &str) -> regex::Regex {
    regex::Regex::new(p).expect("static gauntlet regex")
}
fn fancy(p: &str) -> fancy_regex::Regex {
    fancy_regex::Regex::new(p).expect("static gauntlet fancy-regex")
}

/// first `n` chars of `s` (Python `s[:n]`).
fn char_take(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `gd_draft[max(0, start-20) : end+20]` in CHARACTER offsets, from byte offsets.
fn context(s: &str, byte_start: usize, byte_end: usize) -> String {
    let cstart = s[..byte_start].chars().count();
    let cend = s[..byte_end].chars().count();
    let chars: Vec<char> = s.chars().collect();
    let a = cstart.saturating_sub(20);
    let b = (cend + 20).min(chars.len());
    chars[a..b].iter().collect()
}

/// 8-grams: lowercase, replace `[^a-z0-9\s]` with space, split on whitespace, join windows.
fn ngrams(text: &str, strip: &regex::Regex) -> HashSet<String> {
    let lowered = text.to_lowercase();
    let cleaned = strip.replace_all(&lowered, " ");
    let w: Vec<&str> = cleaned.split_whitespace().collect();
    let mut set = HashSet::new();
    if w.len() >= 8 {
        for i in 0..=w.len() - 8 {
            set.insert(w[i..i + 8].join(" "));
        }
    }
    set
}

fn strip_backtick_apos(s: &str) -> &str {
    s.trim_matches(|c| c == '`' || c == '\'')
}
fn strip_locus_punct(s: &str) -> &str {
    s.trim_matches(|c| ".,;:".contains(c))
}
fn norm_locus(s: &str) -> String {
    s.replace("--", "-").replace('\u{2013}', "-")
}

/// Run the mechanical gauntlet. `draft` is the post-substitution text; `presub` (if given)
/// is the pre-substitution draft (for the G-B/D2 literal-quote check).
pub fn run_gauntlet(
    draft: &str,
    pack: &Pack,
    bank: &QuoteBank,
    presub: Option<&str>,
) -> GauntletReport {
    let mut defects: Vec<String> = Vec::new();
    let mut advisories: Vec<String> = Vec::new();

    // ── compiled patterns ──
    let found_span = plain(r#"["\u{201c}`]{1,2}([^"\u{201d}`']{1,40}?)["\u{201d}']{1,2}"#);
    let quote_span = plain(r"(?s)``(.+?)''");
    let marker = plain(r"«[A-Z]\d+[+@]?»");
    let straight_dq = fancy(r#"(?<!\\)""#);
    let ngram_strip = plain(r"[^a-z0-9\s]");
    let cite_re = plain(r"\(([^()]{2,60}?\d[^()]*?)\)");
    let split_comma_ws = plain(r"[,\s]+");
    let has_digit = plain(r"\d");
    let squote = plain(r"'([^']+)'");

    // foundation short-span allowances (<=3 words, author's own device)
    let foundation = &pack.p7_foundation;
    let mut found_spans: HashSet<String> = HashSet::new();
    for cap in found_span.captures_iter(foundation) {
        let m = &cap[1];
        if m.split_whitespace().count() <= 3 {
            found_spans.insert(strip_locus_punct(m.trim()).to_lowercase());
        }
    }
    let foundation_ok = |span: &str| -> bool {
        span.split_whitespace().count() <= 3
            && found_spans.contains(&strip_locus_punct(span.trim()).to_lowercase())
    };

    // ── G-B/D2: no literal quoted span in the pre-substitution draft ──
    if let Some(pre) = presub {
        for cap in quote_span.captures_iter(pre) {
            let s = &cap[1];
            if foundation_ok(s) {
                continue;
            }
            defects.push(format!(
                "G-B/D2: literal quoted span in pre-substitution draft (must enter via \u{ab}Qnn\u{bb}): ``{}''",
                char_take(s, 60)
            ));
        }
    }

    // ── G-B (post-substitution): quoted spans must trace to the bank ──
    let bank_texts: Vec<&str> = bank.values().map(|v| v.text.as_str()).collect();
    for cap in quote_span.captures_iter(draft) {
        let s = &cap[1];
        let inner_ok = bank_texts
            .iter()
            .any(|bt| s.trim() == strip_backtick_apos(bt) || format!("``{s}''") == *bt)
            || foundation_ok(s);
        if !inner_ok {
            if s.chars().count() > 60 {
                defects.push(format!(
                    "G-B: quoted span not covered by bank: ``{}...''",
                    char_take(s, 60)
                ));
            } else {
                defects.push(format!("G-B: quoted span not covered by bank: ``{s}''"));
            }
        }
    }
    // leftover markers = substitution incomplete
    if marker.is_match(draft) {
        defects.push("G-B: unsubstituted \u{ab}Qnn\u{bb} markers remain".to_string());
    }
    // straight double-quote outside LaTeX markup
    if straight_dq.is_match(draft).unwrap_or(false) {
        advisories.push(
            "G-B: straight double-quote characters present \u{2014} verify not quotation use"
                .to_string(),
        );
    }
    // unused ASSIGNED quotes
    for (qid, entry) in bank.iter() {
        let txt = strip_backtick_apos(&entry.text);
        if !draft.contains(txt) && !draft.contains(&format!("\u{ab}{qid}")) {
            defects.push(format!("G-B: assigned quote {qid} unused in draft"));
        }
    }

    // ── exemplar leakage: any 8-gram shared with a P2b exemplar (bank grams excluded) ──
    let mut bank_grams: HashSet<String> = HashSet::new();
    for v in bank.values() {
        for form in [
            v.text.clone(),
            format!("{} {}", v.text, v.cite),
            v.cite.clone(),
        ] {
            bank_grams.extend(ngrams(&form, &ngram_strip));
        }
    }
    let draft_grams: HashSet<String> = ngrams(draft, &ngram_strip)
        .difference(&bank_grams)
        .cloned()
        .collect();
    for ex in &pack.p2b_exemplars {
        let ex_grams: HashSet<String> = ngrams(&ex.text, &ngram_strip)
            .difference(&bank_grams)
            .cloned()
            .collect();
        let mut hits: Vec<&String> = draft_grams.intersection(&ex_grams).collect();
        if !hits.is_empty() {
            hits.sort();
            defects.push(format!(
                "G-B/exemplar-leak ({}): {}...",
                ex.movement_type,
                char_take(hits[0], 70)
            ));
        }
    }

    // ── G-C: citation rigor — paren-loci must trace to the pack ──
    let mut known_srcs = pack
        .p4a_quote_index
        .iter()
        .map(|q| format!("{} {}", q.source, q.locus))
        .collect::<Vec<_>>()
        .join(" ");
    known_srcs.push(' ');
    known_srcs.push_str(
        &bank
            .values()
            .map(|v| v.cite.clone())
            .collect::<Vec<_>>()
            .join(" "),
    );
    known_srcs.push(' ');
    known_srcs.push_str(foundation);
    known_srcs.push(' ');
    known_srcs.push_str(
        &pack
            .p5_evidence
            .iter()
            .map(|e| e.content.clone())
            .collect::<Vec<_>>()
            .join(" "),
    );
    let known_srcs_n = norm_locus(&known_srcs);
    for cap in cite_re.captures_iter(draft) {
        let cite = &cap[1];
        let tokens: Vec<String> = split_comma_ws
            .split(cite)
            .filter(|t| has_digit.is_match(t))
            .map(norm_locus)
            .collect();
        if !tokens.is_empty() && !tokens.iter().any(|t| known_srcs_n.contains(t)) {
            defects.push(format!(
                "G-C: locus '({cite})' does not trace to pack (memory locus?)"
            ));
        }
    }
    if draft.contains("******") {
        advisories.push(
            "G-C: ****** placeholder(s) present \u{2014} expected for missing sources; verify at handoff"
                .to_string(),
        );
    }

    // ── G-D: terminology locks (greppable subset) ──
    let gd_patterns: Vec<(Matcher, &str)> = vec![
        (
            Matcher::Plain(plain(r"phantasmat")),
            "retired stem 'phantasmat-'",
        ),
        (
            Matcher::Plain(plain(r"\\textbf\{[^}]{2,40}\.\}")),
            "bold run-in head",
        ),
        (
            Matcher::Fancy(fancy(
                r"\b(she|he|his|her|hers)\b(?!\s+\w+,)[^.]{0,40}\bplayer\b|\bplayer\b[^.]{0,60}\b(she|he|his|her|hers)\b(?!\s+\w+,)",
            )),
            "gendered pronoun bound to 'the player'",
        ),
        (
            Matcher::Plain(plain(r"supplies the [a-z ]*anchor")),
            "'supplies the ... anchor' pattern",
        ),
        (
            Matcher::Plain(plain(
                r"\b[Aa]s (discussed|noted|mentioned) (above|earlier|previously)\b",
            )),
            "explicit back-reference",
        ),
        (
            Matcher::Plain(plain(r"[\u{201c}\u{201d}\u{2018}\u{2019}]")),
            "Unicode quote character",
        ),
        (
            Matcher::Plain(plain(
                r"\bfaculty of (imagination|opinion|desire|perception)\b",
            )),
            "'faculty of X' instead of Greek term",
        ),
    ];
    // mask bank quotes so their content can't trip the locks
    let mut gd_draft = draft.to_string();
    for bt in &bank_texts {
        gd_draft = gd_draft.replace(strip_backtick_apos(bt), " [BANK-QUOTE] ");
    }
    for (m, label) in &gd_patterns {
        if let Some((start, end)) = m.find(&gd_draft) {
            defects.push(format!(
                "G-D: {label}: '{}'",
                context(&gd_draft, start, end)
            ));
        }
    }
    // P8 negative constraints, applied greppably against the draft
    for c in &pack.p8_negative_constraints {
        if let Some(cap) = squote.captures(c) {
            let phrase = &cap[1];
            let pat = format!("(?i){}", regex::escape(phrase));
            if plain(&pat).is_match(draft) {
                defects.push(format!("G-D/P8: banned phrase present: {phrase}"));
            }
        }
    }

    // ── G-F: degradation checklist (mechanical subset) ──
    let wc = draft.split_whitespace().count();
    let (lo, hi) = pack.p1_task.target_words;
    if !((lo as f64) * 0.8 <= wc as f64 && wc as f64 <= (hi as f64) * 1.3) {
        advisories.push(format!("G-F: word count {wc} vs target [{lo},{hi}]"));
    }
    use unicode_normalization::UnicodeNormalization;
    if draft.nfkd().any(|ch| ch as u32 > 0x2500) {
        advisories.push("G-F: unusual Unicode blocks present".to_string());
    }

    let pass = defects.is_empty();
    GauntletReport {
        defects,
        advisories,
        pass,
    }
}
