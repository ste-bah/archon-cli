//! Judge gates G-E (foundation fidelity) and G-G (consistency).
//!
//! Ported from `scripts/fcdp/judge.py`. The model call runs through [`crate::fable`];
//! everything else here is deterministic and golden-tested:
//!   * seed = `int(sha256(section_id)[:8], 16)`;
//!   * a reproducible LCG Fisher-Yates battery shuffle (no RNG lib → identical on every
//!     platform) — validated against the real M6 orders (GE `[E3,E4,E1,E2,E5]`,
//!     GG `[G3,G4,G2,G1]`);
//!   * block-based, fail-closed verdict extraction (anything not a clean YES/NO counts
//!     toward the defect side).
//!
//! `extract` is HARDENED beyond the Python original (see its rustdoc) to cut phantom
//! fail-closed defects from judge format drift — a relaxed VERDICT separator, broadened
//! item headers, and a bare-terminal-YES/NO fallback. The prompt is also tightened with
//! a worked example. All changes only ever rescue an UNAMBIGUOUS verdict; a genuinely
//! murky answer still fail-closes, and the golden extraction fixtures parse identically.

use crate::fable::{self, FableError};
use crate::{EvidenceGrade, Pack};
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    GE,
    GG,
}

impl Gate {
    pub fn as_str(&self) -> &'static str {
        match self {
            Gate::GE => "GE",
            Gate::GG => "GG",
        }
    }
    pub fn parse(s: &str) -> Option<Gate> {
        match s {
            "GE" => Some(Gate::GE),
            "GG" => Some(Gate::GG),
            _ => None,
        }
    }
}

/// A battery item: (code, question, `bad_on` — the verdict that counts as a defect).
type BatteryItem = (&'static str, &'static str, &'static str);

fn base_battery(gate: Gate) -> Vec<BatteryItem> {
    match gate {
        Gate::GE => vec![
            ("E1", "Is every claim of the foundation text present in the passage (none silently dropped)?", "NO"),
            ("E2", "Does the passage alter any foundation claim's strength or modality beyond what its evidence grade licenses (e.g., asserting an UNCERTAIN item flatly)?", "YES"),
            ("E3", "Does the passage assert anything with no warrant in the evidence bank or foundation text?", "YES"),
            ("E4", "Is each quotation used in a way consistent with its stated intended rhetorical use in the quote index?", "NO"),
            ("E5", "Are all evidence items asserted at a strength matching their grade (AUTHOR-CONFIRMED flat; CONFIRMED with its measure; UNCERTAIN hedged or omitted)?", "NO"),
        ],
        Gate::GG => vec![
            ("G1", "Is every locked term used per its locked definition at every occurrence?", "NO"),
            ("G2", "Does any later passage contradict an earlier passage's claim without explicit acknowledgment?", "YES"),
            ("G3", "Is each evidence item characterized with the same value/strength everywhere it appears?", "NO"),
            ("G4", "Do any two passages characterize the same concept incompatibly?", "YES"),
        ],
    }
}

/// seed = `int(sha256(section_id).hexdigest()[:8], 16)`.
pub fn battery_seed(section_id: &str) -> u64 {
    let hex = crate::provenance::sha256_hex(section_id.as_bytes());
    u64::from_str_radix(&hex[..8], 16).expect("8 hex chars")
}

/// Deterministic LCG Fisher-Yates permutation of `0..n` (Python parity).
pub fn shuffle_order(seed: u64, n: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    let mut r = seed;
    let mut i = n - 1;
    while i > 0 {
        r = (r.wrapping_mul(1103515245).wrapping_add(12345)) % (1u64 << 31);
        let j = (r % (i as u64 + 1)) as usize;
        order.swap(i, j);
        i -= 1;
    }
    order
}

/// The battery in shuffled order + the seed.
fn shuffled_battery(gate: Gate, section_id: &str) -> (Vec<BatteryItem>, u64) {
    let seed = battery_seed(section_id);
    let base = base_battery(gate);
    let order = shuffle_order(seed, base.len());
    let battery = order.iter().map(|&i| base[i]).collect();
    (battery, seed)
}

fn grade_str(g: EvidenceGrade) -> &'static str {
    match g {
        EvidenceGrade::Confirmed => "CONFIRMED",
        EvidenceGrade::AuthorConfirmed => "AUTHOR-CONFIRMED",
        EvidenceGrade::Uncertain => "UNCERTAIN",
    }
}

/// Reference-material context block (lean, per-gate).
pub fn build_ctx(gate: Gate, pack: &Pack, plan: Option<&str>) -> String {
    match gate {
        Gate::GE => {
            let ev = pack
                .p5_evidence
                .iter()
                .map(|e| format!("[{} {}] {}", e.id, grade_str(e.grade), e.content))
                .collect::<Vec<_>>()
                .join("\n");
            let qi = pack
                .p4a_quote_index
                .iter()
                .map(|q| {
                    format!(
                        "[{}] {} {} \u{2014} intended: {}",
                        q.id, q.source, q.locus, q.intended_use
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "CONCEPTUAL SEMANTICS (assertions warranted here are in-pack):\n{}\n\nFOUNDATION TEXT:\n{}\n\nEVIDENCE BANK (grades set assertion strength):\n{}\n\nQUOTE INDEX (intended rhetorical use):\n{}",
                pack.p6_semantics, pack.p7_foundation, ev, qi
            )
        }
        Gate::GG => {
            let locks = pack.p3_terminology_locks.join("\n");
            let plan_block = match plan {
                Some(p) => format!(
                    "\n\nAPPROVED MOVEMENT PLAN (the claims the passage is committed to; FCDP G-G rubric context):\n{p}"
                ),
                None => String::new(),
            };
            format!(
                "TERMINOLOGY LOCKS:\n{}\n\nCONCEPTUAL SEMANTICS:\n{}{}",
                locks, pack.p6_semantics, plan_block
            )
        }
    }
}

/// Assemble the judge prompt from ctx, draft, and the (shuffled) battery.
fn build_prompt(ctx: &str, draft: &str, battery: &[BatteryItem]) -> String {
    let items = battery
        .iter()
        .enumerate()
        .map(|(i, b)| format!("{}. [{}] {}", i + 1, b.0, b.1))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You are evaluating a passage of scholarly prose against reference material. Answer each numbered question independently.\n\nREFERENCE MATERIAL:\n{ctx}\n\nPASSAGE UNDER EVALUATION:\n{draft}\n\nQUESTIONS:\n{items}\n\nANSWER FORMAT — follow it EXACTLY. For each question, on ITS OWN line: the plain number and a period, one sentence of reasoning, then the literal verdict marker. Use this exact shape, and nothing else:\n\n1. <one sentence of reasoning>. VERDICT: YES\n2. <one sentence of reasoning>. VERDICT: NO\n\nRULES: start each line with the plain number and a period (\"1.\", \"2.\", …) — no bold, no headings, no bullets. End each line with exactly \"VERDICT: YES\" or \"VERDICT: NO\" — the word VERDICT, a colon, then YES or NO. Answer every question, in order. Do not write a preamble or a closing summary."
    )
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct TranscriptItem {
    pub item: String,
    pub question: String,
    pub line: String,
    pub verdict: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct JudgeReport {
    pub gate: String,
    pub model: String,
    pub stop_reason: Option<String>,
    pub battery_seed: u64,
    pub battery_order: Vec<String>,
    pub defects: Vec<String>,
    pub pass: bool,
    pub transcript: Vec<TranscriptItem>,
    pub usage: Value,
}

fn char_take(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Conservative fallback for a block whose verdict wasn't written as `VERDICT: …`:
/// if the block's LAST non-empty line, stripped to bare letters, is exactly `YES`
/// or `NO`, take it (the model put the verdict on its own line). Deliberately
/// narrow — a lone terminal YES/NO is unambiguously the verdict, whereas
/// "…, so no." (letters `SONO`) or "…unclear." stays fail-closed. Never turns a
/// murky answer into a pass, so the fail-closed safety property is preserved.
fn terminal_verdict(block: &str) -> Option<String> {
    let last = block.lines().rev().find(|l| !l.trim().is_empty())?;
    let letters: String = last
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_uppercase();
    match letters.as_str() {
        "YES" => Some("YES".to_string()),
        "NO" => Some("NO".to_string()),
        _ => None,
    }
}

/// Block-based, fail-closed verdict extraction. Returns (defects, transcript).
/// Item N's block runs from its header ("N." / "N)" / "N:", optionally
/// bold/bulleted, multiline) to the next present header. A block with no verdict
/// is a fail-closed defect; a verdict equal to the item's `bad_on` is a defect.
/// HARDENED beyond the original Python port to cut phantom fail-closed defects
/// (judge format drift): a relaxed VERDICT separator, broadened item headers, and
/// a bare-terminal-YES/NO fallback ([`terminal_verdict`]). Every change only ever
/// RESCUES an unambiguous verdict — genuinely murky answers still fail-closed.
pub fn extract(
    gate: Gate,
    battery: &[BatteryItem],
    text: &str,
) -> (Vec<String>, Vec<TranscriptItem>) {
    // Hardened vs. the Python port (still fail-closed): tolerate a dash / en-dash
    // / em-dash (or nothing) where a colon is expected, so "VERDICT — YES" reads.
    let verdict_re =
        regex::Regex::new(r"(?i)VERDICT\s*[:\u{2013}\u{2014}-]?\s*\**\s*(YES|NO)").unwrap();
    let g = gate.as_str();

    // byte offsets of each item header, or None
    let starts: Vec<Option<usize>> = (0..battery.len())
        .map(|i| {
            // Hardened: tolerate a leading markdown bullet/heading marker, bold
            // around the number, and a ":" terminator — so "**1.**", "- 1.",
            // "### 1", and "1:" all register as item headers.
            let re = regex::Regex::new(&format!(
                r"(?m)^\s*(?:[#>*\-]\s+)?(?:\*\*)?{}(?:\*\*)?\s*[.):]",
                i + 1
            ))
            .unwrap();
            re.find(text).map(|m| m.start())
        })
        .collect();

    let mut defects = Vec::new();
    let mut transcript = Vec::new();
    for (i, (code, q, bad_on)) in battery.iter().enumerate() {
        let block: String = match starts[i] {
            None => String::new(),
            Some(s) => {
                let nxt = starts[i + 1..]
                    .iter()
                    .find_map(|o| *o)
                    .unwrap_or(text.len());
                text[s..nxt].to_string()
            }
        };
        let verdict = verdict_re
            .captures(&block)
            .map(|c| c[1].to_uppercase())
            .or_else(|| terminal_verdict(&block));
        // match = " ".join(block.split())[:300] if block else None
        let match_str: Option<String> = if block.is_empty() {
            None
        } else {
            Some(char_take(&normalize_ws(&block), 300))
        };
        let line = match match_str.as_deref() {
            Some(m) if !m.is_empty() => m.to_string(),
            _ => "(not found)".to_string(),
        };
        transcript.push(TranscriptItem {
            item: code.to_string(),
            question: q.to_string(),
            line,
            verdict: verdict.clone(),
        });
        match verdict {
            None => defects.push(format!(
                "{g}/{code}: no clean verdict extracted \u{2014} fail-closed"
            )),
            Some(ref v) if v == bad_on => defects.push(format!(
                "{g}/{code}: {}",
                match_str.unwrap_or_default().trim()
            )),
            _ => {}
        }
    }
    (defects, transcript)
}

/// Run a judge gate through an injectable model call (the orchestrator's replay seam).
/// `model` labels the report; `plan` supplies the D1 movement plan for G-G (per FCDP §5).
pub fn judge_with(
    call: &dyn Fn(&str, u32) -> Result<fable::FableResponse, FableError>,
    model: &str,
    gate: Gate,
    draft: &str,
    pack: &Pack,
    plan: Option<&str>,
) -> Result<JudgeReport, FableError> {
    let (battery, seed) = shuffled_battery(gate, &pack.meta.section_id);
    let ctx = build_ctx(gate, pack, plan);
    let prompt = build_prompt(&ctx, draft, &battery);
    let resp = call(&prompt, 9000)?;
    let (defects, transcript) = extract(gate, &battery, &resp.text);
    Ok(JudgeReport {
        gate: gate.as_str().to_string(),
        model: model.to_string(),
        stop_reason: resp.stop_reason,
        battery_seed: seed,
        battery_order: battery.iter().map(|b| b.0.to_string()).collect(),
        defects: defects.clone(),
        pass: defects.is_empty(),
        transcript,
        usage: resp.usage,
    })
}

/// Run a judge gate live through a ready [`fable::FableClient`] (subscription or API key).
/// `plan` supplies the D1 movement plan for G-G (per FCDP §5).
pub fn judge(
    client: &fable::FableClient,
    model: &str,
    gate: Gate,
    draft: &str,
    pack: &Pack,
    plan: Option<&str>,
) -> Result<JudgeReport, FableError> {
    let call = |p: &str, mt: u32| client.call(model, p, mt);
    judge_with(&call, model, gate, draft, pack, plan)
}

/// Expose the shuffled battery order (codes) for tests/tooling.
pub fn battery_order(gate: Gate, section_id: &str) -> (Vec<String>, u64) {
    let (battery, seed) = shuffled_battery(gate, section_id);
    (battery.iter().map(|b| b.0.to_string()).collect(), seed)
}

/// Deterministic evaluation (no model call): shuffle the battery for `section_id`, then
/// extract verdicts from an already-obtained judge `text`. Returns (battery_order,
/// defects, transcript). Used by the P4 extraction differential test.
pub fn evaluate(
    gate: Gate,
    section_id: &str,
    text: &str,
) -> (Vec<String>, Vec<String>, Vec<TranscriptItem>) {
    let (battery, _seed) = shuffled_battery(gate, section_id);
    let order = battery.iter().map(|b| b.0.to_string()).collect();
    let (defects, transcript) = extract(gate, &battery, text);
    (order, defects, transcript)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_and_shuffle_match_m6_golden() {
        // Real M6 data: section_id part-ii-two-unions-v1 → seed 4256758519,
        // GE order [E3,E4,E1,E2,E5], GG order [G3,G4,G2,G1].
        let sid = "part-ii-two-unions-v1";
        assert_eq!(battery_seed(sid), 4_256_758_519);
        let (ge, seed_ge) = battery_order(Gate::GE, sid);
        assert_eq!(seed_ge, 4_256_758_519);
        assert_eq!(ge, vec!["E3", "E4", "E1", "E2", "E5"]);
        let (gg, _) = battery_order(Gate::GG, sid);
        assert_eq!(gg, vec!["G3", "G4", "G2", "G1"]);
    }

    #[test]
    fn extract_hardened_rescues_format_drift_but_keeps_fail_closed() {
        // Base (unshuffled) GE battery: header N maps to item N-1 (E1..E5).
        let battery = base_battery(Gate::GE);
        // 1: dash separator; 2: plain colon (control); 3: bold header + bare
        // terminal verdict on its own line; 4: genuinely unclear (must fail-closed);
        // 5: no space after the colon (control).
        let text = "1. Reasoning. VERDICT \u{2014} YES\n\
                    2. Reasoning. VERDICT: NO\n\
                    **3.** Reasoning here.\n\
                    NO\n\
                    4. Reasoning trails off, genuinely unclear here.\n\
                    5. Reasoning. VERDICT:YES\n";
        let (_defects, transcript) = extract(Gate::GE, &battery, text);
        let verdicts: Vec<Option<&str>> = transcript.iter().map(|t| t.verdict.as_deref()).collect();
        assert_eq!(
            verdicts,
            vec![Some("YES"), Some("NO"), Some("NO"), None, Some("YES")],
            "dash-verdict (1) + bare-terminal (3) rescued; unclear (4) stays fail-closed"
        );
    }

    #[test]
    fn terminal_verdict_only_fires_on_a_lone_yes_no() {
        assert_eq!(terminal_verdict("blah\nYES").as_deref(), Some("YES"));
        assert_eq!(terminal_verdict("x\n**NO**").as_deref(), Some("NO"));
        assert_eq!(terminal_verdict(": yes").as_deref(), Some("YES"));
        // Reasoning that merely contains yes/no must NOT be mistaken for a verdict.
        assert_eq!(terminal_verdict("reasoning, so no.").as_deref(), None);
        assert_eq!(terminal_verdict("the answer is unclear.").as_deref(), None);
        assert_eq!(terminal_verdict("").as_deref(), None);
    }
}
