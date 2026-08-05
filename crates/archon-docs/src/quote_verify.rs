//! V-3/V-4 — quote verification. Locate a (dissertation) quote in the corpus and return where it
//! is — document, page(s), and per-fragment bboxes — so a citation can be verified against the
//! source and a PDF highlight drawn. Catches misquotes/hallucinated quotes: an exact match confirms
//! the quote verbatim; a fuzzy match surfaces near-misses (OCR noise, transcription drift) with a
//! similarity score; no match means it is not in the corpus.
//!
//! Matching is whitespace/punctuation-normalized (smart quotes, dashes, soft hyphens, collapsed
//! whitespace) with an offset map back to the ORIGINAL text, so the returned span is verbatim source
//! and fragments attribute to the exact chunks a quote crosses. Fuzzy matching uses approximate
//! substring alignment (Sellers) so an OCR/transcription-drifted quote still resolves with a score.

use cozo::DbInstance;

use crate::errors::DocsError;
use crate::models::ChunkArtifact;
use crate::retrieval::{self, SearchMode};
use crate::store;

/// Below this similarity a document is not reported as a location at all.
const REPORT_FLOOR: f64 = 0.60;
/// How many FTS candidate documents to reconstruct + match against.
const MAX_CANDIDATE_DOCS: usize = 8;

/// Whether the located text matched the quote verbatim (normalized) or approximately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchKind {
    Exact,
    Fuzzy,
}

/// One chunk the quote overlaps, with its page + bbox (for a PDF highlight).
#[derive(Clone, Debug)]
pub struct QuoteFragment {
    pub chunk_id: String,
    pub page: u32,
    /// Marker `super_box` `[x0,y0,x1,y1]`; `None` on the pdftotext path (no real bbox).
    pub bbox: Option<[f32; 4]>,
    pub coord_space: String,
}

/// Where a quote was located in one document.
#[derive(Clone, Debug)]
pub struct QuoteLocation {
    pub document_id: String,
    pub source_path: String,
    pub page_start: u32,
    pub page_end: u32,
    /// 1.0 = verbatim (normalized) substring; `<1.0` = approximate.
    pub similarity: f64,
    pub match_kind: MatchKind,
    /// The verbatim SOURCE text over the matched span — compare against the quote to spot misquotes.
    pub source_span: String,
    /// The chunks the match crosses, each with page + bbox.
    pub fragments: Vec<QuoteFragment>,
}

/// Locate a quote across the corpus. FTS-narrows to candidate documents, reconstructs each and
/// matches (exact then fuzzy), and returns the locations ranked best-first (exact before fuzzy,
/// higher similarity first). Empty when nothing reaches [`REPORT_FLOOR`].
pub fn locate_quote(
    db: &DbInstance,
    quote: &str,
    max_results: usize,
) -> Result<Vec<QuoteLocation>, DocsError> {
    let mut doc_ids = candidate_documents(db, quote)?;
    doc_ids.truncate(MAX_CANDIDATE_DOCS);

    let mut locations: Vec<QuoteLocation> = Vec::new();
    for doc_id in doc_ids {
        if let Some(loc) = find_fragment_bboxes(db, &doc_id, quote)? {
            locations.push(loc);
        }
    }
    // Exact before fuzzy; then higher similarity; then earlier pages (stable, deterministic).
    locations.sort_by(|a, b| {
        (a.match_kind == MatchKind::Fuzzy)
            .cmp(&(b.match_kind == MatchKind::Fuzzy))
            .then(
                b.similarity
                    .partial_cmp(&a.similarity)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(a.page_start.cmp(&b.page_start))
    });
    locations.truncate(max_results);
    Ok(locations)
}

/// FTS candidate documents for the quote (distinct, in relevance order). Falls back to hybrid search
/// when exact FTS finds nothing (a heavily drifted quote may only surface semantically).
fn candidate_documents(db: &DbInstance, quote: &str) -> Result<Vec<String>, DocsError> {
    let mut ids = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |results: &[retrieval::SearchResult], ids: &mut Vec<String>| {
        for r in results {
            if seen.insert(r.document_id.clone()) {
                ids.push(r.document_id.clone());
            }
        }
    };
    let exact = retrieval::search_with_mode(
        db,
        quote,
        24,
        SearchMode::Exact,
        retrieval::RetrievalWeights::default(),
    )?;
    push(&exact.results, &mut ids);
    if ids.is_empty() {
        let hybrid = retrieval::search(db, quote, 24)?;
        push(&hybrid.results, &mut ids);
    }
    Ok(ids)
}

/// Within one document, find the fragments (chunks + bboxes) whose reconstructed text contains the
/// quote verbatim (normalized), or best-approximates it. Returns `None` below [`REPORT_FLOOR`].
pub fn find_fragment_bboxes(
    db: &DbInstance,
    document_id: &str,
    quote: &str,
) -> Result<Option<QuoteLocation>, DocsError> {
    let mut chunks = store::list_chunks_for_doc(db, document_id).map_err(storage)?;
    if chunks.is_empty() {
        return Ok(None);
    }
    // Reading order: page, then chunk index within the page.
    chunks.sort_by(|a, b| {
        a.page_start
            .cmp(&b.page_start)
            .then(a.chunk_index.cmp(&b.chunk_index))
    });
    let doc = Reconstructed::build(&chunks);
    let q = normalize(quote).chars;
    if q.is_empty() {
        return Ok(None);
    }

    let source_path = store::get_doc_source(db, document_id)
        .ok()
        .flatten()
        .map(|d| d.source_path)
        .unwrap_or_default();

    // Exact (normalized) substring across the concatenated chunks.
    if let Some(mstart) = find_subslice(&doc.norm.chars, &q) {
        let (a, b) = (
            doc.norm.orig_byte[mstart],
            doc.norm.orig_byte[mstart + q.len()],
        );
        let hit_chunks = doc.chunks_in_range(a, b);
        return Ok(Some(build_location(
            db,
            document_id,
            &source_path,
            &chunks,
            &hit_chunks,
            doc.original[a..b].to_string(),
            1.0,
            MatchKind::Exact,
        )));
    }

    // Fuzzy: best approximate-substring alignment within the single best chunk.
    let mut best: Option<(usize, f64)> = None; // (chunk index, similarity)
    for (i, chunk) in chunks.iter().enumerate() {
        let cn = normalize(&chunk.content).chars;
        if cn.is_empty() {
            continue;
        }
        let sim = approx_substring_similarity(&q, &cn);
        if best.map(|(_, s)| sim > s).unwrap_or(true) {
            best = Some((i, sim));
        }
    }
    if let Some((i, sim)) = best
        && sim >= REPORT_FLOOR
    {
        let hit = &chunks[i];
        return Ok(Some(build_location(
            db,
            document_id,
            &source_path,
            &chunks,
            &[i],
            hit.content.clone(),
            sim,
            MatchKind::Fuzzy,
        )));
    }
    Ok(None)
}

#[allow(clippy::too_many_arguments)]
fn build_location(
    db: &DbInstance,
    document_id: &str,
    source_path: &str,
    all_chunks: &[ChunkArtifact],
    hit_indices: &[usize],
    source_span: String,
    similarity: f64,
    match_kind: MatchKind,
) -> QuoteLocation {
    let fragments: Vec<QuoteFragment> = hit_indices
        .iter()
        .map(|&i| fragment_for(db, &all_chunks[i]))
        .collect();
    let page_start = hit_indices
        .iter()
        .map(|&i| all_chunks[i].page_start)
        .min()
        .unwrap_or(0);
    let page_end = hit_indices
        .iter()
        .map(|&i| all_chunks[i].page_end)
        .max()
        .unwrap_or(0);
    QuoteLocation {
        document_id: document_id.to_string(),
        source_path: source_path.to_string(),
        page_start,
        page_end,
        similarity,
        match_kind,
        source_span,
        fragments,
    }
}

/// Build a fragment (page + bbox) for a chunk from its `doc_chunk_spatial` row.
fn fragment_for(db: &DbInstance, chunk: &ChunkArtifact) -> QuoteFragment {
    let spatial = store::get_chunk_spatial(db, &chunk.chunk_id).ok().flatten();
    let (page, bbox, coord_space) = match spatial {
        Some(s) => (
            s.page_num,
            // Both bbox-carrying coord spaces are trustworthy (PDF points, top-left origin);
            // "pdf-native" comes from the born-digital glyph-position extractor.
            parse_bbox(&s.super_box)
                .filter(|_| s.coord_space == "marker" || s.coord_space == "pdf-native"),
            s.coord_space,
        ),
        None => (chunk.page_start, None, "none".to_string()),
    };
    QuoteFragment {
        chunk_id: chunk.chunk_id.clone(),
        page,
        bbox,
        coord_space,
    }
}

fn parse_bbox(s: &str) -> Option<[f32; 4]> {
    let v: Vec<f32> = serde_json::from_str(s).ok()?;
    if v.len() == 4 {
        Some([v[0], v[1], v[2], v[3]])
    } else {
        None
    }
}

fn storage(e: anyhow::Error) -> DocsError {
    DocsError::Storage {
        message: e.to_string(),
    }
}

// ---- normalization + reconstruction --------------------------------------------------------------

/// A normalized string plus a map from each normalized char index to its ORIGINAL byte offset.
/// `orig_byte` has `chars.len() + 1` entries (the last is the original length sentinel), so a match
/// over `chars[i..j]` maps to the original byte span `orig_byte[i]..orig_byte[j]`.
struct Normalized {
    chars: Vec<char>,
    orig_byte: Vec<usize>,
}

/// Fold a char for matching: smart quotes/dashes → ascii, soft hyphen dropped, whitespace unified,
/// everything lowercased. Returns `None` to DROP the char (soft hyphen), `Some(' ')` for whitespace.
fn fold_char(c: char) -> Option<char> {
    match c {
        '\u{00AD}' | '\u{200B}' | '\u{FEFF}' => None, // soft hyphen / zero-width / BOM
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{2032}' | '`' => Some('\''),
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{2033}' => Some('"'),
        '\u{2013}' | '\u{2014}' | '\u{2212}' => Some('-'),
        '\u{00A0}' => Some(' '),
        c if c.is_whitespace() => Some(' '),
        c => Some(c.to_ascii_lowercase()),
    }
}

/// Normalize a string, collapsing whitespace runs to one space and tracking original byte offsets.
fn normalize(s: &str) -> Normalized {
    let mut chars = Vec::new();
    let mut orig_byte = Vec::new();
    let mut prev_space = false;
    for (byte, c) in s.char_indices() {
        match fold_char(c) {
            None => continue,
            Some(' ') => {
                if prev_space || chars.is_empty() {
                    continue; // collapse runs; drop leading whitespace
                }
                chars.push(' ');
                orig_byte.push(byte);
                prev_space = true;
            }
            Some(f) => {
                chars.push(f);
                orig_byte.push(byte);
                prev_space = false;
            }
        }
    }
    // Drop a single trailing space so it never anchors a boundary.
    if chars.last() == Some(&' ') {
        chars.pop();
        orig_byte.pop();
    }
    orig_byte.push(s.len()); // sentinel
    Normalized { chars, orig_byte }
}

/// The document's chunks concatenated (original + normalized), with each chunk's ORIGINAL byte range
/// recorded so a matched byte range attributes back to the chunks it crosses.
struct Reconstructed {
    original: String,
    norm: Normalized,
    /// (orig_start, orig_end, chunk_index) in `original`, in reading order.
    chunk_ranges: Vec<(usize, usize, usize)>,
}

impl Reconstructed {
    fn build(chunks: &[ChunkArtifact]) -> Reconstructed {
        let mut original = String::new();
        let mut chunk_ranges = Vec::with_capacity(chunks.len());
        for (i, c) in chunks.iter().enumerate() {
            if i > 0 {
                original.push('\n'); // boundary → normalizes to a collapsible space
            }
            let start = original.len();
            original.push_str(&c.content);
            chunk_ranges.push((start, original.len(), i));
        }
        let norm = normalize(&original);
        Reconstructed {
            original,
            norm,
            chunk_ranges,
        }
    }

    /// Chunk indices whose original byte range overlaps `[a, b)`.
    fn chunks_in_range(&self, a: usize, b: usize) -> Vec<usize> {
        self.chunk_ranges
            .iter()
            .filter(|(s, e, _)| *s < b && a < *e)
            .map(|(_, _, i)| *i)
            .collect()
    }
}

/// First index where `needle` occurs in `haystack` (naive; needles are short quotes).
fn find_subslice(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Similarity of the BEST approximate-substring alignment of `pattern` within `text`, in `[0,1]`
/// (`1.0` = `pattern` occurs exactly). Sellers' algorithm: a Levenshtein DP whose first row is 0 so
/// the match may start anywhere; the answer is the min over the last DP row. O(|pattern|·|text|).
fn approx_substring_similarity(pattern: &[char], text: &[char]) -> f64 {
    let m = pattern.len();
    if m == 0 {
        return 0.0;
    }
    let mut prev: Vec<usize> = (0..=m).collect(); // column 0: delete all of pattern
    let mut best = m;
    for &tc in text {
        let mut cur = vec![0usize; m + 1]; // first row stays 0 (match starts anywhere)
        for i in 1..=m {
            let cost = if pattern[i - 1] == tc { 0 } else { 1 };
            cur[i] = (prev[i - 1] + cost)
                .min(prev[i] + 1) // deletion in text
                .min(cur[i - 1] + 1); // insertion
        }
        best = best.min(cur[m]);
        prev = cur;
    }
    1.0 - (best as f64 / m as f64)
}

#[cfg(test)]
#[path = "quote_verify_tests.rs"]
mod quote_verify_tests;
