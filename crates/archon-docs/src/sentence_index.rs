//! The sentence layer (index-overhaul S2+S3): `doc_chunk_sentences`.
//!
//! A durable, deterministic sentence segmentation of the store's text-of-record —
//! the one layer standing between the block/byte/bbox machinery and sentence-level
//! verbatim granularity. Every chunk's content is segmented into byte-span sentences;
//! each sentence carries its sha256, its page (derived once from
//! `doc_chunk_page_breaks`) and a tight bbox (derived once from the
//! byte-overlapping `doc_chunk_blocks` on that page; "" when the extraction path
//! has no boxes — the D-contract's degradation case).
//!
//! Determinism contract: identical `(content, segmenter version)` produces an
//! identical table. `DERIVATION_VERSION` bumps on any rule change.
//!
//! Ported from archon-cli (primary) `sentence_index.rs`.
//! Store calls re-homed to v3's modular `store/` layout.

use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::errors::DocsError;
use crate::store;

pub const DERIVATION_VERSION: &str = "sent-v1";

// ── schema ─────────────────────────────────────────────────────────────────────────────

/// Ensure the `doc_chunk_sentences` relation exists. Idempotent.
/// Called from `schema::ensure_doc_schema()`; defined here so ownership of the
/// sentence schema lives alongside the segmentation logic.
pub fn ensure_sentence_schema(db: &DbInstance) -> Result<(), DocsError> {
    match db.run_script(
        r#":create doc_chunk_sentences {
    chunk_id: String,
    sentence_idx: Int =>
    byte_start: Int,
    byte_end: Int,
    text_sha256: String,
    page: Int,
    bbox: String,
    derivation_version: String
}"#,
        Default::default(),
        ScriptMutability::Mutable,
    ) {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("already exists") || msg.contains("conflicts with an existing") {
                Ok(())
            } else {
                Err(DocsError::Storage {
                    message: format!("create doc_chunk_sentences: {msg}"),
                })
            }
        }
    }
}

// ── segmentation ───────────────────────────────────────────────────────────────────────

const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "st", "vs", "cf", "etc", "al", "fig", "figs", "eq", "no",
    "pp", "p", "vol", "vols", "ch", "chap", "sec", "ed", "eds", "trans", "approx", "dept", "univ",
    "inc", "jr", "sr", "resp", "ca", "esp", "repr", "rev",
];

fn is_greek(c: char) -> bool {
    matches!(c, '\u{0370}'..='\u{03FF}' | '\u{1F00}'..='\u{1FFF}')
}

/// Deterministic, dependency-free sentence segmentation. Returns byte spans into
/// `text` (trimmed of surrounding whitespace per sentence). Rules:
/// - terminals: `.` `!` `?` `…`, plus `;`/`\u{037E}` in Greek context (the Greek
///   question mark), each followed by optional closing quotes/brackets + whitespace;
/// - a blank line (paragraph break) always terminates (headings, list items);
/// - guards: decimals (3.14), single-letter initials (C. King), dotted tokens
///   (e.g., i.e., U.S.), a curated abbreviation list, and ellipsis followed by a
///   lowercase continuation.
pub fn segment_sentences(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    let mut prev_chars: Vec<char> = Vec::new(); // small trailing window for guards

    let mut iter = text.char_indices().peekable();
    while let Some((bi, c)) = iter.next() {
        if start.is_none() {
            if c.is_whitespace() {
                continue;
            }
            start = Some(bi);
            prev_chars.clear();
        }
        prev_chars.push(c);
        if prev_chars.len() > 24 {
            prev_chars.remove(0);
        }

        // paragraph break: newline followed by (whitespace-only run containing) another newline
        if c == '\n' {
            let mut saw_second_newline = false;
            while let Some(&(_, n)) = iter.peek() {
                if n == '\n' {
                    saw_second_newline = true;
                    iter.next();
                } else if n.is_whitespace() {
                    iter.next();
                } else {
                    break;
                }
            }
            if saw_second_newline && let Some(s) = start.take() {
                push_span(text, bytes, s, bi, &mut spans);
            }
            continue;
        }

        let greek_ctx = prev_chars.iter().any(|&p| is_greek(p));
        let terminal =
            matches!(c, '.' | '!' | '?' | '…') || (greek_ctx && (c == ';' || c == '\u{037E}'));
        if !terminal {
            continue;
        }
        if c == '.' && dot_is_guarded(&prev_chars, &mut iter.clone()) {
            continue;
        }
        // consume closing quotes/brackets after the terminal
        let mut end_bi = bi + c.len_utf8();
        while let Some(&(nbi, n)) = iter.peek() {
            if matches!(
                n,
                '"' | '\'' | '\u{201C}' | '\u{2019}' | '»' | ')' | ']' | '}'
            ) {
                end_bi = nbi + n.len_utf8();
                iter.next();
            } else {
                break;
            }
        }
        // require whitespace (or end) after the terminal run to split
        match iter.peek() {
            None => {
                if let Some(s) = start.take() {
                    push_span(text, bytes, s, end_bi, &mut spans);
                }
            }
            Some(&(_, n)) if n.is_whitespace() => {
                if let Some(s) = start.take() {
                    push_span(text, bytes, s, end_bi, &mut spans);
                }
            }
            _ => {} // mid-token dot (file.name) — keep going
        }
    }
    if let Some(s) = start {
        push_span(text, bytes, s, text.len(), &mut spans);
    }
    spans
}

/// A '.' that must NOT split: decimals, single-letter initials, dotted tokens, a
/// curated abbreviation list, and ellipsis continuing in lowercase.
fn dot_is_guarded(
    prev: &[char],
    lookahead: &mut std::iter::Peekable<std::str::CharIndices>,
) -> bool {
    let before: Vec<char> = prev.iter().rev().skip(1).cloned().collect(); // chars before the '.'
    // decimal: digit '.' digit
    if let Some(&(_, next)) = lookahead.peek()
        && next.is_ascii_digit()
        && before.first().is_some_and(|c| c.is_ascii_digit())
    {
        return true;
    }
    // trailing token before the dot (letters/dots only)
    let mut token = String::new();
    for &c in &before {
        if c.is_alphanumeric() || c == '.' {
            token.push(c);
        } else {
            break;
        }
    }
    let token: String = token.chars().rev().collect();
    if token.chars().count() == 1 && token.chars().all(|c| c.is_uppercase()) {
        return true; // single-letter initial: "C."
    }
    if token.contains('.') {
        return true; // dotted acronym: "e.g", "U.S", "i.e"
    }
    if ABBREVIATIONS.contains(&token.to_lowercase().as_str()) {
        // an abbreviation dot splits only when what FOLLOWS starts a new sentence
        // with an uppercase letter after whitespace AND the abbreviation is
        // sentence-final-looking; be conservative: never split on the list
        return true;
    }
    // ellipsis "..." continuing lowercase
    if before.first() == Some(&'.') {
        let mut la = lookahead.clone();
        while let Some(&(_, n)) = la.peek() {
            if n.is_whitespace() || n == '.' {
                la.next();
            } else {
                return n.is_lowercase();
            }
        }
    }
    false
}

fn push_span(text: &str, _bytes: &[u8], start: usize, end: usize, spans: &mut Vec<(usize, usize)>) {
    let slice = &text[start..end];
    let trimmed = slice.trim_end();
    if trimmed.is_empty() {
        return;
    }
    let end = start + trimmed.len();
    spans.push((start, end));
}

// ── build + store ──────────────────────────────────────────────────────────────────────

pub struct SentenceBuildStats {
    pub chunks: usize,
    pub sentences: usize,
    pub with_bbox: usize,
    pub with_page: usize,
}

/// Marker block types whose box is a FIGURE region (used for figure-box inheritance).
fn is_figure_block(block_type: &str) -> bool {
    matches!(
        block_type,
        "Figure" | "Picture" | "FigureGroup" | "Image" | "PictureGroup"
    )
}

/// Rebuild the sentence rows for one document — deterministic, and BATCHED: one
/// guarded delete + one guarded multi-row insert for the whole document (the
/// per-chunk version cycled the cross-process write lock ~150× per document and
/// turned the full-corpus build into hours).
///
/// Bbox derivation (in priority order):
/// 1. TIGHT: union of byte-overlapping TEXT blocks. Blocks are matched by byte
///    overlap alone; when the overlapping blocks span multiple pages (a sentence
///    crossing a page break, or off-by-one page attribution at chunk edges), the
///    union is restricted to the sentence's START page — this fixes the
///    page-attribution misses of the strict `block.page == page` rule.
/// 2. FIGURE-INHERITED: a sentence with no text-block overlap (image-OCR side-channel
///    text) inherits the union of FIGURE-type block boxes on its page, when the
///    document has any — coarser than line-tight, but a real drawable location.
/// 3. NONE: bbox = "" (the D-contract degradation case, counted).
pub fn rebuild_document(
    db: &DbInstance,
    document_id: &str,
) -> Result<SentenceBuildStats, DocsError> {
    ensure_sentence_schema(db)?;
    let chunks = store::list_chunks_for_doc(db, document_id).map_err(storage)?;
    let mut stats = SentenceBuildStats {
        chunks: 0,
        sentences: 0,
        with_bbox: 0,
        with_page: 0,
    };

    // Doc-wide batched reads: ONE query for all blocks and ONE for all page
    // breaks (grouped per chunk), instead of two guarded queries per chunk —
    // the per-chunk pattern cost ~0.6s/chunk in exclusive-lock round trips and
    // made rebuild time linear in chunk count (measured: 223-chunk doc = 130s).
    let mut all_breaks = store::list_page_breaks_for_doc(db, document_id).unwrap_or_default();
    let mut all_blocks = store::list_chunk_blocks_for_doc(db, document_id).unwrap_or_default();

    // Doc-wide page → union-of-figure-boxes map (for inheritance), gathered once.
    let mut figure_boxes: BTreeMap<u32, [f32; 4]> = BTreeMap::new();
    let mut per_chunk: Vec<(
        &crate::models::ChunkArtifact,
        Vec<crate::models::PageBreak>,
        Vec<crate::models::ChunkBlock>,
    )> = Vec::new();
    for chunk in &chunks {
        let breaks = all_breaks.remove(&chunk.chunk_id).unwrap_or_default();
        let blocks = all_blocks.remove(&chunk.chunk_id).unwrap_or_default();
        for bl in &blocks {
            if is_figure_block(&bl.block_type) {
                let e = figure_boxes
                    .entry(bl.page)
                    .or_insert([bl.x0, bl.y0, bl.x1, bl.y1]);
                *e = [
                    e[0].min(bl.x0),
                    e[1].min(bl.y0),
                    e[2].max(bl.x1),
                    e[3].max(bl.y1),
                ];
            }
        }
        per_chunk.push((chunk, breaks, blocks));
    }

    let mut rows: Vec<DataValue> = Vec::new();
    for (chunk, breaks, blocks) in &per_chunk {
        let spans = segment_sentences(&chunk.content);
        for (idx, (b0, b1)) in spans.iter().enumerate() {
            let page = page_at(breaks, *b0).unwrap_or(chunk.page_start);
            // 1. TIGHT: byte-overlap first; page restriction only to break multi-page ties.
            //    (doc_chunk_blocks' char_* columns hold bytes — bytes-to-bytes discipline.)
            let overlapping: Vec<&crate::models::ChunkBlock> = blocks
                .iter()
                .filter(|bl| {
                    !is_figure_block(&bl.block_type) && bl.char_start < *b1 && bl.char_end > *b0
                })
                .collect();
            let pages: std::collections::BTreeSet<u32> =
                overlapping.iter().map(|bl| bl.page).collect();
            let chosen_page = if pages.len() > 1 {
                pages.iter().next().copied()
            } else {
                None
            };
            let mut bb: Option<[f32; 4]> = None;
            for bl in &overlapping {
                if let Some(cp) = chosen_page {
                    // multi-page overlap: keep the sentence's start-side page only
                    if bl.page != cp {
                        continue;
                    }
                }
                bb = Some(match bb {
                    None => [bl.x0, bl.y0, bl.x1, bl.y1],
                    Some(u) => [
                        u[0].min(bl.x0),
                        u[1].min(bl.y0),
                        u[2].max(bl.x1),
                        u[3].max(bl.y1),
                    ],
                });
            }
            // 2. FIGURE-INHERITED for image-OCR side-channel text.
            if bb.is_none()
                && let Some(f) = figure_boxes.get(&page)
            {
                bb = Some(*f);
            }
            let bbox_s = bb
                .map(|b| format!("[{},{},{},{}]", b[0], b[1], b[2], b[3]))
                .unwrap_or_default();
            if bb.is_some() {
                stats.with_bbox += 1;
            }
            if page > 0 {
                stats.with_page += 1;
            }
            let text = &chunk.content[*b0..*b1];
            rows.push(DataValue::List(vec![
                DataValue::from(chunk.chunk_id.as_str()),
                DataValue::from(idx as i64),
                DataValue::from(*b0 as i64),
                DataValue::from(*b1 as i64),
                DataValue::from(crate::hash::sha256_str(text).as_str()),
                DataValue::from(page as i64),
                DataValue::from(bbox_s.as_str()),
                DataValue::from(DERIVATION_VERSION),
            ]));
            stats.sentences += 1;
        }
        stats.chunks += 1;
    }

    // ONE guarded delete for the whole document (join through doc_chunks)…
    let mut params = BTreeMap::new();
    params.insert("did".to_string(), DataValue::from(document_id));
    crate::cozo_retry::run_script_guarded(
        db,
        "?[chunk_id, sentence_idx] := *doc_chunks{chunk_id, document_id}, document_id = $did, \
         *doc_chunk_sentences{chunk_id, sentence_idx} \
         :rm doc_chunk_sentences { chunk_id, sentence_idx }",
        params,
        ScriptMutability::Mutable,
        "rm doc_chunk_sentences (doc)",
    )
    .map_err(|e| DocsError::Storage {
        message: format!("rm sentences: {e}"),
    })?;
    // …and ONE guarded batched insert (chunked to keep single scripts bounded).
    for batch in rows.chunks(5000) {
        let mut params = BTreeMap::new();
        params.insert("rows".to_string(), DataValue::List(batch.to_vec()));
        crate::cozo_retry::run_script_guarded(
            db,
            "?[chunk_id, sentence_idx, byte_start, byte_end, text_sha256, page, bbox, \
             derivation_version] <- $rows \
             :put doc_chunk_sentences { chunk_id, sentence_idx => byte_start, byte_end, \
             text_sha256, page, bbox, derivation_version }",
            params,
            ScriptMutability::Mutable,
            "put doc_chunk_sentences",
        )
        .map_err(|e| DocsError::Storage {
            message: format!("put sentences: {e}"),
        })?;
    }
    Ok(stats)
}

fn page_at(breaks: &[crate::models::PageBreak], byte: usize) -> Option<u32> {
    breaks
        .iter()
        .filter(|b| b.offset_in_chunk <= byte)
        .max_by_key(|b| b.offset_in_chunk)
        .map(|b| b.page)
}

/// Verify a sample of stored sentences: re-slice the chunk content by the stored
/// byte span and compare sha256. Returns `(checked, mismatches)`.
pub fn verify_sample(
    db: &DbInstance,
    sample: usize,
    seed: u64,
) -> Result<(usize, usize), DocsError> {
    let out = db
        .run_script(
            "?[chunk_id, sentence_idx, byte_start, byte_end, text_sha256] := \
             *doc_chunk_sentences{chunk_id, sentence_idx, byte_start, byte_end, text_sha256}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| DocsError::Storage {
            message: format!("sentence sample: {e}"),
        })?;
    if out.rows.is_empty() {
        return Ok((0, 0));
    }
    // deterministic LCG sample
    let mut state = seed;
    let mut pick = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as usize
    };
    let mut mismatches = 0usize;
    let mut checked = 0usize;
    let mut content_cache: BTreeMap<String, String> = BTreeMap::new();
    for _ in 0..sample.min(out.rows.len()) {
        let row = &out.rows[pick() % out.rows.len()];
        let chunk_id = row[0].get_str().unwrap_or_default().to_string();
        let (b0, b1) = (
            row[2].get_int().unwrap_or(0) as usize,
            row[3].get_int().unwrap_or(0) as usize,
        );
        let want = row[4].get_str().unwrap_or_default();
        let content = match content_cache.get(&chunk_id) {
            Some(c) => c.clone(),
            None => {
                let mut p = BTreeMap::new();
                p.insert("c".to_string(), DataValue::from(chunk_id.as_str()));
                let r = db
                    .run_script(
                        "?[content] := *doc_chunks{chunk_id, content}, chunk_id = $c",
                        p,
                        ScriptMutability::Immutable,
                    )
                    .map_err(|e| DocsError::Storage {
                        message: format!("chunk read: {e}"),
                    })?;
                let c = r
                    .rows
                    .first()
                    .and_then(|row| row[0].get_str())
                    .unwrap_or_default()
                    .to_string();
                content_cache.insert(chunk_id.clone(), c.clone());
                c
            }
        };
        checked += 1;
        let ok = content
            .get(b0..b1)
            .map(|s| crate::hash::sha256_str(s) == want)
            .unwrap_or(false);
        if !ok {
            mismatches += 1;
        }
    }
    Ok((checked, mismatches))
}

fn storage(e: anyhow::Error) -> DocsError {
    DocsError::Storage {
        message: e.to_string(),
    }
}

#[cfg(test)]
#[path = "sentence_index_tests.rs"]
mod tests;
