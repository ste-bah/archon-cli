//! SONA must not reach Phase 6.
//!
//! Requirement satisfaction is binary and evidence-anchored: a requirement is
//! proven by a named artefact or it is not proven. A SONA weight is a
//! continuous number derived from how earlier runs went. If one ever reaches
//! the other, the traceability layer gains a dial that can make a gap look
//! smaller without any new evidence — which is finding F1 (an LLM padded a gap
//! report with reused evidence and claimed 93 requirements mapped) rebuilt on
//! arithmetic that reviewers are more inclined to believe.
//!
//! The boundary is enforced structurally rather than by convention: this is a
//! source scan, so it fails the moment a symbol appears, not the moment someone
//! notices. `archon-knowledge` also has no dependency on `archon-pipeline`, and
//! the manifest check below pins that too — the crate graph is what makes the
//! symbol scan hard to work around.

use std::path::{Path, PathBuf};

/// Symbols that only exist because SONA exists. Any of them inside the
/// traceability layer means a learned value is one call away from a
/// satisfaction verdict.
const SONA_SYMBOLS: &[&str] = &[
    "SonaEngine",
    "SonaParameterTuner",
    "TuningObservation",
    "get_weight",
    "provide_feedback",
    "GeneratedTuningDecision",
    "apply_generated_tuning",
    // The Phase 8 structural knob. It is fenced for the same reason and by the
    // same scan: a learned number that could reach a satisfaction verdict is
    // finding F1 with better arithmetic, and it makes no difference whether the
    // number is a timeout or a fan-out width.
    "ShapeDecision",
    "decide_fanout_width",
    "TunableShapeKnob",
    "learning::sona",
    "archon_pipeline",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(rust_sources(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    files
}

fn assert_sona_free(files: &[PathBuf], area: &str) {
    assert!(
        !files.is_empty(),
        "{area}: found no sources to scan; the guard would pass vacuously"
    );
    for file in files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        for symbol in SONA_SYMBOLS {
            assert!(
                !text.contains(symbol),
                "{area}: {} references '{symbol}'. A learned weight must never be able to \
                 influence whether a requirement is proven — satisfaction is binary and \
                 anchored to evidence, and a tunable number in this layer is finding F1 with \
                 better arithmetic. Move the consumer out of the traceability path.",
                file.display()
            );
        }
    }
}

/// The traceability crate: requirements, coverage, the falsification ladder,
/// and the report that states what is proven.
#[test]
fn the_traceability_crate_never_reads_a_sona_weight() {
    let root = repo_root().join("crates/archon-knowledge/src");
    assert_sona_free(&rust_sources(&root), "archon-knowledge");
}

/// The command surface that drives it.
#[test]
fn the_requirement_trace_command_never_reads_a_sona_weight() {
    let command_dir = repo_root().join("src/command");
    let files: Vec<PathBuf> = rust_sources(&command_dir)
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("requirement_trace"))
                || path
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == "requirement_trace")
        })
        .collect();
    assert_sona_free(&files, "requirement_trace");
}

/// The crate graph is the structural half of the boundary: `archon-knowledge`
/// cannot call SONA because it cannot see `archon-pipeline`. The symbol scan
/// above catches a copy-paste; this catches the dependency edit that would make
/// the copy-paste compile.
#[test]
fn the_traceability_crate_does_not_depend_on_the_learning_pipeline() {
    let manifest = std::fs::read_to_string(repo_root().join("crates/archon-knowledge/Cargo.toml"))
        .expect("archon-knowledge manifest must be readable");
    assert!(
        !manifest.contains("archon-pipeline"),
        "archon-knowledge gained a dependency on archon-pipeline. Phase 6 must not be able to \
         see the learner at all; a weight that can reach a satisfaction verdict turns an \
         evidence-anchored answer into a tunable one."
    );
}

/// The tuner's own blast radius, stated as a test: it may only change how long
/// something runs and how many times a loop retries. If a fifth parameter ever
/// appears here it must be checked against that sentence before this list is
/// updated.
#[test]
fn the_tuner_can_only_move_timeouts_and_retry_counts() {
    let keys: Vec<&str> = archon_core::config::TunableGeneratedParameter::ALL
        .into_iter()
        .map(archon_core::config::TunableGeneratedParameter::key)
        .collect();
    assert_eq!(
        keys,
        [
            "max_repair_iterations",
            "max_investigation_iterations",
            "verification_branch_timeout_secs",
            "host_call_timeout_secs",
        ],
        "the tuner's parameter set changed; every entry must be a retry count or a timeout, \
         never anything that decides whether work is accepted"
    );
}

/// The structural knobs get their own sentence, because theirs is a different
/// one. A budget knob may only change how long something runs; a shape knob may
/// only change how work is distributed, and only in the direction the runtime
/// already clamps. Neither may decide whether work is accepted, and neither may
/// remove a stage.
#[test]
fn the_shape_tuner_can_only_move_how_work_is_distributed() {
    let keys: Vec<&str> = archon_core::config::TunableShapeKnob::ALL
        .into_iter()
        .map(archon_core::config::TunableShapeKnob::key)
        .collect();
    assert_eq!(
        keys,
        ["implementation_wave_fanout_width"],
        "the shape knob set changed; every entry must be a distribution knob whose dangerous \
         direction is already closed by a shipped clamp. A knob that could move a stage, \
         remove a reviewer, or decide acceptance does not belong in this set — see \
         sona_workflow_shape_gate for what a structural knob has to prove before it runs"
    );
}
