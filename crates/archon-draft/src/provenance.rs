//! Provenance hash chain (FCDP Move 7) + disclosure declaration (Move 6).
//!
//! Byte-faithful port of `scripts/fcdp/provenance.py`. Records stay JSONL for now;
//! promotion to the CozoDB-backed `archon-provenance` store is follow-up #3.
//!
//! The chain hash is `sha256(prev_chain_hash + content_sha256 + canonical_json(detail))`,
//! where `canonical_json` reproduces Python's `json.dumps(detail, sort_keys=True)`
//! exactly (sorted keys, `", "`/`": "` separators, `ensure_ascii` escaping). That exact
//! reproduction is what lets a Rust `verify` accept a Python-written chain — the P1
//! golden test asserts precisely this against the real 19-record M6 chain.

use crate::Pack;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io;
use std::path::Path;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProvenanceRecord {
    pub record_id: String,
    pub artifact_id: String,
    pub stage: String,
    pub content_sha256: String,
    pub detail: Value,
    pub prev_chain_hash: String,
    pub chain_hash: String,
}

const GENESIS: &str = "GENESIS";

/// sha256 hex digest of bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let d = h.finalize();
    let mut s = String::with_capacity(64);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Reproduce Python `json.dumps(value, sort_keys=True)` byte-for-byte:
/// keys sorted, `", "` item separator, `": "` key separator, `ensure_ascii=True`.
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        // serde_json renders JSON integers without a decimal point and floats as Python
        // would for the values FCDP records emit (token counts are integers).
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_py_string(s, out),
        Value::Array(a) => {
            out.push('[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort(); // ASCII keys → byte order == Python's codepoint order
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_py_string(k, out);
                out.push_str(": ");
                write_canonical(map.get(*k).unwrap(), out);
            }
            out.push('}');
        }
    }
}

/// Python json string encoding with `ensure_ascii=True`.
fn write_py_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            // printable ASCII 0x20..=0x7e passes through; everything else is \u-escaped
            // (Python escapes 0x7f DEL and all >= 0x80 under ensure_ascii).
            c if (c as u32) < 0x7f => out.push(c),
            c => {
                let cp = c as u32;
                if cp <= 0xFFFF {
                    out.push_str(&format!("\\u{cp:04x}"));
                } else {
                    let v = cp - 0x10000;
                    let hi = 0xD800 + (v >> 10);
                    let lo = 0xDC00 + (v & 0x3FF);
                    out.push_str(&format!("\\u{hi:04x}\\u{lo:04x}"));
                }
            }
        }
    }
    out.push('"');
}

/// Parse a chain file's records (skips blank lines, as Python's `.strip().splitlines()`).
pub fn read_chain(chain_path: &Path) -> io::Result<Vec<ProvenanceRecord>> {
    let text = std::fs::read_to_string(chain_path)?;
    let mut recs = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let rec: ProvenanceRecord = serde_json::from_str(line)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        recs.push(rec);
    }
    Ok(recs)
}

/// Append a record for `artifact_path` at `stage` with `detail`, chaining onto the
/// current head. Returns the new record. Mirrors `provenance.py record`.
pub fn record(
    chain_path: &Path,
    artifact_path: &Path,
    stage: &str,
    detail: &Value,
) -> io::Result<ProvenanceRecord> {
    let prev = match read_chain(chain_path) {
        Ok(recs) => recs
            .last()
            .map(|r| r.chain_hash.clone())
            .unwrap_or_else(|| GENESIS.to_string()),
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => GENESIS.to_string(),
        Err(e) => return Err(e),
    };
    let content = std::fs::read(artifact_path)?;
    let content_hash = sha256_hex(&content);
    let record_id = format!("{stage}-{}", &content_hash[..12]);
    let chain_hash =
        sha256_hex(format!("{prev}{content_hash}{}", canonical_json(detail)).as_bytes());
    let rec = ProvenanceRecord {
        record_id,
        artifact_id: artifact_path.display().to_string(),
        stage: stage.to_string(),
        content_sha256: content_hash,
        detail: detail.clone(),
        prev_chain_hash: prev,
        chain_hash,
    };
    let mut line =
        serde_json::to_string(&rec).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    line.push('\n');
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(chain_path)?;
    f.write_all(line.as_bytes())?;
    Ok(rec)
}

/// Verify a chain's integrity. Recomputes each `chain_hash` from the stored
/// `content_sha256` + `detail` (never re-reads artifacts), exactly like
/// `provenance.py verify`. Returns `true` iff every link holds.
pub fn verify(chain_path: &Path) -> io::Result<bool> {
    let recs = read_chain(chain_path)?;
    Ok(verify_records(&recs))
}

/// Verify already-parsed records (chain integrity only).
pub fn verify_records(recs: &[ProvenanceRecord]) -> bool {
    let mut prev = GENESIS.to_string();
    let mut ok = true;
    for rec in recs {
        let want = sha256_hex(
            format!(
                "{prev}{}{}",
                rec.content_sha256,
                canonical_json(&rec.detail)
            )
            .as_bytes(),
        );
        if rec.prev_chain_hash != prev || rec.chain_hash != want {
            ok = false;
        }
        prev = rec.chain_hash.clone();
    }
    ok
}

/// Generate the author-facing disclosure declaration from the chain + pack.
/// Byte-faithful port of `provenance.py declare` (returns the text ending in a
/// single newline; the CLI's `print` adds one more).
pub fn declare(recs: &[ProvenanceRecord], pack: &Pack) -> String {
    let stages: Vec<&str> = recs.iter().map(|r| r.stage.as_str()).collect();
    let cycles = stages.iter().filter(|s| **s == "revision").count();

    let mut gateset: BTreeSet<String> = BTreeSet::new();
    for r in recs {
        if let Some(arr) = r.detail.get("gates_run").and_then(|v| v.as_array()) {
            for g in arr {
                if let Some(s) = g.as_str() {
                    gateset.insert(s.to_string());
                }
            }
        }
    }
    let gates: Vec<String> = gateset.into_iter().collect();
    let gates_str = if gates.is_empty() {
        "n/a".to_string()
    } else {
        gates.join(", ")
    };

    // dict.fromkeys(stages): unique, first-occurrence order
    let mut seen = BTreeSet::new();
    let mut uniq: Vec<&str> = Vec::new();
    for s in &stages {
        if seen.insert(*s) {
            uniq.push(*s);
        }
    }
    let stage_seq = uniq.join(" \u{2192} ");

    let head = recs
        .last()
        .map(|r| r.chain_hash.chars().take(16).collect::<String>())
        .unwrap_or_default();

    format!(
        "## Declaration of AI use \u{2014} {section}\n\n\
*(Draft for author revision \u{2014} generated from the provenance chain; formatted to the\n\
Cambridge template-declaration structure at HGSE tool/prompt/integration granularity.)*\n\n\
**Tools.** Anthropic Claude (Fable 5, `claude-fable-5`) for drafting and judge-gate\n\
evaluation, orchestrated by the FCDP v2 protocol; Lanham stylometric analyzer\n\
(`archon-lanham`, Rust) for mechanical style measurement.\n\n\
**What the model contributed.** Phrasing, discourse structure, and analysis of\n\
evidence supplied in a sealed, pre-verified context pack ({nq}\n\
quotations, {ne} graded evidence items). Stage sequence recorded:\n\
{stage_seq}.\n\n\
**What the model was barred from.** Introducing claims beyond the evidence bank;\n\
inventing or recalling citations (all loci trace to the pack; violations gate-fail);\n\
generating quoted text (quotations enter only by mechanical ID substitution from a\n\
PDF-verified bank). No retrieval was available to the model during drafting.\n\n\
**Human control points.** Plan approval before drafting; gate reports at every cycle\n\
({cycles} revision cycle(s); gates run: {gates_str}); final acceptance\n\
by the author. Nothing entered the dissertation without author sign-off.\n\n\
**Verification trail.** {nrec} hash-chained provenance records (chain head\n\
`{head}\u{2026}`), exportable to W3C-PROV. Full chain retained with\n\
the dissertation sources.\n",
        section = pack.meta.section_id,
        nq = pack.p4a_quote_index.len(),
        ne = pack.p5_evidence.len(),
        stage_seq = stage_seq,
        cycles = cycles,
        gates_str = gates_str,
        nrec = recs.len(),
        head = head,
    )
}
