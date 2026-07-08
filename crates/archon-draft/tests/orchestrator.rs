//! P5 orchestrator control-flow tests. The model call is injected (replay seam), so the
//! full pipeline — stage sequence, G-1 gate, gauntlet composition, R-loop, stop-rule, and
//! provenance chain — is exercised deterministically with zero API. Recorded M6 drafting
//! outputs drive D1/D1.5/D2; judge outputs are controlled to steer the gate outcome.
//! (Exact live reproduction of the M6 verdicts is the V3 event, not a unit test.)

use archon_draft::fable::{FableError, FableResponse};
use archon_draft::orchestrator::{self, RunError};
use archon_draft::{GateConfig, Pack, QuoteBank};
use serde_json::json;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn m6() -> PathBuf {
    manifest().join("tests/fixtures/m6")
}

/// m6 fixtures are gitignored (unpublished dissertation content); these tests skip when absent.
macro_rules! require_m6 {
    () => {
        if !m6().join("pack.json").exists() {
            eprintln!("skip: m6 fixtures absent (gitignored dissertation content)");
            return;
        }
    };
}

fn load() -> (Pack, QuoteBank, GateConfig) {
    let pack: Pack =
        serde_json::from_str(&std::fs::read_to_string(m6().join("pack.json")).unwrap()).unwrap();
    let bank: QuoteBank =
        serde_json::from_str(&std::fs::read_to_string(m6().join(&pack.p4b_bank_path)).unwrap())
            .unwrap();
    let cfg: GateConfig = serde_json::from_str(
        &std::fs::read_to_string(manifest().join("data/ga-gate-locked-v2.json")).unwrap(),
    )
    .unwrap();
    (pack, bank, cfg)
}

fn read(name: &str) -> String {
    std::fs::read_to_string(m6().join(name)).unwrap()
}

fn reply(text: &str) -> Result<FableResponse, FableError> {
    Ok(FableResponse {
        text: text.to_string(),
        usage: json!({}),
        stop_reason: Some("end_turn".to_string()),
    })
}

/// Classify a prompt into a stage tag (for dispatch + call-order assertions).
fn tag(prompt: &str) -> &'static str {
    if prompt.contains("produce a SKELETON only") {
        "D1.5"
    } else if prompt.contains("produce a MOVEMENT PLAN only") {
        "D1"
    } else if prompt.contains("write ONLY movement 1 as finished prose") {
        "D2-1"
    } else if prompt.contains("write ONLY movement 2 as finished prose") {
        "D2-2"
    } else if prompt.contains("write ONLY movement 3 as finished prose") {
        "D2-3"
    } else if prompt.contains("Revise the passage below to fix ONLY the named defects") {
        "REPAIR"
    } else {
        "JUDGE"
    }
}

fn tmp_work(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fcdp-orch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn stop_rule_fires_after_three_cycles() {
    require_m6!();
    let (pack, bank, cfg) = load();
    let (d1, d15) = (read("d1-plan.md"), read("d15-skeleton.md"));
    let (m1, m2, m3) = (read("d2-m1.md"), read("d2-m2.md"), read("d2-m3.md"));
    let repair_body = read("draft-presub.md"); // valid markers → substitutes cleanly
    let log = RefCell::new(Vec::<&'static str>::new());

    // Judges always return no clean verdict → every gate item fail-closes → G-E/G-G fail
    // every cycle → the R-loop runs to the cap and stops.
    let call = |prompt: &str, _mt: u32| -> Result<FableResponse, FableError> {
        let t = tag(prompt);
        log.borrow_mut().push(t);
        match t {
            "D1" => reply(&d1),
            "D1.5" => reply(&d15),
            "D2-1" => reply(&m1),
            "D2-2" => reply(&m2),
            "D2-3" => reply(&m3),
            "REPAIR" => reply(&repair_body),
            _ => reply("This judge output contains no parseable ruling."),
        }
    };

    let work = tmp_work("stop");
    let outcome = orchestrator::run(&call, "claude-fable-5", &pack, &bank, &cfg, &work).unwrap();

    assert_eq!(outcome.cycles, 3, "R-loop should hit the cap");
    assert_eq!(outcome.status, "STOP-AND-SURFACE after 3 cycle(s)");
    assert!(outcome.chain_verified, "provenance chain must verify");
    assert!(!outcome.surfaced_defects.is_empty());

    // stage sequence: D1 → D1.5 → D2×3 → (GE,GG) → then 3×(REPAIR → GE,GG)
    let l = log.borrow();
    assert_eq!(
        &l[0..7],
        &["D1", "D1.5", "D2-1", "D2-2", "D2-3", "JUDGE", "JUDGE"]
    );
    assert_eq!(l.iter().filter(|t| **t == "REPAIR").count(), 3);
    assert_eq!(l.iter().filter(|t| **t == "JUDGE").count(), 8); // (1 + 3 cycles) × 2 gates

    // artifacts written
    for f in [
        "d1-plan.md",
        "d15-skeleton.md",
        "d2-m1.md",
        "draft.md",
        "draft-presub-r3.md",
        "declaration.md",
    ] {
        assert!(work.join(f).exists(), "missing artifact {f}");
    }
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn g1_gate_rejects_incomplete_plan() {
    require_m6!();
    let (pack, bank, cfg) = load();
    // A plan naming 3 movements but no quote/evidence IDs → G-1 must fail.
    let call = |prompt: &str, _mt: u32| -> Result<FableResponse, FableError> {
        if prompt.contains("produce a MOVEMENT PLAN only") {
            reply("MOVEMENT 1: x\nMOVEMENT 2: y\nMOVEMENT 3: z\n(no ledger)")
        } else {
            reply("unused")
        }
    };
    let work = tmp_work("g1");
    let err = orchestrator::run(&call, "claude-fable-5", &pack, &bank, &cfg, &work).unwrap_err();
    assert!(
        matches!(err, RunError::Gate(_)),
        "expected G-1 gate failure, got {err:?}"
    );
    let _ = std::fs::remove_dir_all(&work);
}

/// Judge output crafted to PASS every item for the given section's shuffled batteries.
/// GE order [E3,E4,E1,E2,E5] bad_on [YES,NO,NO,YES,NO] → pass verdicts [NO,YES,YES,NO,YES].
/// GG order [G3,G4,G2,G1] bad_on [NO,YES,YES,NO]       → pass verdicts [YES,NO,NO,YES].
fn passing_judge(prompt: &str) -> String {
    // GE has 5 numbered items, GG has 4; disambiguate by item count marker.
    if prompt.contains("5. [") {
        "1. r VERDICT: NO\n2. r VERDICT: YES\n3. r VERDICT: YES\n4. r VERDICT: NO\n5. r VERDICT: YES".to_string()
    } else {
        "1. r VERDICT: YES\n2. r VERDICT: NO\n3. r VERDICT: NO\n4. r VERDICT: YES".to_string()
    }
}

#[test]
fn resume_skips_drafting_calls() {
    require_m6!();
    // Pre-populate a work dir with the D-stage artifacts (as an interrupted run would),
    // then run with a call closure that PANICS on any D-stage prompt. If RESUME works, the
    // pipeline reuses disk and only issues judge/repair calls — no wasted drafting API.
    let (pack, bank, cfg) = load();
    let work = tmp_work("resume");
    std::fs::create_dir_all(&work).unwrap();
    for f in [
        "d1-plan.md",
        "d15-skeleton.md",
        "d2-m1.md",
        "d2-m2.md",
        "d2-m3.md",
        "draft-presub.md",
    ] {
        std::fs::copy(m6().join(f), work.join(f)).unwrap();
    }

    let call = |prompt: &str, _mt: u32| -> Result<FableResponse, FableError> {
        match tag(prompt) {
            "D1" | "D1.5" | "D2-1" | "D2-2" | "D2-3" => {
                panic!("RESUME violated: drafting call issued for a D-stage")
            }
            "REPAIR" => reply(&read("draft-presub.md")),
            _ => reply("no parseable ruling"), // judges fail → exercises the gauntlet path
        }
    };
    let outcome = orchestrator::run(&call, "claude-fable-5", &pack, &bank, &cfg, &work).unwrap();
    assert!(outcome.chain_verified);
    // reused the disk drafts, still ran the gauntlet + R-loop
    assert!(work.join("draft.md").exists());
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn judges_pass_when_verdicts_are_clean() {
    require_m6!();
    // Confirms the green-side wiring: with passing judges, G-E and G-G do not contribute
    // defects, so the outcome is governed by the mechanical gates alone.
    let (pack, bank, cfg) = load();
    let (d1, d15) = (read("d1-plan.md"), read("d15-skeleton.md"));
    let (m1, m2, m3) = (read("d2-m1.md"), read("d2-m2.md"), read("d2-m3.md"));
    let repair_body = read("draft-presub.md");

    let call = |prompt: &str, _mt: u32| -> Result<FableResponse, FableError> {
        match tag(prompt) {
            "D1" => reply(&d1),
            "D1.5" => reply(&d15),
            "D2-1" => reply(&m1),
            "D2-2" => reply(&m2),
            "D2-3" => reply(&m3),
            "REPAIR" => reply(&repair_body),
            _ => reply(&passing_judge(prompt)),
        }
    };
    let work = tmp_work("green");
    let outcome = orchestrator::run(&call, "claude-fable-5", &pack, &bank, &cfg, &work).unwrap();
    // Whatever the mechanical outcome, the judges must not be the cause of any surfaced defect.
    assert!(
        !outcome
            .surfaced_defects
            .iter()
            .any(|d| d.starts_with("GE/") || d.starts_with("GG/")),
        "passing judges should surface no G-E/G-G defects: {:?}",
        outcome.surfaced_defects
    );
    assert!(outcome.chain_verified);
    let _ = std::fs::remove_dir_all(&work);
}
