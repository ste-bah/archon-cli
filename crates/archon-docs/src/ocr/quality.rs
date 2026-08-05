//! S8: engine-agnostic OCR quality scoring and the second-engine arbiter policy.
//!
//! An OCR engine that *succeeds* can still return shredded text (the Kassel Greek
//! scan produced `ἡητορική` — rho read as eta — from surya; tesseract on noisy
//! images emits vowel-less consonant runs). The cascade previously escalated only
//! on hard failure, so this damage flowed straight into chunks and quotes. Every
//! successful OCR result is now scored here; a score below [`quality_floor`]
//! triggers the second engine, and the higher-scoring result wins.
//!
//! Scoring is deterministic and dependency-free: structural signals (mixed-script
//! tokens, vowel-less runs, replacement chars, noise chars, shredded single-char
//! tokens) plus a tiny embedded English/Greek stopword lexicon. It must stay
//! cheap — it runs inline on every ingest, per image and per page.

use cozo::DbInstance;

use crate::models::ChunkArtifact;

/// Structural quality of one OCR text, in `[0.0, 1.0]`.
#[derive(Clone, Debug)]
pub struct OcrQuality {
    pub score: f32,
    pub word_tokens: usize,
    /// Penalised signals, for status-row detail. Empty for clean text.
    pub reasons: Vec<String>,
}

/// Score below which a successful OCR result is treated as low quality:
/// the arbiter escalates to the second engine, and quality-scan rows are
/// written as `suspect`. Override with `ARCHON_OCR_QUALITY_FLOOR`.
pub fn quality_floor() -> f32 {
    std::env::var("ARCHON_OCR_QUALITY_FLOOR")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| (0.0..=1.0).contains(v))
        .unwrap_or(0.55)
}

/// Second-engine escalation on low-quality (but successful) OCR. On by
/// default; `ARCHON_OCR_ARBITER=0|off` disables escalation while keeping
/// scoring and status recording.
pub fn arbiter_enabled() -> bool {
    std::env::var("ARCHON_OCR_ARBITER")
        .map(|v| {
            let v = v.trim();
            !(v == "0" || v.eq_ignore_ascii_case("off"))
        })
        .unwrap_or(true)
}

const ENGLISH_STOPWORDS: &[&str] = &[
    "the", "and", "of", "to", "in", "is", "that", "it", "for", "as", "with", "was", "on", "are",
    "by", "be", "this", "not", "or", "from", "at", "an", "but", "which", "have", "has", "were",
    "their", "one", "all", "we", "can", "its", "they", "been", "if", "more", "when", "who", "will",
];

/// Monotonic and polytonic forms of the highest-frequency Greek function words —
/// enough to tell real Greek prose from shredded Greek OCR.
const GREEK_STOPWORDS: &[&str] = &[
    "καί",
    "καὶ",
    "και",
    "δέ",
    "δὲ",
    "δε",
    "τό",
    "τὸ",
    "το",
    "τά",
    "τὰ",
    "τα",
    "τοῦ",
    "του",
    "τῶν",
    "των",
    "τῆς",
    "της",
    "τήν",
    "τὴν",
    "την",
    "τῷ",
    "τῇ",
    "γάρ",
    "γὰρ",
    "γαρ",
    "μέν",
    "μὲν",
    "μεν",
    "οὐ",
    "οὐκ",
    "ου",
    "ουκ",
    "μή",
    "μὴ",
    "μη",
    "ἐν",
    "εν",
    "εἰς",
    "εις",
    "ὡς",
    "ως",
    "ὅτι",
    "οτι",
    "περί",
    "περὶ",
    "περι",
    "πρός",
    "πρὸς",
    "προς",
    "ἐστι",
    "ἐστιν",
    "εἶναι",
    "τε",
    "ἤ",
    "ἢ",
    "ὁ",
    "ἡ",
    "οἱ",
    "αἱ",
    "τις",
    "τι",
    "ἀλλά",
    "ἀλλὰ",
    "αλλα",
    "οὖν",
    "ουν",
];

fn is_greek_letter(c: char) -> bool {
    matches!(c, '\u{0370}'..='\u{03FF}' | '\u{1F00}'..='\u{1FFF}')
}

fn is_vowel(c: char) -> bool {
    // Latin vowels, Greek base vowels (both cases), monotonic accented vowels,
    // and the polytonic block — every char in U+1F00–U+1FFF except ῤ/ῥ/Ῥ carries
    // a vowel, and treating those three leniently only makes the heuristic safer.
    matches!(
        c,
        'a' | 'e'
            | 'i'
            | 'o'
            | 'u'
            | 'y'
            | 'A'
            | 'E'
            | 'I'
            | 'O'
            | 'U'
            | 'Y'
            | 'α'
            | 'ε'
            | 'η'
            | 'ι'
            | 'ο'
            | 'υ'
            | 'ω'
            | 'Α'
            | 'Ε'
            | 'Η'
            | 'Ι'
            | 'Ο'
            | 'Υ'
            | 'Ω'
            | 'ά'
            | 'έ'
            | 'ή'
            | 'ί'
            | 'ό'
            | 'ύ'
            | 'ώ'
            | 'ϊ'
            | 'ϋ'
            | 'ΐ'
            | 'ΰ'
            | '\u{1F00}'..='\u{1FFF}'
    )
}

fn is_common_punct(c: char) -> bool {
    matches!(
        c,
        '.' | ','
            | ';'
            | ':'
            | '!'
            | '?'
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '"'
            | '\''
            | '«'
            | '»'
            | '·'
            | '—'
            | '–'
            | '-'
            | '\u{2010}'
            | '\u{2011}'
            | '/'
            | '…'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '&'
            | '%'
            | '§'
            | '*'
            | '†'
            | '\u{037E}'
            | '\u{0384}'
            | '\u{0387}'
            | '\u{00AD}'
    )
}

/// Score one OCR text. Empty/whitespace input scores 0.0.
pub fn score_text(text: &str) -> OcrQuality {
    if text.trim().is_empty() {
        return OcrQuality {
            score: 0.0,
            word_tokens: 0,
            reasons: vec!["empty".into()],
        };
    }

    let mut total_chars = 0usize;
    let mut noise_chars = 0usize;
    let mut replacement_chars = 0usize;
    for c in text.chars() {
        total_chars += 1;
        if c == '\u{FFFD}' {
            replacement_chars += 1;
        } else if !c.is_alphanumeric() && !c.is_whitespace() && !is_common_punct(c) {
            noise_chars += 1;
        }
    }

    let mut word_tokens = 0usize;
    let mut mixed_script = 0usize;
    let mut vowelless = 0usize;
    let mut single_char = 0usize;
    let mut stop_hits = 0usize;
    for raw in text.split_whitespace() {
        let token = raw.trim_matches(|c: char| !c.is_alphanumeric());
        if token.is_empty() || !token.chars().any(|c| c.is_alphabetic()) {
            continue;
        }
        word_tokens += 1;
        let has_latin = token.chars().any(|c| c.is_ascii_alphabetic());
        let has_greek = token.chars().any(is_greek_letter);
        if has_latin && has_greek {
            mixed_script += 1;
        }
        let len = token.chars().count();
        if len == 1 {
            single_char += 1;
        }
        if len >= 4 && !token.chars().any(is_vowel) {
            vowelless += 1;
        }
        let lower = token.to_lowercase();
        if ENGLISH_STOPWORDS.contains(&lower.as_str()) || GREEK_STOPWORDS.contains(&lower.as_str())
        {
            stop_hits += 1;
        }
    }

    let wt = word_tokens.max(1) as f32;
    let tc = total_chars.max(1) as f32;
    let mut score = 1.0f32;
    let mut reasons: Vec<String> = Vec::new();

    let mixed_ratio = mixed_script as f32 / wt;
    if mixed_ratio > 0.02 {
        score -= (mixed_ratio * 3.0).min(0.5);
        reasons.push(format!("mixed-script {:.0}%", mixed_ratio * 100.0));
    }
    let vowelless_ratio = vowelless as f32 / wt;
    if vowelless_ratio > 0.05 {
        score -= ((vowelless_ratio - 0.05) * 2.0).min(0.4);
        reasons.push(format!("vowelless {:.0}%", vowelless_ratio * 100.0));
    }
    let single_ratio = single_char as f32 / wt;
    if single_ratio > 0.15 {
        score -= (single_ratio - 0.15).min(0.3);
        reasons.push(format!("single-char {:.0}%", single_ratio * 100.0));
    }
    let replacement_ratio = replacement_chars as f32 / tc;
    if replacement_ratio > 0.0 {
        score -= (replacement_ratio * 20.0).min(0.4);
        reasons.push(format!("U+FFFD {replacement_chars}"));
    }
    let noise_ratio = noise_chars as f32 / tc;
    if noise_ratio > 0.05 {
        score -= ((noise_ratio - 0.05) * 4.0).min(0.4);
        reasons.push(format!("noise {:.0}%", noise_ratio * 100.0));
    }
    // Stopword evidence only carries weight once the text is long enough that a
    // clean passage would certainly contain function words.
    if word_tokens >= 20 {
        let hit_ratio = stop_hits as f32 / wt;
        if stop_hits == 0 {
            score -= 0.25;
            reasons.push("no stopwords".into());
        } else if hit_ratio >= 0.05 {
            score = (score + 0.10).min(1.0);
        }
    }

    OcrQuality {
        score: score.clamp(0.0, 1.0),
        word_tokens,
        reasons,
    }
}

/// S8 page-level quality scan — runs inline on every PDF ingest, after chunk
/// persistence, for BOTH chunkers (marker blocks and text-offset chunks). Pages
/// scoring below [`quality_floor`] get a durable `suspect` row in
/// `doc_image_ocr_status` (probes and gates read that relation), and the ingest
/// outcome gets one summary warning. Write failures degrade to warnings — the
/// scan must never fail an otherwise-good ingest.
pub(crate) fn scan_document_pages(
    db: &DbInstance,
    document_id: &str,
    chunks: &[ChunkArtifact],
    engine: &str,
    warnings: &mut Vec<String>,
) {
    use std::collections::BTreeMap;
    let mut by_page: BTreeMap<u32, String> = BTreeMap::new();
    for chunk in chunks {
        let entry = by_page.entry(chunk.page_start).or_default();
        if !entry.is_empty() {
            entry.push('\n');
        }
        entry.push_str(&chunk.content);
    }
    if by_page.is_empty() {
        return;
    }

    let floor = quality_floor();
    let created_at = chrono::Utc::now().to_rfc3339();
    let mut rows: Vec<cozo::DataValue> = Vec::new();
    let total_pages = by_page.len();
    for (page, text) in &by_page {
        let q = score_text(text);
        if q.score >= floor {
            continue;
        }
        let detail = format!(
            "engine={engine} score={:.2} {}",
            q.score,
            q.reasons.join(", ")
        );
        rows.push(cozo::DataValue::List(vec![
            cozo::DataValue::from(format!("{document_id}-p{page}-quality").as_str()),
            cozo::DataValue::from(document_id),
            cozo::DataValue::from(*page as i64),
            cozo::DataValue::from("suspect"),
            cozo::DataValue::from(&detail[..detail.len().min(400)]),
            cozo::DataValue::from(created_at.as_str()),
        ]));
    }
    if rows.is_empty() {
        return;
    }

    let suspects = rows.len();
    let mut params = std::collections::BTreeMap::new();
    params.insert("rows".to_string(), cozo::DataValue::List(rows));
    if let Err(e) = crate::cozo_retry::run_script_guarded(
        db,
        "?[status_id, document_id, page_number, status, detail, created_at] <- $rows \
         :put doc_image_ocr_status { status_id => document_id, page_number, status, detail, created_at }",
        params,
        cozo::ScriptMutability::Mutable,
        "put doc_image_ocr_status (quality scan)",
    ) {
        warnings.push(format!("OCR quality-scan rows failed to write: {e}"));
    }
    warnings.push(format!(
        "OCR quality scan: {suspects}/{total_pages} pages below floor {floor:.2} (engine={engine})"
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_english_scores_high() {
        let text = "The rhetoric of the survey is that attention in virtual environments \
                    can be measured, and that the measures converge when the instruments \
                    are aligned with what the participants actually experienced during the \
                    session. This is a plain English paragraph with ordinary structure.";
        let q = score_text(text);
        assert!(
            q.score >= 0.8,
            "clean English scored {:.2}: {:?}",
            q.score,
            q.reasons
        );
    }

    #[test]
    fn clean_greek_scores_high() {
        let text = "ἡ ῥητορική ἐστιν ἀντίστροφος τῇ διαλεκτικῇ· ἀμφότεραι γὰρ περὶ τοιούτων \
                    τινῶν εἰσιν ἃ κοινὰ τρόπον τινὰ ἁπάντων ἐστὶ γνωρίζειν καὶ οὐδεμιᾶς \
                    ἐπιστήμης ἀφωρισμένης· διὸ καὶ πάντες τρόπον τινὰ μετέχουσιν ἀμφοῖν.";
        let q = score_text(text);
        assert!(
            q.score >= 0.7,
            "clean Greek scored {:.2}: {:?}",
            q.score,
            q.reasons
        );
    }

    #[test]
    fn shredded_ocr_scores_low() {
        let text = "x7$ g@ qw&# zzkr% vbn~ ju_ pl| mn^ #tr wq= lk> hj< bnm@ x$ z% q# w& \
                    r* t( y) u_ i+ o= p[ a] s; d' f\" g< h> j? k/ l\\ z| xcvb qwrt zxns";
        let q = score_text(text);
        assert!(
            q.score <= 0.45,
            "garbage scored {:.2}: {:?}",
            q.score,
            q.reasons
        );
    }

    #[test]
    fn mixed_script_corruption_scores_below_clean() {
        // rho→p / eta→n class of surya artifacts: Latin letters inside Greek words.
        let corrupt = "ἡ pητοpική ἐστιν ἀντίστpοφος τῇ διαλεκτικῇ ἀμφότεpαι γὰp πεpὶ \
                       τοιούτων τινῶν εἰσιν ἃ κοινὰ τpόπον τινὰ ἁπάντων ἐστὶ γνωpίζειν";
        let clean = "ἡ ῥητορική ἐστιν ἀντίστροφος τῇ διαλεκτικῇ ἀμφότεραι γὰρ περὶ \
                     τοιούτων τινῶν εἰσιν ἃ κοινὰ τρόπον τινὰ ἁπάντων ἐστὶ γνωρίζειν";
        let qc = score_text(corrupt);
        let qk = score_text(clean);
        assert!(
            qc.score + 0.1 < qk.score,
            "corrupt {:.2} should score clearly below clean {:.2}",
            qc.score,
            qk.score
        );
    }

    #[test]
    fn empty_text_scores_zero() {
        assert_eq!(score_text("   \n ").score, 0.0);
    }

    #[test]
    fn replacement_chars_are_penalised() {
        let text = "the qui\u{FFFD}k bro\u{FFFD}n fox jum\u{FFFD}s over the la\u{FFFD}y dog \
                    and the te\u{FFFD}t is damaged in a way that is visible to the scorer";
        let q = score_text(text);
        assert!(q.reasons.iter().any(|r| r.contains("U+FFFD")));
        assert!(q.score < score_text(&text.replace('\u{FFFD}', "z")).score + 0.01);
    }
}
