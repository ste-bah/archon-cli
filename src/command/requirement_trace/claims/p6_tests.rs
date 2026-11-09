//! P6 claim domain map: the same candidate goes through both claim gates.
#![allow(non_snake_case)]
use super::tests::{run, task};
use crate::command::requirement_trace::body_claim_findings;
use std::path::Path;

const FILES: &str = "## Files Expected to Change\n\n- `src/lib.rs` — exists (2 lines)\n\n";
fn body(id: &str, sentence: &str) -> String {
    task(
        "TASK-X-010",
        id,
        &format!("{FILES}## Scope\n\n{sentence}\n"),
    )
}
fn early(raw: &str) -> Vec<crate::command::workflow_gate::GateFinding> {
    body_claim_findings(Path::new("."), Path::new("TASK-X-010.md"), raw)
}
#[test]
fn P6_1_named_obligation_with_file_is_testable() {
    let raw = body("REQ-X-001", "REQ-X-001 changes `src/lib.rs`.");
    assert!(early(&raw).is_empty());
    assert!(run(&[("TASK-X-010", &raw)]).findings.is_empty());
}
#[test]
fn P6_2_metadata_only_obligation_is_a_body_finding() {
    let raw = body("REQ-X-001", "Work is described here.");
    let findings = early(&raw);
    assert_eq!(findings.len(), 1);
    assert!(
        findings[0]
            .text
            .contains("never names REQ-X-001 outside its implements list")
    );
    assert_eq!(
        findings[0].remediation_scope,
        archon_workflow::RemediationScope::Body
    );
    assert!(
        run(&[("TASK-X-010", &raw)]).findings[0]
            .text
            .contains("cannot be tested")
    );
}
#[test]
fn P6_3_metadata_only_constraint_is_a_body_finding() {
    let raw = body("QX-001", "Work is described here.");
    assert!(
        early(&raw)[0]
            .text
            .contains("never names QX-001 outside its implements list")
    );
    assert!(
        run(&[("TASK-X-010", &raw)]).findings[0]
            .text
            .contains("cannot be tested")
    );
}
#[test]
fn P6_4_declared_files_ground_a_constraint_sentence() {
    let raw = body("QX-001", "QX-001 restricts this work.");
    assert!(early(&raw).is_empty());
    assert!(run(&[("TASK-X-010", &raw)]).findings.is_empty());
}
#[test]
fn P6_5_named_claim_without_declared_files_is_refuted() {
    let raw = task(
        "TASK-X-010",
        "QX-001",
        "## Scope\n\nQX-001 restricts this work.\n",
    );
    assert!(
        early(&raw)[0]
            .text
            .contains("declares no file it may change")
    );
    assert!(
        run(&[("TASK-X-010", &raw)]).findings[0]
            .text
            .contains("declares no file it may change")
    );
}
#[test]
fn P6_7_body_gate_and_trace_pairs_are_equal_before_and_after_repair() {
    recorded_repository_pairs();
    let repaired = body(
        "REQ-X-001, QX-001",
        "REQ-X-001 and QX-001 restrict this work.",
    );
    let outside = repaired.replace("src/lib.rs", "../outside.rs");
    let refused = repaired.replace("required_tools: []", "required_tools: {bad: value}");
    for (raw, expected) in [
        (body("REQ-X-001, QX-001", "Work is described here."), 2),
        (repaired, 0),
        (outside, 2),
        (refused, 0),
    ] {
        let early_pairs: Vec<_> = early(&raw)
            .iter()
            .flat_map(|f| {
                ["REQ-X-001", "QX-001"]
                    .into_iter()
                    .filter(|id| f.text.contains(&format!("claim of '{id}'")))
                    .map(|id| (f.subject.clone(), id.to_string()))
                    .collect::<Vec<_>>()
            })
            .collect();
        let trace = run(&[("TASK-X-010", &raw)]);
        let trace_pairs: Vec<_> = trace
            .findings
            .iter()
            .filter(|f| f.text.contains("cannot be tested"))
            .flat_map(|f| {
                ["REQ-X-001", "QX-001"]
                    .into_iter()
                    .filter(|id| f.text.contains(&format!("claim of '{id}'")))
                    .map(|id| (f.subject.clone(), id.to_string()))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(early_pairs, trace_pairs);
        assert_eq!(trace_pairs.len(), expected, "{raw}");
        if raw.contains("required_tools: {bad: value}") {
            assert_eq!(trace.findings.len(), 1, "{}", trace.report);
            assert!(trace.findings[0].text.contains("parser-required edit"));
            let dir = tempfile::tempdir().unwrap();
            let lint = crate::command::topology_lint::evaluate_task_file_candidate(
                dir.path(),
                &dir.path().join("TASK-X-010.md"),
                raw.as_bytes(),
                archon_core::config::GateMode::Enforce,
            )
            .unwrap();
            assert_eq!(lint.findings.len(), 1, "{}", lint.report);
            assert_eq!(
                lint.findings[0].deterministic_defect.as_ref().unwrap().code,
                "unparseable_task_file"
            );
        }
    }
}

// Exercise the real recorded-root lookup with cwd inside, rather than at,
// the repository root. Absolute and relative declarations have one meaning.
fn recorded_repository_pairs() {
    use super::tests::{repository, task_set};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let base = repository(&root);
    let absolute = root.join("src/lib.rs").display().to_string();
    for (declared, expected) in [
        (absolute.as_str(), 0),
        ("src/lib.rs", 0),
        ("../outside.rs", 2),
    ] {
        let raw = body(
            "REQ-X-001, QX-001",
            "REQ-X-001 and QX-001 restrict this work.",
        )
        .replace("src/lib.rs", declared);
        let tasks = task_set(dir.path(), &root, &base, &[("TASK-X-010", &raw)]);
        let path = tasks.join("TASK-X-010.md");
        let early = body_claim_findings(&project, &path, &raw);
        let (bindings, findings) =
            crate::command::requirement_trace::load_bindings_with_findings(&tasks).unwrap();
        assert!(findings.is_empty(), "{findings:?}");
        let trace = super::falsify_claims(&tasks, bindings).unwrap();
        let pairs = |rows: Vec<(&str, &str)>| {
            rows.into_iter()
                .flat_map(|(subject, text)| {
                    ["REQ-X-001", "QX-001"]
                        .into_iter()
                        .filter(move |id| text.contains(&format!("claim of '{id}'")))
                        .map(move |id| (subject.to_string(), id.to_string()))
                })
                .collect::<Vec<_>>()
        };
        let early_pairs = pairs(
            early
                .iter()
                .map(|f| (f.subject.as_str(), f.text.as_str()))
                .collect(),
        );
        let trace_pairs = pairs(
            trace
                .findings
                .iter()
                .map(|f| (f.subject.as_str(), f.text.as_str()))
                .collect(),
        );
        assert_eq!(early_pairs, trace_pairs, "{declared}: {}", trace.report);
        assert_eq!(trace_pairs.len(), expected, "{declared}: {}", trace.report);
        if expected > 0 {
            assert!(early.iter().all(|f| {
                f.text
                    .contains("declarations outside the repository cannot ground a claim")
            }));
        }
    }
}

#[test]
fn P6_6_refused_body_still_declares_files_for_other_claims() {
    let healthy = body("REQ-X-001", "REQ-X-001 uses `src/new.rs`.");
    let refused = task("TASK-X-020", "QX-001",
        "## Files Expected to Change\n\n- `src/new.rs` — new\n\n## Scope\n\nQX-001 restricts this work.\n")
        .replace("required_tools: []", "required_tools: {bad: value}");
    let error = archon_workflow::task_universe::parsing::parse_task_file(
        Path::new("TASK-X-020.md"),
        &refused,
    )
    .unwrap_err();
    assert!(error.to_string().contains("field 'required_tools'"));
    let trace = run(&[("TASK-X-010", &healthy), ("TASK-X-020", &refused)]);
    assert_eq!(trace.findings.len(), 1, "{}", trace.report);
    assert_eq!(trace.findings[0].subject, "TASK-X-020");
    assert!(trace.findings[0].text.contains("parser-required edit"));
    assert!(
        trace
            .report
            .contains("tested TASK-X-010 REQ-X-001: 1 exist, 1 derived"),
        "{}",
        trace.report
    );
    let without_declaration = run(&[("TASK-X-010", &healthy)]);
    assert!(
        without_declaration.findings.iter().any(|f| {
            f.subject == "TASK-X-010" && f.text.contains("no task in the set declares it")
        }),
        "{}",
        without_declaration.report
    );
}

#[test]
fn P6_5_outside_only_claim_preserves_forbidden_conflict_refutations() {
    let raw = body("REQ-X-001", "REQ-X-001 restricts this work.")
        + "\n## Files Forbidden to Change\n\n- `src/lib.rs`\n";
    let raw = raw.replace(
        "- `src/lib.rs` — exists (2 lines)",
        "- `src/lib.rs` — exists (2 lines)\n- `../outside.rs` — new",
    );
    let trace = run(&[("TASK-X-010", &raw)]);
    assert!(
        trace.findings.iter().any(|f| f.text.contains("is refuted")
            && f.text.contains("is also named by forbidden bullet")),
        "{}",
        trace.report
    );
    assert!(
        trace.findings.iter().any(|f| f
            .text
            .contains("declarations outside the repository cannot ground a claim")),
        "{}",
        trace.report
    );
}

#[test]
fn host_classification_is_metadata_and_never_supplies_an_authored_claim() {
    let prefix = archon_workflow::requirement_state::CLASSIFICATION_PREFIX;
    for classification in [
        format!(
            "{prefix} done: QX-001; to implement: none; kept, not rebuilt: none; inspection: none."
        ),
        format!(
            "  {prefix} done: none; to implement: QX-001; kept, not rebuilt: none; inspection: none."
        ),
    ] {
        let raw = body(
            "QX-001",
            &format!("{classification}\nWork is described here."),
        );
        assert!(early(&raw).iter().any(|f| {
            f.text
                .contains("never names QX-001 outside its implements list")
        }));
        assert!(!run(&[("TASK-X-010", &raw)]).findings.is_empty());
        let authored = body(
            "QX-001",
            &format!("{classification}\nQX-001 restricts this work."),
        );
        assert!(early(&authored).is_empty());
        assert!(run(&[("TASK-X-010", &authored)]).findings.is_empty());
    }
}
