//! Token-efficient memory compression layer.
//!
//! Compresses raw memory text (e.g. from CODING_NAMESPACES) into a compact
//! symbolic format readable by Claude without decoders. Target: 10x compression
//! (2000 tokens -> <200 tokens).
//!
//! The output uses the AAAK-inspired symbolic dialect:
//! ```text
//! [MEM|v1]
//! ENT:USvc|AuthMW|TokVal|PgRepo
//! DEC:postgres>persist|jwt>auth|redis>cache
//! REL:AuthMW->TokVal|USvc->PgRepo
//! PAT:repo>data|facade>orchestrate
//! FIX:!unwrap@err|!clone@hotpath
//! SH:P1=INNOC|P2=INNOC
//! @P1:USvc+AuthMW @P3:PgRepo
//! ```


mod abbreviation;
mod extraction;
mod output;

use extraction::extract;
use output::build_compressed;
#[cfg(test)]
use abbreviation::{abbreviate, split_camel_case};


/// Compressed memory output.
#[derive(Debug, Clone)]
pub struct CompressedMemory {
    /// The compressed output text.
    pub text: String,
    /// Estimated token count of `text`.
    pub token_estimate: usize,
    /// Number of distinct entities preserved.
    pub entities_preserved: usize,
    /// Compression ratio: input_tokens / output_tokens.  0.0 when empty.
    pub compression_ratio: f64,
    /// Which section tags are present (e.g. "ENT", "DEC", "REL").
    pub sections_present: Vec<String>,
}

/// Approximate token count using chars/4 heuristic (rounded up).
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Compress raw memory text into compact symbolic format.
pub fn compress(raw: &str, budget_tokens: usize) -> CompressedMemory {
    if raw.trim().is_empty() {
        return CompressedMemory {
            text: String::new(),
            token_estimate: 0,
            entities_preserved: 0,
            compression_ratio: 0.0,
            sections_present: Vec::new(),
        };
    }

    let extracted = extract(raw);
    build_compressed(raw, &extracted, budget_tokens, None)
}

/// Compress with deduplication against existing prompt context.
///
/// Entities/decisions/relationships that already appear (case-insensitive) in
/// `existing_context` are omitted from the output.
pub fn compress_with_dedup(raw: &str, existing_context: &str, budget: usize) -> CompressedMemory {
    if raw.trim().is_empty() {
        return CompressedMemory {
            text: String::new(),
            token_estimate: 0,
            entities_preserved: 0,
            compression_ratio: 0.0,
            sections_present: Vec::new(),
        };
    }

    let extracted = extract(raw);
    build_compressed(raw, &extracted, budget, Some(existing_context))
}

/// Generate a human-readable hint from compressed output (for debugging).
pub fn decompress_hint(compressed: &CompressedMemory) -> String {
    if compressed.text.is_empty() {
        return String::from("(empty memory)");
    }

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "Memory snapshot ({} tokens, {:.1}x compression, {} entities)",
        compressed.token_estimate, compressed.compression_ratio, compressed.entities_preserved,
    ));

    for line in compressed.text.lines() {
        let line = line.trim();
        if line.starts_with("[MEM|") {
            continue; // skip header
        }
        if let Some(rest) = line.strip_prefix("ENT:") {
            let ents: Vec<&str> = rest.split('|').collect();
            lines.push(format!("Entities: {}", ents.join(", ")));
        } else if let Some(rest) = line.strip_prefix("DEC:") {
            let decs: Vec<&str> = rest.split('|').collect();
            lines.push(format!(
                "Decisions: {}",
                decs.iter()
                    .map(|d| d.replace('>', " -> "))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        } else if let Some(rest) = line.strip_prefix("REL:") {
            let rels: Vec<&str> = rest.split('|').collect();
            lines.push(format!("Relationships: {}", rels.join(", ")));
        } else if let Some(rest) = line.strip_prefix("PAT:") {
            let pats: Vec<&str> = rest.split('|').collect();
            lines.push(format!(
                "Patterns: {}",
                pats.iter()
                    .map(|p| p.replace('>', " for "))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        } else if let Some(rest) = line.strip_prefix("FIX:") {
            let fixes: Vec<&str> = rest.split('|').collect();
            lines.push(format!("Corrections: {}", fixes.join(", ")));
        } else if let Some(rest) = line.strip_prefix("SH:") {
            let sh: Vec<&str> = rest.split('|').collect();
            lines.push(format!("Sherlock verdicts: {}", sh.join(", ")));
        } else if line.starts_with('@') {
            lines.push(format!("Phase tags: {}", line));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
#[path = "compression/tests.rs"]
mod tests;
