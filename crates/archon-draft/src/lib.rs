//! archon-draft — FCDP drafting-protocol primitives: typed context-pack schema,
//! G-P pack validation, «Qnn» quote substitution, stylometric measurement over
//! archon-lanham, and the two-tier G-A comparator.
//! Promoted from the validated FCDP sandbox (see docs/fcdp/README.md).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// Orchestration ports (FCDP follow-up #2): mechanical gauntlet + provenance chain +
// model call. Byte-faithful re-implementations of scripts/fcdp/{gauntlet,provenance}.py
// and the fable() model helper.
pub mod fable;
pub mod gauntlet;
pub mod judge;
pub mod orchestrator;
pub mod provenance;

// ── M1: Pack schema (FCDP v2 §1 + plan-v2 Move 3 P2b) ──────────────────────

#[derive(Serialize, Deserialize, Debug)]
pub struct Pack {
    pub meta: PackMeta,
    pub p1_task: P1Task,
    pub p2_style_target: P2Style,
    #[serde(default)]
    pub p2b_exemplars: Vec<Exemplar>,
    pub p3_terminology_locks: Vec<String>,
    pub p8_negative_constraints: Vec<String>,
    pub p9_usage_statement: String,
    pub p4a_quote_index: Vec<QuoteIndexEntry>,
    pub p5_evidence: Vec<EvidenceItem>,
    pub p6_semantics: String,
    pub p7_foundation: String,
    /// Path to the quotes.json bank (P4b), relative to the pack file.
    pub p4b_bank_path: String,
    pub verification: Verification,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PackMeta {
    pub section_id: String,
    pub pack_version: String,
    pub register: String, // "ma-applications" | "part-i"
    pub created: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct P1Task {
    pub section_identity: String,
    pub insertion_point: String,
    pub target_words: (u32, u32),
    pub audience: String,
    pub latex_conventions: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct P2Style {
    /// Path to the locked gate config (ga-gate-locked-*.json).
    pub gate_config_path: String,
    pub register_name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Exemplar {
    pub movement_type: String, // theoretical-exposition | close-reading | transition-argument
    pub source: String,
    pub text: String,
    pub approved: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct QuoteIndexEntry {
    pub id: String, // "Q1"
    pub source: String,
    pub locus: String,
    pub description: String,
    pub intended_use: String, // rhetorical role, checked by G-E/G-G judges
}

#[derive(Serialize, Deserialize, Debug)]
pub struct EvidenceItem {
    pub id: String, // "E1"
    pub grade: EvidenceGrade,
    pub content: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
pub enum EvidenceGrade {
    #[serde(rename = "CONFIRMED")]
    Confirmed,
    #[serde(rename = "AUTHOR-CONFIRMED")]
    AuthorConfirmed,
    #[serde(rename = "UNCERTAIN")]
    Uncertain,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Verification {
    /// ISO date the P4b quotes were verified against PDFs — must be the pack-assembly session.
    pub quotes_verified_at: String,
    pub verified_against: Vec<String>,
    /// TEST-FIXTURE packs skip the same-session verification requirement (G-P warns instead).
    #[serde(default)]
    pub test_fixture: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct QuoteEntry {
    pub text: String,
    #[serde(default)]
    pub cite: String,
}

pub type QuoteBank = BTreeMap<String, QuoteEntry>;

/// G-P: validate a pack + its bank. Returns (errors, warnings).
pub fn gp_validate(pack: &Pack, bank: &QuoteBank, today: &str) -> (Vec<String>, Vec<String>) {
    let mut errs = Vec::new();
    let mut warns = Vec::new();
    if pack.p4a_quote_index.is_empty() && !bank.is_empty() {
        errs.push("P4a empty but P4b bank non-empty".into());
    }
    let index_ids: Vec<&str> = pack.p4a_quote_index.iter().map(|q| q.id.as_str()).collect();
    for id in &index_ids {
        if !bank.contains_key(*id) {
            errs.push(format!("P4a id {id} missing from P4b bank"));
        }
    }
    for id in bank.keys() {
        if !index_ids.contains(&id.as_str()) {
            errs.push(format!("P4b id {id} missing from P4a index"));
        }
    }
    for q in bank.values() {
        if q.text.trim().is_empty() {
            errs.push("P4b entry with empty text".into());
        }
    }
    for e in &pack.p5_evidence {
        if e.content.trim().is_empty() {
            errs.push(format!("P5 {} has empty content", e.id));
        }
    }
    if pack.p3_terminology_locks.is_empty() {
        errs.push("P3 lock list empty — copy the lock list in, never reference it".into());
    }
    if pack.p9_usage_statement.trim().is_empty() {
        errs.push("P9 usage statement missing".into());
    }
    if pack.p7_foundation.trim().is_empty() {
        warns.push("P7 foundation empty — new-section pack? confirm intentional".into());
    }
    for ex in &pack.p2b_exemplars {
        if !ex.approved {
            warns.push(format!(
                "P2b exemplar ({}) not user-approved yet",
                ex.movement_type
            ));
        }
    }
    if pack.verification.quotes_verified_at != today {
        let msg = format!(
            "quotes_verified_at {} != today {today} — quotes must be PDF-verified in-session",
            pack.verification.quotes_verified_at
        );
        if pack.verification.test_fixture {
            warns.push(format!("[test-fixture] {msg}"));
        } else {
            errs.push(msg);
        }
    }
    (errs, warns)
}

// ── M2: quote-ID substitution (1:1 port of scripts/substitute-quote-ids.py) ─

pub struct SubstitutionResult {
    pub output: String,
    pub used: Vec<String>,
    pub unused: Vec<String>,
    pub unknown: Vec<String>,
}

/// Replace «Qnn» / «Qnn+» (text + cite) / «Qnn@» (cite only) from the bank.
pub fn substitute_quote_ids(draft: &str, bank: &QuoteBank) -> SubstitutionResult {
    let re = regex::Regex::new(r"«([A-Z]\d+)([+@]?)»").unwrap();
    let mut used = std::collections::BTreeSet::new();
    let mut unknown = std::collections::BTreeSet::new();
    let output = re
        .replace_all(draft, |caps: &regex::Captures| {
            let qid = &caps[1];
            let mode = &caps[2];
            match bank.get(qid) {
                None => {
                    unknown.insert(qid.to_string());
                    caps[0].to_string()
                }
                Some(e) => {
                    used.insert(qid.to_string());
                    match mode {
                        "@" => e.cite.clone(),
                        "+" if !e.cite.is_empty() => format!("{} {}", e.text, e.cite),
                        _ => e.text.clone(),
                    }
                }
            }
        })
        .into_owned();
    let unused = bank
        .keys()
        .filter(|k| !used.contains(*k))
        .cloned()
        .collect();
    SubstitutionResult {
        output,
        used: used.into_iter().collect(),
        unused,
        unknown: unknown.into_iter().collect(),
    }
}

// ── M2: G-A two-tier comparator ─────────────────────────────────────────────

#[derive(Deserialize)]
pub struct GateConfig {
    pub tier1_per_section: BTreeMap<String, BandSpec>,
    pub tier2_chapter: BTreeMap<String, BandSpec>,
    pub categorical_labels: serde_json::Value,
    #[serde(default)]
    pub tier2_labels: Vec<String>,
}

#[derive(Deserialize)]
pub struct BandSpec {
    pub target: f64,
    pub band: (f64, f64),
}

#[derive(Serialize, Debug)]
pub struct GaReport {
    pub scale: String, // "section" | "chapter"
    pub hard_failures: Vec<String>,
    pub advisories: Vec<String>,
    pub label_failures: Vec<String>,
    pub pass: bool,
}

/// Compare a metrics JSON (measure_text output shape) against the gate config.
/// `chapter_scale` = accumulated prose ≥ 2500 w → Tier-2 becomes hard.
pub fn ga_compare(metrics: &serde_json::Value, cfg: &GateConfig, chapter_scale: bool) -> GaReport {
    let get = |name: &str| -> Option<f64> {
        metrics
            .get(name)
            .or_else(|| metrics.get("lanham").and_then(|l| l.get(name)))
            .and_then(|v| v.as_f64())
    };
    let mut hard = Vec::new();
    let mut adv = Vec::new();
    for (name, spec) in &cfg.tier1_per_section {
        if let Some(v) = get(name) {
            if v < spec.band.0 || v > spec.band.1 {
                hard.push(format!(
                    "T1 {name}: {v:.3} outside [{:.3},{:.3}]",
                    spec.band.0, spec.band.1
                ));
            }
        }
    }
    for (name, spec) in &cfg.tier2_chapter {
        if let Some(v) = get(name) {
            if v < spec.band.0 || v > spec.band.1 {
                let msg = format!(
                    "T2 {name}: {v:.3} outside [{:.3},{:.3}]",
                    spec.band.0, spec.band.1
                );
                if chapter_scale {
                    hard.push(msg);
                } else {
                    adv.push(format!("{msg} [measured at unreliable scale — advisory]"));
                }
            }
        }
    }
    let mut label_failures = Vec::new();
    if let (Some(want), Some(got)) = (
        cfg.categorical_labels.as_object(),
        metrics
            .get("lanham")
            .and_then(|l| l.get("labels"))
            .and_then(|l| l.as_object()),
    ) {
        for (k, wv) in want {
            let gv = got.get(k).and_then(|v| v.as_str()).unwrap_or("");
            // labels derived from Tier-2 metrics gate only at chapter scale
            let t2_label = cfg.tier2_labels.iter().any(|t| t == k);
            if t2_label && !chapter_scale {
                if let serde_json::Value::String(s) = wv {
                    if gv != s {
                        adv.push(format!("label {k}: got '{gv}', want '{s}' [T2-derived label — advisory at section scale]"));
                    }
                }
                continue;
            }
            match wv {
                serde_json::Value::String(s) => {
                    if gv != s {
                        label_failures.push(format!("label {k}: got '{gv}', want '{s}'"));
                    }
                }
                serde_json::Value::Object(o) => {
                    if let Some(acc) = o.get("accept").and_then(|a| a.as_array()) {
                        if !acc.iter().any(|a| a.as_str() == Some(gv)) {
                            label_failures
                                .push(format!("label {k}: got '{gv}', want one of {acc:?}"));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let pass = hard.is_empty() && label_failures.is_empty();
    GaReport {
        scale: if chapter_scale { "chapter" } else { "section" }.into(),
        hard_failures: hard,
        advisories: adv,
        label_failures,
        pass,
    }
}

// ── measurement: sentence axes + full Lanham metrics (G-A's input) ─────────

use archon_lanham::{full_analysis, split_sentences, tokenize};

/// Strip LaTeX commands and markdown emphasis so the analyzer sees prose.
pub fn strip_markup(raw: &str) -> String {
    let mut s = String::with_capacity(raw.len());
    for line in raw.lines() {
        let t = line.trim_start();
        if t.starts_with('%') || t.starts_with("\\documentclass") || t.starts_with("\\usepackage") {
            continue;
        }
        s.push_str(line);
        s.push('\n');
    }
    let re_cmd_arg = regex::Regex::new(r"\\[a-zA-Z]+\*?\{([^{}]*)\}").unwrap();
    let re_cmd = regex::Regex::new(r"\\[a-zA-Z]+\*?").unwrap();
    let re_md = regex::Regex::new(r"[*_`#>]|^\s*[-+]\s").unwrap();
    let s = re_cmd_arg.replace_all(&s, "$1").into_owned();
    let s = re_cmd.replace_all(&s, "").into_owned();
    re_md.replace_all(&s, "").into_owned()
}

/// Measure one text blob: sentence axes + full Lanham metrics as JSON
/// (LanhamMetrics derives only Clone upstream, so fields are mapped by hand).
pub fn measure_text(text: &str) -> serde_json::Value {
    let sents = split_sentences(text);
    let lens: Vec<usize> = sents
        .iter()
        .map(|s| tokenize(s).len())
        .filter(|&n| n > 0)
        .collect();
    let n = lens.len().max(1);
    let total: usize = lens.iter().sum();
    let m = full_analysis(text);
    let t = &m.tacit_patterns;
    let l = &m.labels;
    serde_json::json!({
        "word_count": total,
        "sentence_count": n,
        "avg_sentence_length": total as f64 / n as f64,
        "short_share_lt15": lens.iter().filter(|&&x| x < 15).count() as f64 / n as f64,
        "long_share_gt30": lens.iter().filter(|&&x| x > 30).count() as f64 / n as f64,
        "lanham": {
            "nounVerbRatio": m.noun_verb_ratio,
            "nominalizationDensity": m.nominalization_density,
            "prepositionalPhraseDensity": m.prepositional_phrase_density,
            "beVerbRatio": m.be_verb_ratio,
            "parataxisHypotaxisRatio": m.parataxis_hypotaxis_ratio,
            "coordinatingConjunctionDensity": m.coordinating_conjunction_density,
            "subordinatingConjunctionDensity": m.subordinating_conjunction_density,
            "periodicRunningRatio": m.periodic_running_ratio,
            "preMainVerbClauseCount": m.pre_main_verb_clause_count,
            "voiceScore": m.voice_score,
            "dynamicRange": m.dynamic_range,
            "latinateGermanicRatio": m.latinate_germanic_ratio,
            "registerMarkednessScore": m.register_markedness_score,
            "opacityScore": m.opacity_score,
            "selfConsciousnessScore": m.self_consciousness_score,
            "tacitPatterns": {
                "alliterationDensity": t.alliteration_density,
                "polyptotonDensity": t.polyptoton_density,
                "chiasmusCount": t.chiasmus_count,
                "antithesisCount": t.antithesis_count,
                "anaphoraCount": t.anaphora_count,
                "isocolonCount": t.isocolon_count,
                "climaxPatternCount": t.climax_pattern_count,
            },
            "labels": {
                "nounVerb": l.noun_verb,
                "parataxisHypotaxis": l.parataxis_hypotaxis,
                "periodicRunning": l.periodic_running,
                "voice": l.voice,
                "primaryRegister": l.primary_register,
                "registerMixed": l.register_mixed,
                "opacity": l.opacity,
            },
        },
    })
}

#[cfg(test)]

mod tests {
    use super::*;

    fn bank() -> QuoteBank {
        let mut b = QuoteBank::new();
        b.insert(
            "Q1".into(),
            QuoteEntry {
                text: "``quoted words''".into(),
                cite: "(Gross 4)".into(),
            },
        );
        b.insert(
            "Q2".into(),
            QuoteEntry {
                text: "``other''".into(),
                cite: String::new(),
            },
        );
        b
    }

    #[test]
    fn substitute_modes() {
        let r = substitute_quote_ids("A «Q1+» B «Q2» C «Q1@» D", &bank());
        assert_eq!(
            r.output,
            "A ``quoted words'' (Gross 4) B ``other'' C (Gross 4) D"
        );
        assert!(r.unknown.is_empty());
        assert!(r.unused.is_empty());
    }

    #[test]
    fn substitute_unknown_id_flagged() {
        let r = substitute_quote_ids("«Q9»", &bank());
        assert_eq!(r.unknown, vec!["Q9".to_string()]);
        assert_eq!(r.output, "«Q9»"); // left in place
        assert_eq!(r.unused.len(), 2);
    }
}
