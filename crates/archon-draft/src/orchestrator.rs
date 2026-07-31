//! FCDP orchestrator — the full live-section E2E sequence (port of scripts/fcdp/m6_run.py):
//!
//!   D1 movement plan → G-1 → D1.5 skeleton (+counterargument) → G-1.5
//!   → D2 movement-by-movement → substitute → mechanical gauntlet → G-A → G-E + G-G
//!   → R-loop (≤3; repair prompts restate the FULL movement plan, per L2) → provenance + declaration.
//!
//! All model calls flow through one injectable closure (`ModelCall`) — the replay seam
//! that lets tests drive the whole pipeline deterministically from recorded outputs while
//! production binds it to [`crate::fable::fable`]. Stage sequencing, the G-1 gate, the
//! gauntlet composition, the R-loop, the stop-rule, and the provenance chain are all
//! exercised without a live API.

use crate::fable::{FableError, FableResponse};
use crate::gauntlet::{run_gauntlet, GauntletReport};
use crate::judge::{self, Gate, JudgeReport};
use crate::provenance;
use crate::{
    ga_compare, measure_text, strip_markup, substitute_quote_ids, GateConfig, Pack, QuoteBank,
};
use serde_json::json;
use std::path::Path;

/// A model call: (prompt, max_tokens) → response. Bound to `fable` in production,
/// to recorded/synthetic outputs in tests.
pub type ModelCall<'a> = dyn Fn(&str, u32) -> Result<FableResponse, FableError> + 'a;

pub const MAX_CYCLES: usize = 3;
const D_MAX_TOKENS: u32 = 8000;
const REPAIR_MAX_TOKENS: u32 = 16000;

#[derive(Debug)]
pub struct Outcome {
    pub status: String,
    pub cycles: usize,
    pub final_draft: String,
    pub surfaced_defects: Vec<String>,
    pub surface_to_user_in_skeleton: bool,
    pub chain_verified: bool,
    pub declaration: String,
}

#[derive(Debug)]
pub enum RunError {
    Model(FableError),
    Gate(String),
    Io(String),
}
impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Model(e) => write!(f, "model call failed: {e}"),
            RunError::Gate(s) => write!(f, "gate failure: {s}"),
            RunError::Io(s) => write!(f, "io error: {s}"),
        }
    }
}
impl std::error::Error for RunError {}
impl From<FableError> for RunError {
    fn from(e: FableError) -> Self {
        RunError::Model(e)
    }
}

// ── prompt assembly (byte-faithful to m6_run.py) ──────────────────────────────

/// Shared HEAD block: P1–P3, P8, P9 + quote index + graded evidence + semantics + foundation.
pub fn build_head(pack: &Pack) -> String {
    let locks = pack
        .p3_terminology_locks
        .iter()
        .map(|l| format!("- {l}"))
        .collect::<Vec<_>>()
        .join("\n");
    let neg = pack
        .p8_negative_constraints
        .iter()
        .map(|c| format!("- {c}"))
        .collect::<Vec<_>>()
        .join("\n");
    let qidx = pack
        .p4a_quote_index
        .iter()
        .map(|q| {
            format!(
                "\u{ab}{}\u{bb} {} {} \u{2014} {} (intended use: {})",
                q.id, q.source, q.locus, q.description, q.intended_use
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let ev = pack
        .p5_evidence
        .iter()
        .map(|e| format!("[{} {}] {}", e.id, grade_str(e.grade), e.content))
        .collect::<Vec<_>>()
        .join("\n");
    let (lo, hi) = pack.p1_task.target_words;
    format!(
        "TASK: {}. Target {}-{} words total. LaTeX conventions: {}.\n\nUSAGE BOUNDARY: {}\n\nTERMINOLOGY & STYLE LOCKS (hard constraints):\n{}\n\nFORBIDDEN:\n{}\n\nQUOTE INDEX (quotations exist ONLY as \u{ab}Qnn\u{bb}/\u{ab}Qnn+\u{bb} markers; the quoted words enter later by mechanical substitution — never write quoted words):\n{}\n\nEVIDENCE BANK (grades set assertion strength — AUTHOR-CONFIRMED flat; CONFIRMED asserted with its content; UNCERTAIN hedged or omitted):\n{}\n\nCONCEPTUAL SEMANTICS:\n{}\n\nFOUNDATION TEXT (the existing prose this section must cohere with; drop none of its claims silently):\n{}",
        pack.p1_task.section_identity, lo, hi, pack.p1_task.latex_conventions,
        pack.p9_usage_statement, locks, neg, qidx, ev, pack.p6_semantics, pack.p7_foundation
    )
}

fn grade_str(g: crate::EvidenceGrade) -> &'static str {
    match g {
        crate::EvidenceGrade::Confirmed => "CONFIRMED",
        crate::EvidenceGrade::AuthorConfirmed => "AUTHOR-CONFIRMED",
        crate::EvidenceGrade::Uncertain => "UNCERTAIN",
    }
}

fn d1_prompt(head: &str) -> String {
    format!(
        "{head}\n\nSTAGE D1 — produce a MOVEMENT PLAN only (no prose). Output exactly this structure in markdown:\n\nFor each movement (use 3 movements):\nMOVEMENT <n>: <one-sentence claim>\nEVIDENCE: <ids from the evidence bank>\nQUOTES: <ids from the quote index>\nFOUNDATION-ANCHORS: <which foundation claims this movement carries forward>\nWORD-SHARE: <target words>\nSTYLE-NOTE: <sentence-rhythm intent for this movement, one line>\n\nThen:\nFOUNDATION DISPOSITION: for each distinct claim in the foundation text — RETAIN / EXPAND / CORRECT(state it) / OMIT(reason).\nLEDGER: every quote ID and evidence ID — ASSIGNED(movement) or UNUSED(reason)."
    )
}

fn d15_prompt(head: &str, d1: &str) -> String {
    format!(
        "{head}\n\nAPPROVED MOVEMENT PLAN:\n{d1}\n\nSTAGE D1.5 — produce a SKELETON only (structure, no prose). For each movement:\nNUCLEUS claims in order; under each, its SATELLITES labeled evidence/elaboration/concession/contrast/restatement, with each assigned quote ID and evidence ID attached to the satellite it serves.\nCOUNTERARGUMENT: for each major nucleus claim, the strongest objection an actual interlocutor of this dissertation could press (from the pack only), with disposition ANSWER(which satellite) / CONCEDE-AND-LIMIT / SURFACE-TO-USER.\nTRANSITIONS: the discourse relation each movement boundary performs.\nRHYTHM: where short landing sentences fall; where long periodic builds run."
    )
}

/// Split a D1 plan into its MOVEMENT segments (first 3), mirroring
/// `re.split(r"(?=MOVEMENT\s+\d)", d1)` then filtering to MOVEMENT-headed parts.
pub fn split_movements(d1: &str) -> Vec<String> {
    let re = regex::Regex::new(r"MOVEMENT\s+\d").unwrap();
    let starts: Vec<usize> = re.find_iter(d1).map(|m| m.start()).collect();
    let mut segs = Vec::new();
    for (i, &s) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(d1.len());
        segs.push(d1[s..end].to_string());
    }
    segs.into_iter().take(3).collect()
}

#[allow(clippy::too_many_arguments)]
fn d2_prompt(
    head: &str,
    d1: &str,
    movement_seg: &str,
    d15: &str,
    exemplars: &str,
    prior: &str,
    index: usize,
) -> String {
    let n = index + 1;
    let prior_block = if prior.is_empty() {
        String::new()
    } else {
        format!("\n\nALREADY-DRAFTED PRECEDING MOVEMENTS (continue from them; do not repeat them):\n{prior}")
    };
    format!(
        "{head}\n\nFULL MOVEMENT PLAN (for orientation — you are drafting ONLY movement {n} now):\n{d1}\n\nSKELETON FOR THIS MOVEMENT (follow its satellite structure and rhythm placements):\n{movement_seg}\n\n{d15}\n\nVOICE EXEMPLARS for this movement type (match their texture; do NOT quote or closely paraphrase them — any shared 8-word sequence is a gate failure):\n{exemplars}{prior_block}\n\nSTAGE D2 — write ONLY movement {n} as finished prose. Quotations ONLY as \u{ab}Qnn+\u{bb} or \u{ab}Qnn\u{bb} markers placed where the quote belongs, with your prose written around them. Output only the movement's prose body."
    )
}

fn repair_prompt(d1: &str, defects: &[String], exemplars: &str, passage: &str) -> String {
    let defs = defects
        .iter()
        .map(|d| format!("- {d}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Revise the passage below to fix ONLY the named defects — nothing else. Where a defect names missing content, restore exactly that; where it names unwarranted content, remove exactly that; where it names a style metric, adjust sentence architecture only, landing INSIDE the band, mid-band, not past it. Never add, drop, or alter any other claim, \u{ab}Qnn\u{bb} marker, or evidence assertion.\n\nTHE PASSAGE MUST REALIZE THIS COMPLETE MOVEMENT PLAN — every movement, every assigned quote marker, every evidence commitment (a repair that drops any of these is a failed repair):\n{d1}\n\nNAMED DEFECTS:\n{defs}\n\nVOICE EXEMPLARS (match texture; never quote or closely paraphrase; no shared 8-word sequence):\n{exemplars}\n\nPASSAGE:\n{passage}\n\nOutput ONLY the revised passage body, quotations only as \u{ab}Qnn\u{bb}/\u{ab}Qnn+\u{bb} markers."
    )
}

// ── gates ─────────────────────────────────────────────────────────────────────

/// G-1: ledger closed — every quote & evidence ID appears in the plan, and 3 movements.
pub fn g1_pass(pack: &Pack, d1: &str) -> (bool, Vec<String>) {
    let mut missing = Vec::new();
    for q in &pack.p4a_quote_index {
        if !d1.contains(&q.id) {
            missing.push(q.id.clone());
        }
    }
    for e in &pack.p5_evidence {
        if !d1.contains(&e.id) {
            missing.push(e.id.clone());
        }
    }
    let has_m3 = d1.to_uppercase().replace("**", "").contains("MOVEMENT 3");
    (missing.is_empty() && has_m3, missing)
}

/// Full gauntlet for one draft: substitute → mechanical → G-A → G-E → G-G.
pub struct FullGauntlet {
    pub sub_ok: bool,
    pub substituted: String,
    pub mech: GauntletReport,
    pub ga: crate::GaReport,
    pub ge: JudgeReport,
    pub gg: JudgeReport,
}
impl FullGauntlet {
    pub fn all_pass(&self) -> bool {
        self.sub_ok && self.mech.pass && self.ga.pass && self.ge.pass && self.gg.pass
    }
    /// Aggregate named defects in m6_run order: mech → G-A (hard+label) → G-E → G-G.
    pub fn named_defects(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.mech.pass {
            out.extend(self.mech.defects.iter().cloned());
        }
        if !self.ga.pass {
            out.extend(self.ga.hard_failures.iter().cloned());
            out.extend(self.ga.label_failures.iter().cloned());
        }
        if !self.ge.pass {
            out.extend(self.ge.defects.iter().cloned());
        }
        if !self.gg.pass {
            out.extend(self.gg.defects.iter().cloned());
        }
        out
    }
}

#[allow(clippy::too_many_arguments)]
pub fn full_gauntlet(
    call: &ModelCall,
    model: &str,
    presub: &str,
    pack: &Pack,
    bank: &QuoteBank,
    gate_config: &GateConfig,
    d1_plan: &str,
) -> Result<FullGauntlet, FableError> {
    let sub = substitute_quote_ids(presub, bank);
    let sub_ok = sub.unknown.is_empty();
    let substituted = sub.output;
    let mech = run_gauntlet(&substituted, pack, bank, Some(presub));
    let metrics = measure_text(&strip_markup(&substituted));
    let ga = ga_compare(&metrics, gate_config, false); // section scale
    let ge = judge::judge_with(call, model, Gate::GE, &substituted, pack, None)?;
    let gg = judge::judge_with(call, model, Gate::GG, &substituted, pack, Some(d1_plan))?;
    Ok(FullGauntlet {
        sub_ok,
        substituted,
        mech,
        ga,
        ge,
        gg,
    })
}

// ── the run ─────────────────────────────────────────────────────────────────

fn write(work: &Path, name: &str, content: &str) -> Result<(), RunError> {
    std::fs::write(work.join(name), content).map_err(|e| RunError::Io(e.to_string()))
}
fn read(work: &Path, name: &str) -> Result<String, RunError> {
    std::fs::read_to_string(work.join(name)).map_err(|e| RunError::Io(e.to_string()))
}
fn rec(chain: &Path, work: &Path, name: &str, stage: &str, detail: &serde_json::Value) {
    // provenance is best-effort within a run; a record failure does not abort drafting
    let _ = provenance::record(chain, &work.join(name), stage, detail);
}

/// Run the full FCDP pipeline. `call` is the model seam; `model` labels judge/provenance.
pub fn run(
    call: &ModelCall,
    model: &str,
    pack: &Pack,
    bank: &QuoteBank,
    gate_config: &GateConfig,
    work: &Path,
) -> Result<Outcome, RunError> {
    std::fs::create_dir_all(work).map_err(|e| RunError::Io(e.to_string()))?;
    let chain = work.join("provenance.jsonl");
    let head = build_head(pack);

    // RESUME: an interrupted run left a draft-presub.md → reuse the D-stage artifacts from
    // disk instead of re-spending API on D1/D1.5/D2 (the cost-saver from m6_run.py L2).
    let resume = work.join("draft-presub.md").exists();

    // exemplars grouped by movement type; the three M6 movement types
    let ex_by_type = |t: &str| -> Vec<&str> {
        pack.p2b_exemplars
            .iter()
            .filter(|e| e.movement_type == t)
            .map(|e| e.text.as_str())
            .collect()
    };
    let mv_types = [
        "theoretical-exposition",
        "theoretical-exposition",
        "transition-argument",
    ];

    // ═══ D1 ═══
    let d1 = if resume {
        read(work, "d1-plan.md")?
    } else {
        let d1 = call(&d1_prompt(&head), D_MAX_TOKENS)?.text;
        write(work, "d1-plan.md", &d1)?;
        rec(
            &chain,
            work,
            "d1-plan.md",
            "d1-plan",
            &json!({"gates_run": []}),
        );
        d1
    };
    let (g1, missing) = g1_pass(pack, &d1);
    if !g1 {
        return Err(RunError::Gate(format!(
            "G-1 failed — plan incomplete; missing={missing:?}"
        )));
    }
    if !resume {
        rec(
            &chain,
            work,
            "d1-plan.md",
            "g1-gate",
            &json!({"pass": true, "gates_run": ["G-1"]}),
        );
    }

    // ═══ D1.5 ═══
    let d15 = if resume {
        read(work, "d15-skeleton.md")?
    } else {
        let d15 = call(&d15_prompt(&head, &d1), D_MAX_TOKENS)?.text;
        write(work, "d15-skeleton.md", &d15)?;
        rec(
            &chain,
            work,
            "d15-skeleton.md",
            "d15-skeleton",
            &json!({"surface_to_user": d15.contains("SURFACE-TO-USER"), "gates_run": ["G-1.5"]}),
        );
        d15
    };
    let surface = d15.contains("SURFACE-TO-USER");

    // ═══ D2 (movement by movement) ═══
    let mut presub = if resume {
        read(work, "draft-presub.md")?
    } else {
        let movements = split_movements(&d1);
        let mut parts: Vec<String> = Vec::new();
        for (i, seg) in movements.iter().enumerate() {
            let exs = ex_by_type(mv_types[i])
                .into_iter()
                .take(2)
                .collect::<Vec<_>>()
                .join("\n\n");
            let prior = parts.join("\n\n");
            let p = call(
                &d2_prompt(&head, &d1, seg, &d15, &exs, &prior, i),
                D_MAX_TOKENS,
            )?
            .text;
            let body = p.trim().to_string();
            write(work, &format!("d2-m{}.md", i + 1), &body)?;
            rec(
                &chain,
                work,
                &format!("d2-m{}.md", i + 1),
                &format!("d2-m{}", i + 1),
                &json!({"movement": i + 1, "gates_run": []}),
            );
            parts.push(body);
        }
        let presub = parts.join("\n\n");
        write(work, "draft-presub.md", &presub)?;
        rec(
            &chain,
            work,
            "draft-presub.md",
            "d2-assembled",
            &json!({"words": presub.split_whitespace().count(), "gates_run": []}),
        );
        presub
    };

    // ═══ gauntlet cycle 0 + R-loop ═══
    let repair_exs = {
        // one exemplar per type, in pack order
        let mut seen = std::collections::BTreeSet::new();
        let mut v = Vec::new();
        for e in &pack.p2b_exemplars {
            if seen.insert(e.movement_type.clone()) {
                v.push(e.text.clone());
            }
        }
        v.join("\n\n")
    };

    let mut res = full_gauntlet(call, model, &presub, pack, bank, gate_config, &d1)?;
    write(work, "draft.md", &res.substituted)?;
    rec(
        &chain,
        work,
        "draft.md",
        "gauntlet-0",
        &json!({"sub": res.sub_ok, "mech": res.mech.pass, "ga": res.ga.pass,
                "ge": res.ge.pass, "gg": res.gg.pass,
                "gates_run": ["G-A","G-B","G-C","G-D","G-E","G-F","G-G"]}),
    );

    let mut cycle = 0;
    while !res.all_pass() && cycle < MAX_CYCLES {
        cycle += 1;
        let defects = res.named_defects();
        let revised = call(
            &repair_prompt(&d1, &defects, &repair_exs, &presub),
            REPAIR_MAX_TOKENS,
        )?
        .text;
        presub = revised;
        let tag = format!("-r{cycle}");
        write(work, &format!("draft-presub{tag}.md"), &presub)?;
        rec(
            &chain,
            work,
            &format!("draft-presub{tag}.md"),
            "revision",
            &json!({"cycle": cycle, "defects_addressed": defects.len(), "gates_run": []}),
        );
        res = full_gauntlet(call, model, &presub, pack, bank, gate_config, &d1)?;
        write(work, &format!("draft{tag}.md"), &res.substituted)?;
        rec(
            &chain,
            work,
            &format!("draft{tag}.md"),
            &format!("gauntlet-r{cycle}"),
            &json!({"sub": res.sub_ok, "mech": res.mech.pass, "ga": res.ga.pass,
                    "ge": res.ge.pass, "gg": res.gg.pass,
                    "gates_run": ["G-A","G-B","G-C","G-D","G-E","G-F","G-G"]}),
        );
    }

    // ═══ outcome ═══
    let all_pass = res.all_pass();
    let status = if all_pass {
        "ALL GATES GREEN".to_string()
    } else {
        format!("STOP-AND-SURFACE after {cycle} cycle(s)")
    };
    let surfaced_defects = if all_pass {
        Vec::new()
    } else {
        res.named_defects()
    };
    let final_draft = res.substituted;

    let chain_verified = provenance::verify(&chain).unwrap_or(false);
    let records = provenance::read_chain(&chain).unwrap_or_default();
    let declaration = provenance::declare(&records, pack);
    write(work, "declaration.md", &declaration)?;

    Ok(Outcome {
        status,
        cycles: cycle,
        final_draft,
        surfaced_defects,
        surface_to_user_in_skeleton: surface,
        chain_verified,
        declaration,
    })
}
