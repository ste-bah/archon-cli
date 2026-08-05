//! Layout port (S-3) — running-head locator capture (Bekker numbers / page numbers).
//!
//! Inverts `layout_analyzer.py`'s strip-and-discard (`_PAGE_NUM_RE:53`, `_BEKKER_RE:54`):
//! a block whose entire text is a standalone page number or Bekker number is **captured
//! as a citation locator** and removed from the body stream (strip + capture). Bekker
//! numbers become first-class anchors the verbatim subsystem can resolve.
//!
//! Deviation from the reference (documented, citation-correctness): the reference
//! `_BEKKER_RE = \d{2,3}[a-b]?\d{0,2}` caps at 3 digits and makes the column letter
//! optional, so it (a) misses 4-digit Aristotle Bekker numbers like `1147a` and
//! (b) conflates letterless numbers with page numbers. We require the column letter and
//! allow 1–4 leading digits + up to 3 line digits — matching real Bekker citations
//! (`184b15`, `1147a`, `1147a13`) and keeping Bekker distinct from page numbers.

use regex::Regex;

use crate::chunk::Block;

/// What kind of running-head locator was captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocatorKind {
    PageNumber,
    Bekker,
    /// A side-margin Bekker line-number (the every-5-lines marginalia) — NOT a page number.
    LineNumber,
}

/// A captured locator: its page, kind, normalized value, and bbox (sentinel on non-Marker paths).
#[derive(Clone, Debug, PartialEq)]
pub struct LocatorHit {
    pub page: u32,
    pub kind: LocatorKind,
    pub value: String,
    pub bbox: [f32; 4],
}

/// Plain running-head page number, optionally with an OCR'd trailing `o`/`O`
/// (faithful to `_PAGE_NUM_RE:53`).
fn page_num_re() -> Regex {
    Regex::new(r"^\s*\d{1,3}[oO]?\s*$").expect("static page-num regex")
}

/// Bekker number — broadened from the reference (see module docs): 1–4 digits, a REQUIRED
/// column letter `a`/`b` (or `*`, a superscript column-a that OCRs as an asterisk, e.g. `451*1`),
/// optional 1–3 line-number digits.
fn bekker_re() -> Regex {
    Regex::new(r"^\s*\d{1,4}[ab*][0-9liI|]{0,3}\s*$").expect("static bekker regex")
}

/// Inline Bekker markers WITHIN body text (Aristotle docs): 3–4 page digits, a glued column letter
/// (`a`/`b`, or `*`=a), optional 1–3 line "digits". The line digit `1` frequently OCRs as `l`/`i`/`|`
/// (e.g. `452bl`, `453bi`), so those are allowed and normalized. Boundaries are enforced by the
/// caller (no lookaround in the `regex` crate); the glued-letter requirement rejects "1984 by".
fn inline_bekker_re() -> Regex {
    Regex::new(r"\d{3,4}[ab*][0-9liI|]{0,3}").expect("static inline bekker regex")
}

/// Canonicalize an OCR'd Bekker marker. In the page-number region the superscript column letter
/// often OCRs as `*` (=column a), so `450*1` → `450a1`. In the line-number region (after the column
/// letter) the digit `1` OCRs as `l`/`i`/`I`/`|`, so `452bl` → `452b1`, `453bi` → `453b1`.
fn normalize_bekker(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut seen_col = false;
    for ch in raw.trim().chars() {
        if !seen_col {
            match ch {
                '*' | 'a' | 'A' => {
                    out.push('a');
                    seen_col = true;
                }
                'b' | 'B' => {
                    out.push('b');
                    seen_col = true;
                }
                d if d.is_ascii_digit() => out.push(d),
                _ => out.push(ch),
            }
        } else {
            match ch {
                'l' | 'i' | 'I' | '|' => out.push('1'),
                d if d.is_ascii_digit() => out.push(d),
                _ => {}
            }
        }
    }
    out
}

/// A standalone bare-number margin block on an Aristotle page is a Bekker LINE number (the
/// every-5-lines marginalia), not a page number: a multiple of 5, up to ~40 (a Bekker column holds
/// ~35 lines). A larger or non-multiple number is treated as a genuine page number.
fn is_bekker_line_number(t: &str) -> bool {
    t.trim()
        .parse::<u32>()
        .ok()
        .is_some_and(|n| n > 0 && n <= 40 && n % 5 == 0)
}

/// True when a bare inline Bekker-shaped token sits in author-date citation context rather than
/// being a real Bekker locator — e.g. `(Smith 2020a, 15)`, where `2020a` is a year plus a
/// disambiguating letter, not a Bekker number (H-3). Heuristic: the token is immediately preceded
/// (across one run of spaces) by a Capitalized word — an author surname. Genuine inline Bekker
/// markers in Aristotle body prose follow a lowercase function word ("on the 450a1 soul", "owing to
/// 452b passion"), so this rejects the author-date false positive without dropping real anchors.
/// A parenthetical bare Bekker (`(450a1)`, no author word) is NOT in author-date context and is kept.
fn is_author_date_context(text: &str, match_start: usize) -> bool {
    let prefix = &text[..match_start];
    let trimmed = prefix.trim_end_matches([' ', '\t', '\u{00A0}']);
    // No whitespace gap before the token → not an "Author YEAR" pattern (the caller's ASCII-alnum
    // boundary check already governs a glued predecessor).
    if trimmed.len() == prefix.len() {
        return false;
    }
    // The word immediately before the space run (letters only).
    let preceding_word: String = {
        let rev: String = trimmed
            .chars()
            .rev()
            .take_while(|c| c.is_alphabetic())
            .collect();
        rev.chars().rev().collect()
    };
    let mut cs = preceding_word.chars();
    match cs.next() {
        // Capitalized surname-shaped word (an initial upper-case letter followed by lower case),
        // e.g. "Smith". A single capital ("A") or an all-caps token is not treated as an author.
        Some(first) => first.is_uppercase() && cs.any(|c| c.is_lowercase()),
        None => false,
    }
}

/// Capture inline Bekker markers in `text`, enforcing token boundaries manually (ASCII-alnum on
/// either side rejects a marker glued into a longer word/number). A match in author-date context
/// (`(Smith 2020a, 15)`) is rejected — a modern year + letter is not a Bekker locator (H-3).
fn push_inline_bekker(text: &str, page: u32, bbox: [f32; 4], out: &mut Vec<LocatorHit>) {
    let bytes = text.as_bytes();
    for m in inline_bekker_re().find_iter(text) {
        let (s, e) = (m.start(), m.end());
        let before_ok = s == 0 || !bytes[s - 1].is_ascii_alphanumeric();
        let after_ok = e >= bytes.len() || !bytes[e].is_ascii_alphanumeric();
        if before_ok && after_ok && !is_author_date_context(text, s) {
            out.push(LocatorHit {
                page,
                kind: LocatorKind::Bekker,
                value: normalize_bekker(m.as_str()),
                bbox,
            });
        }
    }
}

/// Split a block stream into (body blocks kept, locator hits removed). A block is pulled out as a
/// locator when its ENTIRE trimmed text is a standalone Bekker or page number; everything else
/// passes through unchanged. Bekker is checked first (more specific).
///
/// `is_aristotle` (filename gated by the caller): on Aristotle primary texts, (a) a standalone
/// bare-number margin block is a Bekker LINE number, not a page number; and (b) inline Bekker
/// markers within body prose (e.g. "On the 450*1 Soul") are ALSO captured (the block stays in the
/// body). Non-Aristotle docs are untouched — byte-identical to the previous behavior.
pub fn extract_locators(blocks: &[Block], is_aristotle: bool) -> (Vec<Block>, Vec<LocatorHit>) {
    let page_re = page_num_re();
    let bekker_re = bekker_re();
    let mut kept = Vec::with_capacity(blocks.len());
    let mut hits = Vec::new();
    for b in blocks {
        let t = b.text.trim();
        if bekker_re.is_match(t) {
            hits.push(LocatorHit {
                page: b.page,
                kind: LocatorKind::Bekker,
                value: normalize_bekker(t),
                bbox: b.bbox,
            });
        } else if page_re.is_match(t) {
            let kind = if is_aristotle && is_bekker_line_number(t) {
                LocatorKind::LineNumber
            } else {
                LocatorKind::PageNumber
            };
            hits.push(LocatorHit {
                page: b.page,
                kind,
                value: t.to_string(),
                bbox: b.bbox,
            });
        } else {
            if is_aristotle {
                push_inline_bekker(&b.text, b.page, b.bbox, &mut hits);
            }
            kept.push(b.clone());
        }
    }
    (kept, hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::BlockType;

    fn blk(text: &str, page: u32) -> Block {
        Block {
            block_type: BlockType::Text,
            text: text.to_string(),
            bbox: [1.0, 2.0, 3.0, 4.0],
            page,
        }
    }

    #[test]
    fn captures_bekker_and_page_numbers_and_strips_them() {
        let blocks = vec![
            blk("Real prose about energeia and entelecheia.", 1),
            blk("1147a", 1),  // 4-digit Bekker (reference would MISS this)
            blk("184b15", 2), // Bekker with line number
            blk("47", 2),     // plain page number
            blk("More body text here.", 3),
            blk("see 1147a below", 3), // NOT standalone → kept
        ];
        let (kept, hits) = extract_locators(&blocks, false);
        assert_eq!(
            kept.len(),
            3,
            "two body paras + the inline-reference para remain"
        );
        assert!(kept.iter().any(|b| b.text.starts_with("see 1147a")));

        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].kind, LocatorKind::Bekker);
        assert_eq!(hits[0].value, "1147a");
        assert_eq!(hits[0].page, 1);
        assert_eq!(hits[0].bbox, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(hits[1].kind, LocatorKind::Bekker);
        assert_eq!(hits[1].value, "184b15");
        assert_eq!(hits[2].kind, LocatorKind::PageNumber);
        assert_eq!(hits[2].value, "47");
    }

    #[test]
    fn letterless_number_is_a_page_number_not_bekker() {
        let (_, hits) = extract_locators(&[blk("123", 5)], false);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].kind,
            LocatorKind::PageNumber,
            "no column letter → page number"
        );
    }

    #[test]
    fn empty_when_no_standalone_numbers() {
        let blocks = vec![blk("ordinary paragraph", 1), blk("another one", 1)];
        let (kept, hits) = extract_locators(&blocks, false);
        assert_eq!(kept.len(), 2);
        assert!(hits.is_empty());
    }

    #[test]
    fn aristotle_reclassifies_margin_line_numbers() {
        // Multiples of 5 in Aristotle margins are Bekker line-numbers, not page numbers.
        let (_, hits) = extract_locators(&[blk("20", 3), blk("25", 3)], true);
        assert!(
            hits.iter().all(|h| h.kind == LocatorKind::LineNumber),
            "{hits:?}"
        );
        // Non-Aristotle: still page numbers.
        let (_, h2) = extract_locators(&[blk("20", 3)], false);
        assert_eq!(h2[0].kind, LocatorKind::PageNumber);
        // A non-multiple-of-5 (a genuine page number) stays a page number even on Aristotle.
        let (_, h3) = extract_locators(&[blk("142", 3)], true);
        assert_eq!(h3[0].kind, LocatorKind::PageNumber);
    }

    #[test]
    fn standalone_star_bekker_is_normalized() {
        let (_, hits) = extract_locators(&[blk("451*1", 5)], true);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, LocatorKind::Bekker);
        assert_eq!(hits[0].value, "451a1", "* normalizes to a");
    }

    #[test]
    fn inline_bekker_captured_for_aristotle_only() {
        let b = blk(
            "On the 450*1 Soul, phantasia moved owing to 452b passion.",
            4,
        );
        let (kept, hits) = extract_locators(std::slice::from_ref(&b), true);
        assert_eq!(kept.len(), 1, "the body block stays");
        let vals: Vec<&str> = hits.iter().map(|h| h.value.as_str()).collect();
        assert!(vals.contains(&"450a1"), "inline 450*1 → 450a1: {vals:?}");
        assert!(vals.contains(&"452b"), "inline 452b captured: {vals:?}");
        // Non-Aristotle: inline markers are left in the body, not captured.
        let (_, none) = extract_locators(std::slice::from_ref(&b), false);
        assert!(
            none.is_empty(),
            "non-Aristotle inline scan is off: {none:?}"
        );
    }

    #[test]
    fn year_by_is_not_a_bekker_false_positive() {
        // "…© 1984 by The Jowett…" must NOT yield a Bekker locus (a space, not a glued letter).
        let (_, hits) = extract_locators(
            &[blk("Copyright 1984 by The Jowett Copyright Trust.", 2)],
            true,
        );
        assert!(hits.is_empty(), "no Bekker from '1984 by': {hits:?}");
    }

    #[test]
    fn glued_author_date_is_not_a_bekker_false_positive() {
        // "(Smith 2020a, 15)" — "2020a" is a year + disambiguating letter in an author-date cite,
        // NOT a Bekker locator (H-3). A capitalized author surname immediately precedes it.
        let (_, hits) = extract_locators(
            &[blk("see also (Smith 2020a, 15) for a rebuttal.", 6)],
            true,
        );
        assert!(hits.is_empty(), "no Bekker from '(Smith 2020a': {hits:?}");
        // Inline with a lowercase function word before it is still a genuine Bekker anchor.
        let (_, kept) = extract_locators(&[blk("as argued at 452b in the text", 6)], true);
        assert!(
            kept.iter().any(|h| h.value == "452b"),
            "lowercase-preceded 452b stays: {kept:?}"
        );
        // A bare parenthetical Bekker (no author word) is kept — not author-date context.
        let (_, paren) = extract_locators(&[blk("the passage (450a1) shows", 6)], true);
        assert!(
            paren.iter().any(|h| h.value == "450a1"),
            "bare parenthetical Bekker kept: {paren:?}"
        );
    }

    #[test]
    fn ocr_confused_line_digits_are_normalized() {
        // The Bekker line-number '1' frequently OCRs as 'l'/'i': 452bl → 452b1, 453bi → 453b1.
        let (_, s) = extract_locators(&[blk("452bl", 8)], true);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].value, "452b1");
        let (_, i) = extract_locators(&[blk("recollection 453bi abnormally weak memory", 9)], true);
        assert!(
            i.iter().any(|h| h.value == "453b1"),
            "inline 453bi → 453b1: {i:?}"
        );
    }
}
