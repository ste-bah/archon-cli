//! End-to-end tests, run against the real 17-task corpus wherever possible.
//!
//! No test here indexes anything, and none constructs an embedding provider:
//! anchoring goes through the [`CodeSearch`] port, so a fixture index is enough.
//! That is the constraint made structural — `Search::new` needs an
//! `EmbeddingProvider` and `archon-leann`'s file replacement holds the Cozo
//! write lock across a whole `multi_transaction`, so a test that could index
//! would eventually be a test that does.

use std::path::PathBuf;

use super::*;
use archon_knowledge::traceability::CodeHit;

/// The real decomposition: 17 task files, all 93 PRD requirement IDs.
fn fixture_tasks() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("prd-trading-data-lake-ahdm-001")
}

/// A PRD with the same bullet grammar as the real one.
fn write_prd(dir: &Path, bullets: &[&str]) -> PathBuf {
    let path = dir.join("PRD.md");
    let body: String = bullets.iter().map(|line| format!("{line}\n")).collect();
    std::fs::write(&path, body).expect("write prd");
    path
}

/// An index that answers every query with the same span — the F1 shape.
struct AlwaysSameSpan {
    file_path: String,
}

impl CodeSearch for AlwaysSameSpan {
    fn search(
        &self,
        _query: &str,
        _limit: usize,
        path_pattern: Option<&str>,
    ) -> std::result::Result<Vec<CodeHit>, archon_knowledge::errors::KnowledgeError> {
        if path_pattern.is_some_and(|p| !self.file_path.contains(p)) {
            return Ok(Vec::new());
        }
        Ok(vec![CodeHit {
            file_path: self.file_path.clone(),
            language: "rust".into(),
            line_start: 1,
            line_end: 40,
            relevance_score: 0.99,
        }])
    }
}

/// The real PRD lives outside the repository, so it cannot be a hard test
/// dependency. `ARCHON_TRACE_PRD` overrides the path; absence skips loudly.
fn real_prd() -> Option<PathBuf> {
    let path = std::env::var("ARCHON_TRACE_PRD")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("F:/PRD-TRADING-DATA-LAKE-AHDM-001.md"));
    path.exists().then_some(path)
}

/// Task-side facts, asserted unconditionally because the fixtures ship with the
/// repository. This is the half of D5's claim that does not need the PRD.
#[test]
fn the_seventeen_real_tasks_cite_ninety_three_distinct_requirement_ids() {
    let bindings = load_bindings(&fixture_tasks()).expect("bindings");
    assert_eq!(bindings.len(), 17);

    let citations: usize = bindings.iter().map(|b| b.implements.len()).sum();
    let distinct: std::collections::BTreeSet<&str> = bindings
        .iter()
        .flat_map(|b| b.implements.iter().map(String::as_str))
        .collect();
    assert_eq!(citations, 95);
    assert_eq!(distinct.len(), 93);
    // The two cited twice: REQ-DL-032 (TDL-030, TDL-041) and REQ-DL-033
    // (TDL-040, TDL-041).
    assert_eq!(citations - distinct.len(), 2);
}

#[test]
fn the_real_corpus_has_exact_coverage_in_both_directions() {
    let Some(prd) = real_prd() else {
        // Asserting against a file we do not ship would be a test that lies
        // about what it checked.
        eprintln!("SKIPPED: set ARCHON_TRACE_PRD to the real PRD to run this check");
        return;
    };
    let cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let options = TraceOptions::new(prd, fixture_tasks());
    let report = build_report(&cwd, &options).expect("report");

    assert_eq!(report.coverage.requirements_total, 93);
    assert_eq!(report.coverage.citations_total, 93);
    assert!(
        report.coverage.phantom.is_empty(),
        "phantom citations: {:?}",
        report.coverage.phantom
    );
    assert!(
        report.coverage.unclaimed.is_empty(),
        "unclaimed requirements: {:?}",
        report.coverage.unclaimed
    );
    assert!(report.coverage.is_exact());
    assert_eq!(report.coverage.multiply_claimed.len(), 2);

    // No code index was consulted, so nothing is satisfied — and 93 exact
    // citations do not add up to one proven requirement. That is the whole
    // point: F1's report claimed exactly this coverage and called it mapped.
    assert_eq!(report.satisfied().len(), 0);
    assert_eq!(report.residual_gaps().len(), 93);
}

/// Severity over the real PRD. Recorded because it is a finding, not a target.
#[test]
fn the_real_prd_declares_severity_per_check_so_falsification_scope_is_tiny() {
    let Some(prd) = real_prd() else {
        eprintln!("SKIPPED: set ARCHON_TRACE_PRD to the real PRD to run this check");
        return;
    };
    let text = std::fs::read_to_string(&prd).expect("prd");
    let requirements = requirements::extract_requirements(&text);
    assert_eq!(requirements.len(), 93);

    let in_scope: Vec<&str> = requirements
        .iter()
        .filter(|r| r.is_error_severity())
        .map(|r| r.id.as_str())
        .collect();
    // PRD §21 attaches severity to a validation check, not to a requirement.
    // The derivation is a recorded phrase match, so the scope is small and
    // every member can be checked by hand.
    assert!(
        in_scope.len() < 10,
        "falsification scope should stay auditable; got {in_scope:?}"
    );
    assert!(in_scope.contains(&"REQ-DL-100"), "{in_scope:?}");
    assert!(
        requirements
            .iter()
            .filter(|r| r.is_error_severity())
            .all(|r| r.severity_evidence.is_some()),
        "every error-severity classification must record the phrase that produced it"
    );
}

#[test]
fn without_a_code_index_every_requirement_is_unproven_and_the_report_says_why() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prd = write_prd(dir.path(), &["- REQ-DL-001: one.", "- REQ-DL-002: two."]);
    let options = TraceOptions::new(prd, fixture_tasks());
    let report = build_report(dir.path(), &options).expect("report");

    assert!(!report.index_consulted);
    assert_eq!(report.rows.len(), 2);
    assert!(
        report
            .rows
            .iter()
            .all(|row| row.level == ProofLevel::Unproven)
    );
    assert_eq!(report.satisfied().len(), 0);

    let text = render::report(&report);
    assert!(text.contains("NOT CONSULTED"), "{text}");
    assert!(text.contains("0/2 requirements satisfied"), "{text}");
    // Neither a pass nor a failure.
    assert!(text.contains("declared residual gaps"), "{text}");
}

#[test]
fn every_requirement_the_real_corpus_claims_resolves_to_a_declared_task() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prd = write_prd(
        dir.path(),
        &["- REQ-DL-034: Ingest OpenBB Polygon natively."],
    );
    let report =
        build_report(dir.path(), &TraceOptions::new(prd, fixture_tasks())).expect("report");
    assert_eq!(report.rows[0].claimed_by, ["TASK-TDL-050"]);
}

#[test]
fn a_requirement_the_corpus_does_not_claim_reads_as_a_decomposition_gap() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prd = write_prd(dir.path(), &["- REQ-ZZ-001: nobody claims this."]);
    let report =
        build_report(dir.path(), &TraceOptions::new(prd, fixture_tasks())).expect("report");

    assert_eq!(report.coverage.unclaimed, ["REQ-ZZ-001"]);
    assert_eq!(report.rows[0].level, ProofLevel::Unproven);
    let text = render::report(&report);
    assert!(text.contains("decomposition gap"), "{text}");
    // 93 phantom citations, because the fixture tasks cite IDs this PRD lacks.
    assert_eq!(report.coverage.phantom.len(), 95);
    assert!(text.contains("phantom citation"), "{text}");
}

/// The F1 shape, end to end: an index that returns one generic span for
/// everything produces `Candidate` edges and a named evidence-reuse finding —
/// never a satisfied requirement.
#[test]
fn one_generic_span_answering_for_four_requirements_is_reported_not_accepted() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("crates/archon-trading/src")).expect("mkdir");
    std::fs::write(
        dir.path().join("crates/archon-trading/src/data_lake.rs"),
        "fn generic() {}\n",
    )
    .expect("write");
    let prd = write_prd(
        dir.path(),
        &[
            "- REQ-DL-001: one.",
            "- REQ-DL-002: two.",
            "- REQ-DL-003: three.",
            "- REQ-DL-004: four.",
        ],
    );

    let options = TraceOptions::new(prd, fixture_tasks());
    let mut report = build_report(dir.path(), &options).expect("report");

    // Re-run the anchoring half against the fixture index. `build_report`
    // reaches the code index only through the port, so substituting here
    // exercises the same path the CLI takes.
    let index = AlwaysSameSpan {
        file_path: "crates/archon-trading/src/data_lake.rs".into(),
    };
    let bindings = load_bindings(&fixture_tasks()).expect("bindings");
    let by_task: BTreeMap<&str, &TaskBinding> =
        bindings.iter().map(|b| (b.task_id.as_str(), b)).collect();
    let requirements =
        requirements::extract_requirements(&std::fs::read_to_string(&options.prd).expect("prd"));
    report.rows = requirements
        .iter()
        .map(|requirement| {
            build_row(
                dir.path(),
                requirement,
                &report.coverage,
                &by_task,
                Some(&index as &dyn CodeSearch),
                &[],
                &[],
                &options,
            )
            .expect("row")
        })
        .collect();
    report.shared_anchors = find_shared_anchors(&report.rows);

    assert_eq!(report.rows.len(), 4);
    assert!(
        report
            .rows
            .iter()
            .all(|row| row.level == ProofLevel::Candidate),
        "a generic span must never promote past Candidate"
    );
    assert_eq!(report.satisfied().len(), 0);

    assert_eq!(report.shared_anchors.len(), 1);
    assert_eq!(report.shared_anchors[0].requirement_ids.len(), 4);

    let text = render::report(&report);
    assert!(text.contains("Evidence reuse"), "{text}");
    assert!(text.contains("REQ-DL-001..004"), "{text}");
    assert!(text.contains("0/4 requirements satisfied"), "{text}");
}

/// Every task in the corpus declares at least one runnable verifier command.
///
/// # What this replaced, and why the numbers moved
///
/// This assertion used to pin the opposite: `with_commands * 2 < 17` and more
/// than thirty prose entries. That was the corpus finding — `## Focused Tests`
/// bullets were test *descriptions* ("Registry schema migration test."), not
/// invocations, so no task could reach `Exercised` and the report read
/// 0 of 93 satisfied. The old test said in as many words that a decomposition
/// declaring real commands would move these numbers and that this is where it
/// would be noticed. The decomposition was rewritten; this is that notice.
///
/// # What it does not claim
///
/// A declared command is not a passing one. Reaching `Exercised` still needs a
/// recorded run whose trace touched the anchor, and most of these tests do not
/// exist yet — they are the verifiers the tasks are contracted to produce. What
/// changed is that the ceiling is no longer structural: a prose bullet could
/// never promote no matter what ran.
#[test]
fn every_task_in_the_real_corpus_declares_a_runnable_verifier_command() {
    let bindings = load_bindings(&fixture_tasks()).expect("bindings");
    assert_eq!(bindings.len(), 17);

    let without: Vec<&str> = bindings
        .iter()
        .filter(|b| b.verifier_commands.is_empty())
        .map(|b| b.task_id.as_str())
        .collect();
    assert!(
        without.is_empty(),
        "these tasks declare no runnable command: {without:?}"
    );

    let prose: Vec<&str> = bindings
        .iter()
        .flat_map(|b| b.prose_focused_tests())
        .collect();
    assert!(
        prose.is_empty(),
        "these focused-test bullets are still descriptions, not commands: {prose:?}"
    );
}

/// `--falsify` is opt-in, and opt-in has to mean the read-only output does not
/// move when the feature ships.
///
/// Two halves, because there are two outputs. The JSON must not gain a
/// `falsification_outcome` key — a `null` where there was no field is a
/// breaking change to anything parsing it — and the text must still say NOT
/// EXECUTED for a plan nobody ran. Nothing here spawns a process or writes a
/// byte, which is the claim itself: without the flag, that code is unreachable.
#[test]
fn without_the_flag_the_report_is_what_it_was_before_execution_existed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prd = write_prd(
        dir.path(),
        &["- REQ-DL-100: any `error` check must have status=failed."],
    );
    let options = TraceOptions::new(prd, fixture_tasks());
    assert!(!options.falsify, "execution must be off unless asked for");

    let report = build_report(dir.path(), &options).expect("report");
    assert!(
        report
            .rows
            .iter()
            .flat_map(|row| row.anchors.iter())
            .all(|verdict| verdict.falsification_outcome.is_none())
    );

    let json = serde_json::to_string_pretty(&report).expect("json");
    assert!(!json.contains("falsification_outcome"), "{json}");

    let text = render::report(&report);
    assert!(!text.contains("DECORATION"), "{text}");
    assert!(!text.contains("FALSIFIABLE:"), "{text}");
}

#[test]
fn a_task_directory_that_cannot_be_read_in_full_is_an_error_naming_the_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("TASK-BROKEN.md"),
        "```yaml\ntask_id: TASK-BROKEN\nimplements:\n  - REQ-DL-001\n```\n",
    )
    .expect("write");
    let err = load_bindings(dir.path()).expect_err("refused");
    assert!(err.to_string().contains("TASK-BROKEN.md"), "{err}");
}

#[test]
fn a_missing_code_index_names_the_out_of_band_constraint() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prd = write_prd(dir.path(), &["- REQ-DL-001: one."]);
    let options = TraceOptions {
        leann_db: Some(dir.path().join("absent.db")),
        ..TraceOptions::new(prd, fixture_tasks())
    };
    let err = build_report(dir.path(), &options).expect_err("refused");
    let message = format!("{err:#}");
    assert!(message.contains("no code index"), "{message}");
    assert!(message.contains("out of band"), "{message}");
}
